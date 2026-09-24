//! 路由核心：`Out` 指令集 + `DispatchHandle`（状态登记表 + 卡片状态操作）。
//!
//! impl 按职责拆到子模块（Rust 子模块可访问私有字段，无需放宽可见性）：
//! - 入站飞书事件（文本/按钮 → Out）: [`inbound`]
//! - ACP 事件 → Out: [`acp_events`]
//! - 登记表类型（MsgIdMap/PermCardMap/SessionAllowlist）: [`maps`]
//! - 会话生命周期事件（SessionEvent 广播 + 快照）: [`events`]

mod acp_events;
mod events;
mod inbound;
mod maps;
pub mod provider_card;
pub mod stall;

pub use events::{
    PendingApproval, RemoteSessionView, SessionEvent, SessionInfo, TurnEntry, TurnStreamEvent,
    count_chat_messages,
};
// 失败分类词表（fix-webui-qa-defects 5.1/5.2，design D5）：webui 前端标签
// 映射与 wire 值同源，避免字符串漂移。
pub use events::failure_class;
pub use maps::{
    AutoModeSwitch, AutoModeSwitchMap, MsgIdMap, PermCardEntry, PermCardMap, ReplyTargetMap,
};

use crate::card_events::{
    apply_event_to_card, card_needs_rotation, count_folded_items, update_parent_title,
};
use crate::card_state::CardState;
use crate::cards::CardConfig;
use crate::commands::{Command, RouterAction};
use crate::crud::ProviderForms;
use crate::state::{Mapping, SessionMap};
use sebas_domain::session::{CardPhase, SessionMode, TurnElementType, TurnKind};
use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use sebas_channels::card::{AppUsage, ChannelCard, TurnChrome};
use sebas_channels::key::ChannelKey;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};

/// （workbench-interaction-polish 1.1）`web_cancel_session` 的三态结果。
/// 调用方据此把「空闲」「未知」转成 typed rejection——取消不再对无事可做
/// 的请求伪造成功。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// 在飞 turn 已收到中断指令（驱动层 interrupt，子进程与会话存活）。
    Dispatched,
    /// 会话存在但没有在飞 turn（card FSM 不在 WORKING）——无事可取消。
    Idle,
    /// 未知会话（无映射或 0-turn 占位尚无 session_id）。
    Unknown,
}

#[derive(Debug)]
pub enum Out {
    SpawnAcp {
        key: ChannelKey,
        prompt: String,
        /// Feishu `message_id` of the input message that triggered this spawn
        /// (the user's own message, i.e. the channel text event's reply target). Recorded
        /// so the session's state reactions (👀/🚧/✅/❌) land on that message
        /// instead of the card. `None` for `/new`, WebUI, or replay spawns.
        input_msg_id: Option<String>,
    },
    /// Lazily respawn a restored session (openspec/specs/session-lifecycle/spec.md): try `session/load`
    /// with `session_id`; the dispatcher falls back to a fresh session when
    /// the agent cannot load it.
    SpawnResume {
        key: ChannelKey,
        session_id: String,
        prompt: String,
        /// Feishu `message_id` of the input message that triggered this resume
        /// (the channel text event's reply target), threaded through so the resumed
        /// session's cards reply to that message. `None` for WebUI resumes.
        input_msg_id: Option<String>,
    },
    SendAcp {
        session_id: String,
        cmd: AcpCommand,
    },
    /// Send a neutral presentation instance (the router's accumulated
    /// [`ChannelCard`]). The channel adapter renders it into its native
    /// card JSON and sends it (`feishu`: `send_card`).
    SendCard {
        key: ChannelKey,
        card: ChannelCard,
        msg_id: Option<String>,
        /// When `Some(req_id)`, the dispatcher records the Feishu message_id
        /// of this card keyed by `req_id` so a later button click can flip
        /// the card in place (used for permission cards). When `None`, the
        /// card is fire-and-forget.
        perm_request_id: Option<String>,
        /// Tool call metadata for permission cards: `(tool_name, args)`. Stashed
        /// alongside `perm_request_id` for the click handler (diagnostics /
        /// future granular grants). Ignored for non-permission cards.
        perm_meta: Option<(String, serde_json::Value)>,
        /// Feishu message_id of the root card for this session. When `Some`,
        /// the card is a reply-to threaded card. `None` for fire-and-forget
        /// cards (permission prompts, help, dead-session, expired).
        root_id: Option<String>,
    },
    /// Update a previously-sent card by its Feishu `message_id` (not session_id).
    /// Used for permission-card click feedback: the dispatcher resolved the
    /// responder or hit a stale request, and we want to flip the card in
    /// place rather than let Feishu show a stale prompt the user can keep
    /// clicking. Keyed by message_id so we don't need a per-session map.
    UpdateCardByMsgId {
        key: ChannelKey,
        msg_id: String,
        card: ChannelCard,
    },
    UpdateCard {
        session_id: String,
        card: ChannelCard,
    },
    React {
        session_id: String,
        emoji: String,
    },
    /// Fire-and-forget reaction on a specific Feishu message (not a card).
    /// Used to acknowledge user message receipt immediately with an emoji,
    /// before any processing begins. The reaction is not tracked by the
    /// ReactionTracker — it's a one-shot acknowledgment.
    AckMsg {
        message_id: String,
        emoji: String,
    },
    HelpText {
        key: ChannelKey,
    },
    /// Plain-text reply to the originating chat (e.g. `/settings`, `/help`).
    /// The dispatcher uses FeishuClient::send_text — not a card.
    PlainText {
        key: ChannelKey,
        content: String,
    },
    WatchdogUpgrade {
        key: ChannelKey,
        dev: bool,
        dry_run: bool,
    },
    WatchdogRollback {
        key: ChannelKey,
    },
    WatchdogRestart {
        key: ChannelKey,
    },
    /// `/confirm <token>` — 兑换待确认危险操作的令牌（sebas-29s）。dispatch
    /// 以同一 Feishu actor（同 chat_id）发送 Confirm RPC，watchdog 校验
    /// 同 actor 同参数单次兑换后真正执行原操作。
    WatchdogConfirm {
        key: ChannelKey,
        token: String,
    },
    WatchdogServices {
        key: ChannelKey,
    },
    /// `/system` — watchdog 系统状态（openspec/specs/router-commands/spec.md control commands, Phase 3）。
    WatchdogSystem {
        key: ChannelKey,
    },
    /// `/router on|off|restart|status` — 管理 router 服务（openspec/specs/router-commands/spec.md）。
    WatchdogRouter {
        key: ChannelKey,
        action: RouterAction,
    },
    /// `/webui status` — 查看 webui 服务状态（openspec/specs/router-commands/spec.md）。
    WatchdogWebui {
        key: ChannelKey,
    },
    /// Spawn a session without sending a root card to Feishu (web-originated
    /// sessions). The dispatcher creates the ACP session and wires the pump,
    /// but skips the Feishu send_card / react operations. Card content is
    /// still accumulated in CardStateMap and readable via the WebUI.
    /// `project_dir` specifies the working directory for the agent process
    /// (if None, falls back to the config default). `kind` is the requested
    /// agent kind from the webui backend hint (`acp:<slug>`); None = the
    /// configured default kind. `model`（add-acp-model-selection）是创建时
    /// 请求的模型 id（会话建立后、首 prompt 前应用；None = 默认模型）。
    /// `mode`（add-agent-mode-selection）是创建时请求的权限模式（控制面
    /// 词汇 ask/edit/allow/auto；None = agent 默认行为——本机 claude 不传
    /// `--permission-mode`，远端节点 mode=None）。
    WebSpawn {
        key: ChannelKey,
        prompt: String,
        project_dir: Option<String>,
        kind: Option<String>,
        model: Option<String>,
        mode: Option<String>,
    },
}

/// Result of `DispatchHandle::web_close_session` — callers use this to
/// render a useful error message when the key isn't found (vs. silently
/// no-op'ing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseOutcome {
    /// Session was found and torn down (mapping dropped, child killed).
    /// `discarded_pending` is the number of pending submissions that died
    /// with it (design D5 — a close must name what it threw away).
    Closed { discarded_pending: usize },
    /// No mapping exists for `key` (already closed, or stale URL).
    NotFound,
}

/// 控制面「自动」mode 词汇（permission-mode-auto-gate）。「本会话不再询问」
/// 的 mode 语义固定切到该值。
pub const AUTO_MODE: &str = "auto";
// 控制面缺省 mode 词表（design D5b）随会话域迁往 `sebas_domain::session`
// （add-domain-layer 3.1），原位再导出保持既有路径可解析。
pub use sebas_domain::session::{ask_mode, ASK_MODE};

/// claude 驱动 SetMode 失败时非终态 `Error` 消息的稳定后缀（「…模式未变」，
/// 见 `sebas-acp/src/claude/driver.rs` 的 SetMode 臂）。dispatch 据此把
/// SetMode 失败与同会话其它游离非终态 Error（如 SetModel 被拒=「模型未变」）
/// 区分开——匹配范围已被在飞记录限定到刚点击 auto 的会话，误报面极窄。
/// 【跨 crate 契约】sebas-acp 若改写该文案需同步此处。
pub const MODE_UNCHANGED_MARKER: &str = "模式未变";

/// web 消息路径的类型化拒绝（workbench-turn-queue 5.1，design D5）：目前唯
/// 一的拒绝是 staging 队列溢出——携带上限，提交面映射为可见 4xx。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueFull {
    pub cap: usize,
}

/// 提交来源（design D3）：决定 back-pressure 下的反馈面——Feishu 在在飞卡
/// 上打 ⏳ reaction，web 侧经 SessionInfo.pending 可见即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOrigin {
    Feishu,
    Web,
}

impl TurnOrigin {
    fn is_feishu(self) -> bool {
        matches!(self, TurnOrigin::Feishu)
    }
}

pub struct DispatchHandle {
    /// Public so integration tests in `tests/` can seed mappings without
    /// going through `/new`. Production callers should use `insert_mapping`
    /// (which also persists to disk on daemon side).
    pub map: SessionMap,
    tx: mpsc::Sender<Out>,
    /// Public for tests; production goes through `record_root_msg_id` /
    /// `root_msg_id`.
    pub msgid: MsgIdMap,
    /// Maps a session to the Feishu `message_id` of the input message that
    /// spawned it, so state reactions land on the user's message rather than
    /// the card. Mirrors `msgid` (same `HashMap<String,String>` shape) but a
    /// distinct entry so card PATCH lookup and reaction target never collide.
    input_msg: MsgIdMap,
    card_states: crate::card_state::CardStateMap,
    card_cfg: Arc<RwLock<CardConfig>>,
    /// Tracks the Feishu `message_id` of each outstanding permission card,
    /// keyed by the card's `request_id`. Used to flip the card in place when
    /// the user clicks (or to mark it expired on a stale click). Entries are
    /// removed once resolved so a duplicate click doesn't re-update.
    perm_cards: PermCardMap,
    /// 「本会话不再询问」点击后在飞的自动模式切换（permission-mode-auto-gate）。
    /// 点击记入（keyed by session_id），driver 的 `ModeChanged`（成功）或
    /// SetMode 失败的非终态 `Error` 到达时取走：失败据实翻卡 + 写 turn 流
    /// 事件契约。随事件消费或会话终结清空，无持久化。
    auto_mode_switches: AutoModeSwitchMap,
    /// Provider CRUD 表单实例（`/provider` 命令 + 卡片回调路由）。
    /// 未接线（None）时 `/provider` 落到 HelpText，表单回调仅记日志。
    /// `ProviderForms` 包含 preset + custom 两张表单（共享同一个 overlay 文件），
    /// 列表卡上有两个「＋ 新增」按钮分别走对应表单；编辑/删除按 item.preset
    /// 是否设置路由到对应表单。
    provider_forms: Option<Arc<ProviderForms>>,
    /// SessionManager handle used by WebUI close (kills the child process)
    /// and by tests that drive `web_send_message` flows. `None` for router
    /// instances that never spawn a child (e.g. pure mapping tests).
    mgr: Option<Arc<SessionManager>>,
    /// 原生 sebas-agent 执行体桥（可选，make-feishu-optional-webui-primary）。
    /// `None` = 未接线，所有会话走 acp 桥（现状）；`Some` 时 `agent-*`
    /// 会话经此桥直达原生内核，acp 会话行为不变。
    /// 用 `RwLock` 包装以支持构造后注入（桥需要 router 句柄 → 先建 router、
    /// 再建桥、再 set）。
    native: Arc<RwLock<crate::native_bridge::NativeBridge>>,
    /// The session currently focused by the WebUI. The dashboard uses this
    /// to highlight the active row and to decide which session's detail
    /// page to deep-link into. `None` until the user clicks Switch on a
    /// row (or opens a session detail page).
    active_session: Arc<RwLock<Option<ChannelKey>>>,
    /// 最近入站回复目标（话题内 = 话题根消息 message_id）。话题出站卡
    /// （权限卡等）用它作为 root_id；sebas 出站层（初始卡/失败提示卡）经
    /// [`DispatchHandle::reply_target`] 读取。纯内存、不持久化。
    reply_targets: ReplyTargetMap,
    /// Tracks the Feishu `message_id` of the interactive help card per chat
    /// (keyed by `ChannelKey` serialized to string). When the user clicks a
    /// group tab, the router looks up this msg_id and sends `UpdateCardByMsgId`
    /// to flip the card in place rather than creating a new message.
    help_card_msgid: MsgIdMap,
    /// Session lifecycle broadcast (`SessionEvent`): every mapping mutation
    /// publishes here so detached frontends converge on the router's view.
    /// Bounded; lagging subscribers read `RecvError::Lagged` and must
    /// re-snapshot (events are a notification, never a gap-free log).
    events: broadcast::Sender<SessionEvent>,
    /// ACP 权限事件广播（design D6, OQ1）：`AcpEvent::PermissionRequest` 的
    /// 独立并行通道，与 session 事件广播分开，避免权限噪声冲刷 session 订阅者。
    /// `InProcessBackend` 订阅它把 Claude/ACP 会话的权限请求转成 webui 审查卡。
    /// 只广播 `PermissionRequest`；其余变体不上这条通道。
    perm_events: broadcast::Sender<AcpEvent>,
    /// 实时回合内容广播（workbench-live-conversation-flow 1.1）：
    /// transcript 每批追加 → TurnStreamEvent，core 通道订阅转发。
    turn_events: broadcast::Sender<crate::engine::events::TurnStreamEvent>,
    /// Per-session rendered transcript (`session_id` → ordered entries),
    /// the source for the WebUI/channel turn-content retrieval. Dropped
    /// with the mapping so a recycled session_id cannot inherit stale
    /// content. In-memory only.
    turn_log: Arc<RwLock<HashMap<String, Vec<TurnEntry>>>>,
    /// 回合停滞看门狗的事实登记表（fix-pending-queue-liveness 2.1/2.2，
    /// design D1/D2）：事件时钟 + 泊车豁免 + 阈值。扫描与收尾逻辑在
    /// [`DispatchHandle::force_settle_stalled_turns`]。
    stall: stall::StallRegistry,
    /// （fix-webui-approval-restore-and-session-identity 2.2，design D2）被
    /// 操作者取消（stop / interrupt）的回合打标：cancel 命令发出时记入
    /// session_id，`apply_event` 的 `Finished` 分支消费——为被打标的回合
    /// append 一条「回合被停止」错误类条目，不再让被停回合无声消失。
    /// 纯内存、随事件消费，`Finished` 之后标志即清（正常完成不含该条目）。
    cancelled_turns: Arc<RwLock<std::collections::HashSet<String>>>,
}

impl Clone for DispatchHandle {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            tx: self.tx.clone(),
            msgid: self.msgid.clone(),
            input_msg: self.input_msg.clone(),
            card_states: self.card_states.clone(),
            card_cfg: self.card_cfg.clone(),
            perm_cards: self.perm_cards.clone(),
            auto_mode_switches: self.auto_mode_switches.clone(),
            provider_forms: self.provider_forms.clone(),
            mgr: self.mgr.clone(),
            native: self.native.clone(),
            active_session: self.active_session.clone(),
            reply_targets: self.reply_targets.clone(),
            help_card_msgid: self.help_card_msgid.clone(),
            events: self.events.clone(),
            perm_events: self.perm_events.clone(),
            turn_events: self.turn_events.clone(),
            turn_log: self.turn_log.clone(),
            stall: self.stall.clone(),
            cancelled_turns: self.cancelled_turns.clone(),
        }
    }
}

impl DispatchHandle {
    pub fn new(map: SessionMap) -> (Self, mpsc::Receiver<Out>) {
        Self::new_with_card_config(map, CardConfig::default())
    }

    pub fn new_with_card_config(
        map: SessionMap,
        card_cfg: CardConfig,
    ) -> (Self, mpsc::Receiver<Out>) {
        Self::new_with_config(map, card_cfg, 256)
    }

    pub fn new_with_config(
        map: SessionMap,
        card_cfg: CardConfig,
        channel_buffer: usize,
    ) -> (Self, mpsc::Receiver<Out>) {
        Self::new_with_provider_form(map, card_cfg, channel_buffer, None, None)
    }

    /// 带 SessionManager 的构造（生产/WebUI 用法）。
    pub fn new_with_manager(
        map: SessionMap,
        card_cfg: CardConfig,
        mgr: Arc<SessionManager>,
    ) -> (Self, mpsc::Receiver<Out>) {
        Self::new_with_details(map, card_cfg, 256, None, Some(mgr), None)
    }

    /// 带 provider CRUD 表单的完整构造（root crate 启动时注入）。
    pub fn new_with_provider_form(
        map: SessionMap,
        card_cfg: CardConfig,
        channel_buffer: usize,
        provider_forms: Option<Arc<ProviderForms>>,
        mgr: Option<Arc<SessionManager>>,
    ) -> (Self, mpsc::Receiver<Out>) {
        Self::new_with_details(map, card_cfg, channel_buffer, provider_forms, mgr, None)
    }

    fn new_with_details(
        map: SessionMap,
        card_cfg: CardConfig,
        channel_buffer: usize,
        provider_forms: Option<Arc<ProviderForms>>,
        mgr: Option<Arc<SessionManager>>,
        native: crate::native_bridge::NativeBridge,
    ) -> (Self, mpsc::Receiver<Out>) {
        let (tx, rx) = mpsc::channel(channel_buffer);
        let (events, _) = broadcast::channel(256);
        let (perm_events, _) = broadcast::channel(256);
        let (turn_events, _) = broadcast::channel(256);
        (
            Self {
                map,
                tx,
                msgid: MsgIdMap::default(),
                input_msg: MsgIdMap::default(),
                card_states: crate::card_state::CardStateMap::default(),
                card_cfg: Arc::new(RwLock::new(card_cfg)),
                perm_cards: PermCardMap::default(),
                auto_mode_switches: AutoModeSwitchMap::default(),
                provider_forms,
                mgr,
                native: Arc::new(RwLock::new(native)),
                active_session: Arc::new(RwLock::new(None)),
                turn_log: Arc::new(RwLock::new(HashMap::new())),
                reply_targets: ReplyTargetMap::default(),
                help_card_msgid: MsgIdMap::default(),
                events,
                perm_events,
                turn_events,
                stall: stall::StallRegistry::default(),
                cancelled_turns: Arc::new(RwLock::new(std::collections::HashSet::new())),
            },
            rx,
        )
    }

    /// 构造后注入原生执行体桥（make-feishu-optional-webui-primary）。桥需要
    /// router 句柄 → 先建 router（native = None）、再建桥、再 set；幂等。
    pub async fn set_native_bridge(&self, bridge: crate::native_bridge::NativeBridge) {
        *self.native.write().await = bridge;
    }

    /// Replace the live `CardConfig` at runtime (used by the `/settings`
    /// handler in a later task). Takes the write lock; blocks readers
    /// (cheap — config is small and writes are rare).
    pub async fn set_card_config(&self, new_cfg: CardConfig) {
        let mut g = self.card_cfg.write().await;
        *g = new_cfg;
    }

    /// Snapshot the current `CardConfig` (cloned out of the lock so callers
    /// can hold it without holding the read guard).
    pub async fn card_config(&self) -> CardConfig {
        self.card_cfg.read().await.clone()
    }

    /// Snapshot all session mappings (for WebUI dashboard).
    pub async fn session_snapshot(&self) -> Vec<(ChannelKey, Mapping)> {
        self.map.snapshot_all().await
    }

    /// Whether any mapping (Active, Dormant, or Spawning) exists for `key`.
    /// Unlike `session_alive` (live child only), this accepts Spawning
    /// placeholders — the channel's message/close rejection rule is
    /// "unknown key", not "no live child".
    pub async fn session_exists(&self, key: &ChannelKey) -> bool {
        self.map.get(key).await.is_some()
    }

    /// External snapshot of every known session — the shape the WebUI's
    /// session rows need (mapping + card phase/prompt, `SessionInfo`).
    pub async fn session_info_snapshot(&self) -> Vec<SessionInfo> {
        let keys: Vec<ChannelKey> = self
            .map
            .snapshot_all()
            .await
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let mut out = Vec::with_capacity(keys.len());
        for k in &keys {
            if let Some(info) = self.session_info_for(k).await {
                out.push(info);
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.last_active_unix));
        out
    }

    /// Build the external `SessionInfo` for `key`: mapping state joined with
    /// card-derived phase/prompt. Returns `None` when no mapping exists.
    pub async fn session_info_for(&self, key: &ChannelKey) -> Option<SessionInfo> {
        let m = self.map.get(key).await?;
        // （type-session-vocabularies 2.2）相位来自 `MappingState::phase()` 的
        // 类型化映射（原字符串字面量已收敛）；`session_id` 仍是本机的 live
        // 路由 id——SpawnFailed 的合成 id 语义不同，故不合并成一次匹配。
        let status = m.state.phase();
        let session_id = match &m.state {
            crate::state::MappingState::Active { session_id } => Some(session_id.clone()),
            crate::state::MappingState::Dormant { session_id } => Some(session_id.clone()),
            crate::state::MappingState::Spawning { .. } => None,
            // fail-fast-on-startup-errors：spawn-failed 是对外可见的诚实状态
            // （webui spec delta「会话状态 SHALL 标记为 spawn-failed」）。
            crate::state::MappingState::SpawnFailed { .. } => None,
        };
        // （round4 2.2/2.3）接收回执事实：回合已被服务端接受（prompt 已随
        // 开轮 seed 记入卡态）但 agent 首个输出条目未落（相位仍在 SEED）。
        // 卡态存在是前提——激活语义（resume 不带 prompt）的 SEED 卡 prompt
        // 为空，不算接收回执（没有已接受的提交可停）。
        let (phase, user_prompt, usage, receipt_phase) = match session_id.as_ref() {
            Some(sid) => match self.card_states.snapshot(sid).await {
                Some(st) => {
                    // 空串归 None（fix-webui-approval-restore-and-session-identity
                    // review 4）：种子卡态的默认空 prompt 投成 Some("") 会让
                    // rail 行名闪现空串窗口（`??` 链不跳过空串）。
                    let receipt =
                        st.status_emoji == crate::card_state::phase::SEED
                            && !st.user_prompt.is_empty();
                    let prompt = if st.user_prompt.is_empty() {
                        // （round4 1.2/3.1）卡态无预览时回退命名迁移位——
                        // terminal teardown 退役 / 归档恢复重建的行不丢行名。
                        m.prompt_preview.clone().filter(|p| !p.is_empty())
                    } else {
                        Some(st.user_prompt)
                    };
                    (Some(st.status_emoji), prompt, Some(st.usage), receipt)
                }
                // 无卡态：命名来源只剩迁移位（dormant 退役行 / 恢复行）。
                None => (None, m.prompt_preview.clone().filter(|p| !p.is_empty()), None, false),
            },
            None => (None, None, None, false),
        };
        // rail-declutter-unread D1：可见回复段计数由 transcript 投影（口径见
        // [`count_chat_messages`]）。transcript 只追加、随映射删除（Dormant/
        // Spawning 无可寻址 transcript → 0），单调性由 transcript 保证，无需
        // 第二份计数状态；随 `session.updated` 广播 + rail 10s 轮询兜底。
        let msg_count = match m.transcript_id() {
            Some(tid) => {
                let g = self.turn_log.read().await;
                g.get(tid).map(|log| count_chat_messages(log)).unwrap_or(0)
            }
            None => 0,
        };
        // fix-pending-queue-liveness 2.3（design D3）：「回合占用」的引擎事实
        // ——WORKING 相位 ∨ 接收回执相位（round4 2.2：提交已接受、prompt 已
        // 是最新转录单元、agent 首个输出条目未落）∨ 泊车审批在等 ∨ spawn 窗
        // 口。呈现层据此驱动提交控件的排队/停止形态，不再猜展示词 slug。
        // SpawnFailed/Dormant/终态相位一律不占用。
        let turn_engaged = match &m.state {
            crate::state::MappingState::Spawning { .. } => true,
            crate::state::MappingState::Active { session_id } => {
                phase.as_deref() == Some(crate::card_state::phase::WORKING)
                    || receipt_phase
                    || self.stall.parked_count(session_id).await > 0
            }
            _ => false,
        };
        // （review 3c 补口）本地泊车审批数——`waiting` 投影的数据源；在
        // `turn_engaged` 判定处已读过一次 parked_count，这里复用同一借用点，
        // 避免第二次上锁。u32 与 remote 视图字段对齐。
        let parked_approvals = match &m.state {
            crate::state::MappingState::Active { session_id } => {
                self.stall.parked_count(session_id).await as u32
            }
            _ => 0,
        };
        Some(SessionInfo {
            channel: key.channel_str().to_string(),
            key: key.reference.clone(),
            session_id,
            status,
            // （type-session-vocabularies 2.3）卡相位类型化：卡态里的
            // `status_emoji` 是飞书 emoji_type 词汇，收敛为共享 `CardPhase`。
            phase: phase.as_deref().map(CardPhase::from_wire),
            user_prompt,
            last_active_unix: m.last_active_unix,
            project_dir: m.project_dir.clone(),
            current_model: m.current_model.clone(),
            available_models: m.available_models.clone(),
            agent_kind: m.pending_kind.clone(),
            usage,
            // workbench-turn-queue D6：待生效提交全量随 SessionInfo 下发
            //（快照与每次事件都携带，投递序）。
            pending: self.map.pending_submissions(key).await,
            // （add-agent-mode-selection）desired/effective mode 随快照下发
            // （本机 claude 的 effective 在 spawn argv 应用/ModeChanged 时
            // 落定；远端会话的 mode 走 remote 视图，此处不留）。D5b：desired
            // 非空（缺省 ask），wire 上永远携带确定词。
            desired_mode: m.desired_mode.clone(),
            effective_mode: m.effective_mode.clone(),
            // （session-parallel-liveness-and-unread-polish 1.3）spawn 失败
            // 原因随快照/事件透传（SpawnFailed { reason }），呈现层据此就地
            // 呈现失败原因；非失败会话为 None（不上 wire）。
            spawn_failure_reason: m.spawn_failed_reason().map(str::to_string),
            // 执行体归属由复合后端在快照/事件出口统一打标（D4）；router 自身
            // 只跟踪 ACP 侧映射，留 None 交给上游。
            backend: None,
            // （add-remote-execution-node 8.x）router 只跟踪主控本机会话；远端
            // 投影由 core 侧节点链路投影合并进来，这里不臆造节点维度。
            remote: None,
            // rail-declutter-unread D1：可见回复段数随快照/事件下发。
            msg_count,
            // （session-slash-commands 2.1）agent 广告的命令表随快照下发
            // （AvailableCommands 事件物化；无发现能力 = 空表）。
            available_commands: m.available_commands.clone(),
            // fix-pending-queue-liveness 2.3：回合占用事实随快照下发。
            turn_engaged,
            // （review 3c 补口）本地泊车审批数——`waiting` 投影的数据源；
            // remote 会话的泊车走 remote 视图，此处照填无妨（上游合并时
            // remote 值优先）。
            parked_approvals,
            // （5.1，design D6）操作者 label 随快照下发（None 不上 wire）。
            label: m.label.clone(),
        })
    }

    /// The session's pending submissions (delivery order). Empty for unknown
    /// keys and for backends without a queue (native kernel sessions).
    pub async fn session_pending(&self, key: &ChannelKey) -> Vec<crate::state::PendingSubmission> {
        self.map.pending_submissions(key).await
    }

    /// Remove a pending submission by id（design D7）。返回操作后的全量
    /// pending 视图（成功时），或类型化拒绝。
    pub async fn remove_pending(
        &self,
        key: &ChannelKey,
        pending_id: u64,
    ) -> Result<Vec<crate::state::PendingSubmission>, crate::state::PendingOpError> {
        self.map.remove_pending(key, pending_id).await?;
        Ok(self.map.pending_submissions(key).await)
    }

    /// Reorder a pending submission to `to_index` within its disposition
    /// group（design D7）。返回操作后的全量 pending 视图（成功时）。
    pub async fn move_pending(
        &self,
        key: &ChannelKey,
        pending_id: u64,
        to_index: usize,
    ) -> Result<Vec<crate::state::PendingSubmission>, crate::state::PendingOpError> {
        self.map.move_pending(key, pending_id, to_index).await?;
        Ok(self.map.pending_submissions(key).await)
    }

    /// The session's transcript after `from` (monotonic positions).
    /// `None` when no mapping exists for `key`; a session without a
    /// transcript (Spawning, or no content yet) yields an empty vec.
    /// SpawnFailed 会话经合成 id 返回其错误条目（3.1：inline 错误可读）。
    pub async fn session_turns(&self, key: &ChannelKey, from: u64) -> Option<Vec<TurnEntry>> {
        let m = self.map.get(key).await?;
        let Some(sid) = m.transcript_id() else {
            return Some(Vec::new());
        };
        let sid = sid.to_string();
        let g = self.turn_log.read().await;
        Some(
            g.get(&sid)
                .map(|log| log.iter().filter(|e| e.position >= from).cloned().collect())
                .unwrap_or_default(),
        )
    }

    /// Append one entry to a session's transcript. Position is assigned
    /// monotonically from the log length.
    async fn transcript_push(&self, session_id: &str, mut entry: TurnEntry) {
        {
            let mut g = self.turn_log.write().await;
            let log = g.entry(session_id.to_string()).or_default();
            entry.position = log.len() as u64;
            log.push(entry.clone());
        };
        // 实时回合内容广播（workbench-live-conversation-flow 1.1）：条目
        // 已落账（日志是唯一事实），广播只是增量补充——订阅者落后时
        // Lagged 由消费端按快照收敛，绝不为此阻塞入账路径。无 key（如
        // spawn-failed 合成 id）时静默跳过：没有可寻址的会话面。
        if let Some(key) = self.map.lookup_key_by_session(session_id).await {
            let _ = self
                .turn_events
                .send(crate::engine::events::TurnStreamEvent {
                    channel: key.channel_str().to_string(),
                    key: key.reference.clone(),
                    entries: vec![entry],
                });
        }
    }

    /// Public transcript append for out-of-impl callers (native bridge).
    /// Positions are assigned monotonically from the log length.
    pub async fn push_transcript_entry(&self, session_id: &str, entry: TurnEntry) {
        self.transcript_push(session_id, entry).await;
    }

    /// Drop a session's transcript. Called when the mapping is removed so a
    /// recycled session_id cannot inherit stale content.
    async fn transcript_drop(&self, session_id: &str) {
        self.turn_log.write().await.remove(session_id);
    }

    /// Subscribe to session lifecycle events. Receivers that fall behind get
    /// `RecvError::Lagged` and must re-snapshot via [`Self::session_info_snapshot`].
    pub fn subscribe_session_events(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// Subscribe to live turn-content events（workbench-live-conversation-flow
    /// 1.1）。Lagged 是建议性的：内容可从 transcript 快照恢复，消费端不因
    /// 落后断链。
    pub fn subscribe_turn_events(
        &self,
    ) -> broadcast::Receiver<crate::engine::events::TurnStreamEvent> {
        self.turn_events.subscribe()
    }

    /// 聚焦即拉起（workbench-live-conversation-flow 3.1）：为聚焦的会话
    /// 拉起子进程——0-turn 占位走 fresh spawn、Dormant 走 resume，均**不带
    /// prompt**（不跑首轮；通用 ACP driver 的 session/new 与模型广告在握手
    /// 期完成，无需 prompt）。幂等：Active/Spawning 是无操作。失败经既有
    /// fail_spawn 路径如实进 transcript，占位保留。
    pub async fn web_activate_session(
        &self,
        key: ChannelKey,
    ) -> Result<bool, crate::error::DispatchError> {
        use crate::state::ActivateRoute;
        match self.map.route_activate(&key).await {
            None => Ok(false),
            Some(ActivateRoute::AlreadyLive | ActivateRoute::AlreadyStarting) => Ok(false),
            Some(ActivateRoute::SpawnNew) => {
                // 与首条消息的 SpawnNew 臂同一装配：读回 0-turn 创建时记住的
                // project_dir/kind/model/mode，spawn 出正确的 agent。
                let (project_dir, kind, model, mode) = self
                    .map
                    .get(&key)
                    .await
                    .map(|m| {
                        (
                            m.project_dir.clone(),
                            m.pending_kind.clone(),
                            m.pending_model.clone(),
                            m.pending_mode.clone(),
                        )
                    })
                    .unwrap_or((None, None, None, None));
                self.publish_created(&key).await;
                self.emit(Out::WebSpawn {
                    key,
                    // 空 prompt = 激活语义（拉起但不跑首轮），出站泵据此跳过
                    // 首轮命令。
                    prompt: String::new(),
                    project_dir,
                    kind,
                    model,
                    mode,
                })
                .await;
                Ok(true)
            }
            Some(ActivateRoute::Resume(old_sid)) => {
                self.publish_updated(&key).await;
                self.emit(Out::SpawnResume {
                    key,
                    session_id: old_sid,
                    // 空 prompt = 激活语义：resume 加载历史对话但不跑首轮。
                    prompt: String::new(),
                    input_msg_id: None,
                })
                .await;
                Ok(true)
            }
        }
    }

    /// 重启 spawning 收敛（close-acceptance-blind-spots 盲区 4，design D2
    /// 重投优先）：core 启动恢复状态后对恢复出的 spawning 相位会话一次性落定。
    ///
    /// 恢复（`SessionMap::restore_rows`）能产生的 spawning 只有一种形态——
    /// 0-turn 占位（行上 `session_id=""` + `awaiting_first_prompt=true`，重启后
    /// 仍是等待首条消息的空会话）；真实 spawn-in-flight 不入盘，本就不会出现在
    /// 恢复面。不落定时它就是事故里的那种僵尸：相位永远停在 spawning，无人
    /// 收敛。落定 = 按既定 spawn 流程**重投指令**（经 [`Self::web_activate_session`]，
    /// 消费占位标记、以创建时记住的 project_dir/kind/model/mode 走空 prompt
    /// fresh spawn），成功则照常激活；重投后的成败由出站泵的既有分支收敛——
    /// spawn 成功 → Active，失败（agent 配置缺失等）→ [`Self::fail_spawn`] 追加
    /// 合成错误条目并转 spawn-failed 非 spawning 终态。「一律落失败」被否
    /// （D2）：会把可恢复会话误杀。
    ///
    /// 逐会话独立落定且只投递指令（真正的 spawn 握手在出站泵的独立任务里）：
    /// 单个会话的重投不阻塞、不误伤其它会话的恢复——dormant/active 原样保留。
    /// 幂等：已消费标记的在飞占位经 `route_activate` 走 AlreadyStarting，重复
    /// 调用不再二次投递。返回实际重投的会话数。
    pub async fn settle_restored_spawning(&self) -> usize {
        let spawning = self
            .map
            .snapshot_all()
            .await
            .into_iter()
            .filter(|(_, m)| m.awaiting_first_prompt())
            .map(|(k, _)| k)
            .collect::<Vec<_>>();
        let mut redispatched = 0usize;
        for key in spawning {
            match self.web_activate_session(key.clone()).await {
                Ok(true) => {
                    redispatched += 1;
                    tracing::info!(
                        channel = %key.channel_str(),
                        reference = %key.reference,
                        "restore: spawning 相位会话已重投 spawn 指令"
                    );
                }
                Ok(false) => tracing::warn!(
                    channel = %key.channel_str(),
                    reference = %key.reference,
                    "restore: spawning 相位会话无需重投（占位标记缺失，留待常规路径收敛）"
                ),
                // web_activate_session 现无 Err 出口——兜底同既有 spawn 失败
                // 落点（合成错误条目 + spawn-failed 终态），绝不停在 spawning。
                Err(e) => {
                    tracing::warn!(
                        channel = %key.channel_str(),
                        reference = %key.reference,
                        error = %e,
                        "restore: spawning 相位会话重投失败，落 spawn-failed 终态"
                    );
                    self.fail_spawn(&key, &format!("恢复重投失败: {e}"))
                        .await;
                }
            }
        }
        redispatched
    }

    /// Subscribe to the ACP permission broadcast (design D6): every
    /// `AcpEvent::PermissionRequest` the router applies is forwarded here as
    /// an independent side-channel, parallel to the Feishu card path. Lagging
    /// subscribers see `RecvError::Lagged` (advisory — a missed card can be
    /// recovered from the session transcript, never a correctness loss).
    pub fn subscribe_acp_permission_requests(&self) -> broadcast::Receiver<AcpEvent> {
        self.perm_events.subscribe()
    }

    /// 待批请求读模型（fix-webui-approval-restore-and-session-identity 1.1，
    /// design D1）：按会话枚举当前泊车审批（request_id / 工具 / 参数）。
    /// 数据取自引擎的泊车登记（推送通道之外的第二条腿）——客户端刷新/
    /// 重连后从这里重建审批面，不依赖是否收到过原始事件。`None` = 会话
    /// 未知（无映射）；无泊车 = 空表。
    pub async fn pending_permission_requests(
        &self,
        key: &ChannelKey,
    ) -> Option<Vec<crate::engine::events::PendingApproval>> {
        let m = self.map.get(key).await?;
        let sid = match m.session_id() {
            Some(sid) => sid.to_string(),
            // Spawning 占位没有 session_id，也不可能有泊车（泊车发生在回合
            // 内）——如实返回空表而非 None（会话是存在的）。
            None => return Some(Vec::new()),
        };
        Some(
            self.stall
                .parked_requests(&sid)
                .await
                .into_iter()
                .map(|p| crate::engine::events::PendingApproval {
                    request_id: p.request_id,
                    tool_name: p.tool_name,
                    args: p.args,
                })
                .collect(),
        )
    }

    /// 该 request_id 当前泊车在哪个会话（批复路由的 fail-closed 校验源）。
    /// `None` = 未泊车（从未泊车 / 已批复 / 已随 cancel 释放）——此类 id
    /// 不可再批复（1.4 / 2.3）。
    pub async fn permission_parked_session(&self, request_id: &str) -> Option<String> {
        self.stall.parked_owner(request_id).await
    }

    /// 设置/清空会话 label（5.1，design D6）。会话未知 = `Err`（Unknown），
    /// 成功后发布 Updated 让 rail 行即时反映新名。
    pub async fn web_set_session_label(
        &self,
        key: ChannelKey,
        label: Option<String>,
    ) -> Result<(), crate::state::PendingOpError> {
        if self.map.set_label(&key, label).await {
            self.publish_updated(&key).await;
            Ok(())
        } else {
            Err(crate::state::PendingOpError::Unknown)
        }
    }

    /// Publish a native-kernel permission request onto the same ACP permission
    /// broadcast (make-feishu-optional-webui-primary, design D3). The webui
    /// `InProcessBackend` relays `AcpEvent::PermissionRequest` into its
    /// review-card feed already — reusing the shape means feishu-originated
    /// native sessions surface permission cards for free, encoded key lookup
    /// included. `session_id` is the URL-safe encoded `ChannelKey`.
    ///
    /// （fix-webui-approval-restore-and-session-identity 1.1）原生泊车同步登记
    /// 进引擎泊车表（编码 key 反查路由 sid）：审批读模型对原生会话同样成立，
    /// 批复的 fail-closed 校验（未泊车不可批）对两条执行体同一口径。查不到
    /// 映射时只广播、不登记（现状行为不变）。
    pub async fn publish_native_permission(
        &self,
        session_id: String,
        request_id: String,
        tool_name: String,
        args: serde_json::Value,
    ) {
        if let Some(key) = decode_key(&session_id)
            && let Some(m) = self.map.get(&key).await
            && let Some(sid) = m.session_id()
        {
            self.stall
                .note_permission_parked(sid, &request_id, &tool_name, args.clone())
                .await;
        }
        let _ = self.perm_events.send(AcpEvent::PermissionRequest {
            session_id,
            request_id,
            tool_name,
            args,
        });
    }

    /// 按 request_id 解除泊车登记（原生桥批复成功的收尾；acp 路径走 `emit`
    /// 的 PermissionReply 钩子，不经这里）。返回解除所在的会话 id（`None` =
    /// 本就未泊车）。解除即广播 Updated——waiting 投影即时翻转。
    pub async fn resolve_permission_request(&self, request_id: &str) -> Option<String> {
        let sid = self.stall.note_permission_resolved(request_id).await;
        if let Some(ref sid) = sid
            && let Some(key) = self.map.lookup_key_by_session(sid).await
        {
            self.publish_updated(&key).await;
        }
        sid
    }

    /// 原生内核会话登记为已存在（幂等）：事件驱动地刷新一次 Updated，
    /// 让 webui/channel 看到最新状态。
    pub async fn touch_native_session(&self, key: &ChannelKey) {
        if self.session_info_for(key).await.is_some() {
            self.publish_updated(key).await;
        }
    }

    /// 关闭原生会话（映射移除 + 广播 Removed）。桥在终端错误/会话结束时
    /// 调用。`remove_by_key` 是幂等的（无映射则 no-op），这里总是广播
    /// Removed 以收敛订阅者视图。
    pub async fn fail_native_session(&self, key: &ChannelKey) {
        let existed = self.map.get(key).await.is_some();
        self.map.remove_by_key(key).await;
        if existed {
            self.publish_removed(key);
        }
    }

    /// 回填一个原生权限决定。返回 false = 该 request_id 不在桥的待决表
    /// （可能是 acp 会话的权限，或已过期）。供 webui 审查卡先试 native
    /// 再回退 acp。
    pub async fn answer_native_permission(
        &self,
        request_id: &str,
        decision: crate::native_bridge::NativeApprovalDecision,
    ) -> bool {
        let bridge = self.native.read().await.clone();
        match bridge {
            Some(b) => b.answer_permission(request_id, decision),
            None => false,
        }
    }

    fn publish(&self, event: SessionEvent) {
        // No subscribers is the normal quiet case; lagging ones get the
        // Lagged error on their next recv and re-snapshot. Never propagate.
        let _ = self.events.send(event);
    }

    async fn publish_created(&self, key: &ChannelKey) {
        if let Some(session) = self.session_info_for(key).await {
            self.publish(SessionEvent::Created { session });
        }
    }

    async fn publish_updated(&self, key: &ChannelKey) {
        if let Some(session) = self.session_info_for(key).await {
            self.publish(SessionEvent::Updated { session });
        }
    }

    fn publish_removed(&self, key: &ChannelKey) {
        self.publish(SessionEvent::Removed {
            channel: key.channel_str().to_string(),
            key: key.reference.clone(),
        });
    }

    /// Snapshot all card states (for WebUI session detail).
    pub async fn card_state_snapshot(&self) -> HashMap<String, CardState> {
        self.card_states.snapshot_all().await
    }

    /// Snapshot the MsgIdMap (for message_id lookup).
    pub async fn msgid_snapshot(&self) -> HashMap<String, String> {
        self.msgid.snapshot_all().await
    }

    /// Send an `Out` to the outbound pump. Per openspec/specs/acp-driver/spec.md ("Channel send
    /// fail"): a closed channel is a bug in dev (panic via debug_assert)
    /// and an error-log-and-continue in prod — never a silent drop.
    ///
    /// fix-pending-queue-liveness 2.1（design D2）：`PermissionReply` 出站的
    /// 唯一漏斗在这里——批复离开引擎即解除该请求的泊车豁免（webui 审查卡与
    /// 飞书点击两条路径都经 `emit`），泊车解除后看门狗对回合重新计时。
    pub async fn emit(&self, out: Out) {
        if let Out::SendAcp {
            cmd: AcpCommand::PermissionReply { request_id, .. },
            ..
        } = &out
        {
            // （review 3c 补口，session-unread-badge delta「parked-approval
            // exit emits a frame」）批复离开引擎即解除泊车——解除的会话从
            // waiting 翻回原相位，是生命周期 flip，即刻广播。
            if let Some(session_id) = self.stall.note_permission_resolved(request_id).await
                && let Some(key) = self.map.lookup_key_by_session(&session_id).await
            {
                self.publish_updated(&key).await;
            }
        }
        if let Err(e) = self.tx.send(out).await {
            tracing::error!(?e, "router→outbound channel closed; dropping message");
            debug_assert!(false, "router→outbound channel send failed: {e}");
        }
    }

    // persist-session-map 3.1：`dump_json`（关停快照）随文件持久化一并退休
    // ——映射持久化改为生命周期事件处按变更落库（state.rs 的 persist_upsert）。

    /// Record the root card message_id for a session. Called from the outbound
    /// pump after the first `send_card` returns its message_id.
    pub async fn record_root_msg_id(&self, session_id: String, msg_id: String) {
        self.msgid.record(session_id, msg_id).await;
    }

    /// Record the Feishu `message_id` of the input message that spawned this
    /// session, so state reactions target the user's message. The caller (the
    /// outbound dispatcher, on `create_session`) passes the id that rode in on
    /// `Out::SpawnAcp.input_msg_id`.
    pub async fn record_input_msg_id(&self, session_id: String, msg_id: String) {
        self.input_msg.record(session_id, msg_id).await;
    }

    /// The input message a session's state reactions should land on. `None`
    /// when the session had no Feishu input message (WebUI/`/new`/replay) —
    /// callers fall back to the card's `root_msg_id`.
    pub async fn input_msg_id(&self, session_id: &str) -> Option<String> {
        self.input_msg.get(session_id).await
    }

    /// Record the Feishu message_id of a permission card keyed by its
    /// `request_id`. The dispatcher calls this after `send_card` returns
    /// the actual message_id; a later button click looks it up via
    /// `take_perm_card` to PATCH the card in place.
    pub async fn record_perm_card_msg_id(
        &self,
        request_id: String,
        key: ChannelKey,
        msg_id: String,
        tool_name: String,
        args: Value,
    ) {
        self.perm_cards
            .record(request_id, key, msg_id, tool_name, args)
            .await;
    }

    /// Take (and remove) the permission-card entry for a `request_id`.
    /// Returns the entry (chat, msg_id, tool_name, args) so the caller can
    /// PATCH the card and, on "本会话不再询问", put the chat in allow-all
    /// mode. Returns `None` if no live card (already resolved, or never
    /// existed).
    pub async fn take_perm_card(&self, request_id: &str) -> Option<PermCardEntry> {
        self.perm_cards.take(request_id).await
    }

    /// 「本会话不再询问」点击后在飞的自动模式切换登记/取用（permission-mode-
    /// auto-gate）。生产路径：`on_button`（点击记入）→ `apply_event`
    /// （ModeChanged / SetMode 失败 Error 到达时取走）。公开给测试断言。
    pub fn auto_mode_switches(&self) -> &AutoModeSwitchMap {
        &self.auto_mode_switches
    }

    /// Look up the root card message_id for a session (used by `UpdateCard`).
    pub async fn root_msg_id(&self, session_id: &str) -> Option<String> {
        self.msgid.get(session_id).await
    }

    /// Record the Feishu `message_id` of the interactive help card for a chat.
    /// Called from the outbound dispatcher after `send_card` returns the msg_id.
    /// Keyed by `ChannelKey` serialized to string so the router can later look it
    /// up and PATCH the card in place when the user clicks a group tab.
    pub async fn record_help_card_msgid(&self, key: &ChannelKey, msg_id: String) {
        let key_str = serde_json::to_string(key).expect("ChannelKey serialization");
        self.help_card_msgid.record(key_str, msg_id).await;
    }

    /// Look up the Feishu `message_id` of the help card for a chat.
    /// Returns `None` if no help card was sent yet (or the msg_id was evicted).
    pub async fn help_card_msg_id(&self, key: &ChannelKey) -> Option<String> {
        let key_str = serde_json::to_string(key).expect("ChannelKey serialization");
        self.help_card_msgid.get(&key_str).await
    }

    /// seed_card：SpawnAcp 臂发完 root 卡后调用（dispatch_out）。
    /// 幂等：已存在则保留（防 SpawnAcp 重入冲掉已累积状态）。openspec/specs/feishu-cards/spec.md。
    pub async fn seed_card(&self, session_id: String, user_prompt: String) {
        let seeded = self
            .card_states
            .seed_and_report(session_id.as_str(), &user_prompt)
            .await;
        if seeded {
            // fix-pending-queue-liveness 2.1：回合开轮点之一（spawn 首轮 /
            // emit_turn_card 开轮都经此）——停滞时钟以本轮开轮时刻起算，排队
            // 回合不继承上一回合的等待时长。
            self.stall.touch(&session_id).await;
            // 幂等语义：只有真正新建（而非重入保留）才记录 prompt，防止
            // 重入把同一条 prompt 重复追加进 transcript。
            self.transcript_push(&session_id, TurnEntry::prompt(0, user_prompt.clone()))
                .await;
            if let Some(key) = self.map.lookup_key_by_session(&session_id).await {
                self.publish_updated(&key).await;
            }
        }
    }

    /// apply_event：纯状态变更（FSM emoji + apply_event_to_card append/截断/总量）。
    /// 不发 Out。session 无 CardState 时 lazy seed（prompt="" 兜底）。openspec/specs/feishu-cards/spec.md。
    ///
    /// 返回 `Some(新 emoji)` 表示 FSM 发生转移 —— 由调用方决定是否发
    /// `Out::React`（本方法保持纯状态契约），见 `emit_reaction`。
    pub async fn apply_event(&self, session_id: &str, event: &AcpEvent) -> Option<&'static str> {
        // fix-pending-queue-liveness 2.1：事件时钟单点写入（流式漏斗臂）——
        // 任何事件到达都会重置该会话的停滞计时。
        self.stall.touch(session_id).await;
        let cfg = self.card_cfg.read().await;
        // transcript 条目在 apply 闭包外追加（锁序：card_states → turn_log，
        // 与其他路径不交叉）。TextDelta/Thinking/Tool 事件是内容流，逐条入账。
        // ModelChanged（add-acp-model-selection）：更新映射 current model 并
        // 发布 Updated，让快照立即反映中程切换 —— 覆盖流式 pump（apply_event）
        // 与即时路径（apply_event_to_out）两条到达线。
        if let AcpEvent::ModelChanged { model_id, .. } = event
            && let Some(key) = self.map.lookup_key_by_session(session_id).await
        {
            self.map.set_current_model(&key, model_id.clone()).await;
            self.publish_updated(&key).await;
        }
        // ModeChanged（add-agent-mode-selection）：运行时权限模式切换被
        // agent 接受——更新映射的 effective mode 并发布 Updated，快照立即
        // 反映（与 ModelChanged 同一到达线覆盖）。
        if let AcpEvent::ModeChanged { mode, .. } = event
            && let Some(key) = self.map.lookup_key_by_session(session_id).await
        {
            self.map
                .set_effective_mode(&key, Some(SessionMode::from_wire(&mode)))
                .await;
            self.publish_updated(&key).await;
        }
        // AvailableCommands（session-slash-commands 2.1）：agent 广告的命令
        // 表物化进映射并发布 Updated，快照立即反映。全量覆盖——二次通知
        // （重新广告）天然刷新旧表；pump（apply_event）与即时路径
        // （apply_event_to_out 的 `_` 臂）两条到达线都经过这里。
        if let AcpEvent::AvailableCommands { commands, .. } = event
            && let Some(key) = self.map.lookup_key_by_session(session_id).await
        {
            self.map
                .set_available_commands(&key, commands.clone())
                .await;
            self.publish_updated(&key).await;
        }
        // permission-mode-auto-gate：「本会话不再询问」发起的 auto 切换被
        // 接受 → 取走在飞记录（成功静默——卡片在点击时已翻「已切换自动
        // 模式」，无需二次上报）。
        if let AcpEvent::ModeChanged { mode, .. } = event
            && mode == AUTO_MODE
        {
            let _ = self.auto_mode_switches.take(session_id).await;
        }
        // permission-mode-auto-gate：SetMode 被执行体拒绝（非终态 Error，
        // 带驱动「模式未变」标记）→ 放行不回滚、失败如实上报：写 turn 流
        // 事件契约条目（im 等 detached 前端翻出失败态）+ 就地翻卡（卡归
        // 本进程跟踪时）。
        if let AcpEvent::Error {
            terminal: false,
            message,
            ..
        } = event
            && message.contains(MODE_UNCHANGED_MARKER)
            && let Some(sw) = self.auto_mode_switches.take(session_id).await
        {
            self.report_auto_mode_switch_failed(session_id, &sw, message)
                .await;
        }
        match event {
            AcpEvent::TextDelta { delta, .. } => {
                self.transcript_push(session_id, TurnEntry::markdown(0, delta.clone()))
                    .await;
            }
            // fix-webui-qa-defects 5.1（design D5）：is_error 终态（含 refusal
            // 的非终态 Error + Finished 配对）此前只翻卡片 FSM、不进 transcript
            // ——被拒回合「石沉大海」。错误消息如实合成一条带 generic 分类的
            // error 条目（与 spawn-failure 条目同通道），前端必有可见气泡。
            // SetMode 失败的「模式未变」标记错误已由 permission_mode_result
            // 契约条目上报，不重复合成。
            AcpEvent::Error { message, .. } => {
                if !message.contains(MODE_UNCHANGED_MARKER) {
                    let entry = TurnEntry::error(0, message.clone())
                        .with_failure_class(failure_class::GENERIC);
                    self.transcript_push(session_id, entry).await;
                }
            }
            AcpEvent::ThinkingDelta { delta, .. } => {
                if cfg.thinking != crate::cards::ThinkingDisplay::Hide {
                    self.transcript_push(session_id, TurnEntry::thinking(0, delta.clone()))
                        .await;
                }
            }
            AcpEvent::ToolStart {
                tool_name, args, ..
            } => {
                let args_str = serde_json::to_string_pretty(args).unwrap_or_default();
                // workbench-conversation-view 1.3（design D2）：工具条目打
                // `element_type = "tool"` 标签（内容仍为可读 markdown），让
                // 前端把工具调用与正文区分开、收进可展开组。
                // workbench-agent-identity-and-process-folds 1.2：再附结构化
                // 标题（`{tool} · {key_arg}`，偏好键序提取 + 200 字符上限）
                // 供前端二级折叠收起时显示。
                self.transcript_push(
                    session_id,
                    TurnEntry::tool(0, format!("📖 **{tool_name}**\n```json\n{args_str}\n```"))
                        .with_title(events::tool_entry_title(false, tool_name, Some(args))),
                )
                .await;
            }
            AcpEvent::ToolEnd {
                tool_name, result, ..
            } => {
                // workbench-agent-identity-and-process-folds 1.2：完成态标题
                // 带 `✓ ` 前缀；ToolEnd wire 不携带 args（无 call id 可配对），
                // 传 None → 标题退化为 `✓ {tool}`。
                self.transcript_push(
                    session_id,
                    TurnEntry::tool(0, format!("✓ **{tool_name}**\n{result}"))
                        .with_title(events::tool_entry_title(true, tool_name, None)),
                )
                .await;
            }
            AcpEvent::Finished { session_id } => {
                // （2.2，design D2）被取消的回合在此收尾：cancel 命令发出的
                // 打标在 Finished 到达时消费——append 一条错误类「回合被停止」
                // 条目（复用既有错误条目渲染，不新增 entry kind），transcript
                // 不再让被停回合无声消失。正常完成回合无标、无条目。
                if self.cancelled_turns.write().await.remove(session_id) {
                    let entry = TurnEntry::error(0, "回合被停止（操作者中断了本次回复）")
                        .with_failure_class(failure_class::GENERIC);
                    self.transcript_push(session_id, entry).await;
                }
                // close-acceptance-blind-spots 4.1：零可见输出回合的合成提示
                // 落点（spec「Turn completing without visible output appends
                // a notice」）。取消条目已在上面的分支落账，本检测天然跳过
                // 被停回合（error 条目即可见输出）。
                self.append_zero_output_notice_if_empty(session_id).await;
            }
            _ => {}
        }
        // FSM 转移从闭包里带出来：供返回值（reaction 契约）与 Updated 事件共用。
        let next_cell = std::sync::Mutex::new(None);
        self.card_states
            .apply(session_id, |st| {
                // Handle usage updates separately — they don't affect the FSM
                // or the card body, but update accumulated token counts.
                if let AcpEvent::UsageUpdate { usage, .. } = event {
                    if let Some(model) = &usage.model {
                        st.usage.model = Some(model.clone());
                    }
                    if let Some(input) = usage.input_tokens {
                        st.usage.total_input += input;
                    }
                    if let Some(output) = usage.output_tokens {
                        st.usage.total_output += output;
                    }
                    return None;
                }
                // FSM（openspec/specs/feishu-cards/spec.md）
                let next = next_emoji(&st.status_emoji, event);
                if let Some(e) = next {
                    st.status_emoji = e.into();
                    *next_cell.lock().unwrap() = Some(e);
                }
                // On Finished, reset round counters for the next turn.
                if matches!(event, AcpEvent::Finished { .. }) {
                    st.usage.total_input = 0;
                    st.usage.total_output = 0;
                }
                apply_event_to_card(&mut st.body, event, &cfg);
                next
            })
            .await;
        let next = *next_cell.lock().unwrap();
        // 卡片 emoji 相位转移 = 外部可见的 phase 变化，对外发 Updated。
        // extract-im-service 2.3：UsageUpdate 也发布 —— usage 随 SessionInfo
        // 到达通道订阅端（detached im 的 footer 数据源，design D3）。
        let usage_changed = matches!(event, AcpEvent::UsageUpdate { .. });
        if (next.is_some() || usage_changed)
            && let Some(key) = self.map.lookup_key_by_session(session_id).await
        {
            self.publish_updated(&key).await;
        }
        next
    }

    /// 空回合落点（close-acceptance-blind-spots 4.1，design D3）：真实回合
    /// 正常结束但 transcript 里本回合片段没有任何可见输出条目时，追加一条
    /// 合成 `notice` 条目——回合在时间线上必须可见，绝不可见地消失
    /// （真实事故：`/code-review` 发出后 claude 对未知命令零输出结束回合，
    /// 时间线上无任何痕迹）。
    ///
    /// 判据与守卫：
    /// - **真实回合**才检测：transcript 里存在 prompt 条目（`seed_card` 在
    ///   每个真实回合开轮落下）。无 prompt = 从未开轮（占位/激活幽灵回合），
    ///   沿 fix-webui-qa-defects 3.1 的语义不注入任何合成条目。
    /// - 回合片段 = 最后一条 prompt 之后的部分；片段内有**任一**可见输出
    ///   （正文/thinking/工具/错误，非空内容）即正常回合，不追加。
    /// - 片段内已有 notice（防御：异常的重复 Finished）不再重复追加。
    async fn append_zero_output_notice_if_empty(&self, session_id: &str) {
        let segment: Vec<TurnEntry> = {
            let g = self.turn_log.read().await;
            let Some(log) = g.get(session_id) else {
                return;
            };
            // 最后一条 prompt 的下一位起步；None = 无 prompt 条目（从未开轮）。
            let Some(start) = log
                .iter()
                .rposition(|e| e.kind == TurnKind::Prompt)
                .map(|i| i + 1)
            else {
                return;
            };
            log[start..].to_vec()
        };
        if turn_has_visible_output(&segment) || segment.iter().any(is_zero_output_notice) {
            return;
        }
        self.transcript_push(session_id, TurnEntry::notice(0, ZERO_OUTPUT_NOTICE.to_string()))
            .await;
        tracing::info!(
            session_id = %session_id,
            "turn finished with no visible output; appended a synthetic notice entry (close-acceptance-blind-spots)"
        );
    }

    /// 发射 root 卡 reaction（apply_event 报告 FSM 转移 / continue 回切时由
    /// 调用方触发）。root 卡消息上的 emoji 由此跟踪会话状态：
    /// 🚧 working → ✅ done → ❌ failed。
    pub async fn emit_reaction(&self, session_id: &str, emoji: &str) {
        self.emit(Out::React {
            session_id: session_id.into(),
            emoji: emoji.into(),
        })
        .await;
    }

    /// 「本会话不再询问」的 auto 切换失败上报（permission-mode-auto-gate，
    /// spec「Allow session with failed mode switch is honest」）：放行已在
    /// 点击时发生、不回滚；这里只负责把失败如实送达两张面——
    /// ① turn 流事件契约条目（detached 前端据此翻出失败态，形状见
    ///    [`TurnEntry::permission_mode_result`]）；
    /// ② 就地翻卡（卡归本进程跟踪时，orange 主题失败文案）。
    async fn report_auto_mode_switch_failed(
        &self,
        session_id: &str,
        sw: &maps::AutoModeSwitch,
        cause: &str,
    ) {
        tracing::warn!(
            %session_id,
            request_id = %sw.request_id,
            %cause,
            "allow-session auto mode switch failed; allow stands, reporting honestly"
        );
        let payload = serde_json::json!({
            "request_id": sw.request_id,
            "ok": false,
            "mode": AUTO_MODE,
            "detail": cause,
        });
        self.transcript_push(session_id, TurnEntry::permission_mode_result(0, payload))
            .await;
        if let Some(msg_id) = &sw.msg_id {
            let label = format!(
                "✅ 当前调用已放行；⚠️ 自动模式切换失败：{cause}\n本会话仍会在工具调用时询问（可用 /new 结束会话）。"
            );
            self.emit(Out::UpdateCardByMsgId {
                key: sw.key.clone(),
                msg_id: msg_id.clone(),
                card: crate::cards_ui::resolved_permission_card_titled(&label, "orange"),
            })
            .await;
        }
    }

    /// flush_card：快照 → 累积中立卡（turn chrome + body）→ Out::UpdateCard。
    /// 无 CardState 则 no-op。openspec/specs/feishu-cards/spec.md。节流契约保证 flush 只在 debounce 到点或
    /// Finished/terminal 即时被调，故不维护 dirty flag。
    pub async fn flush_card(&self, session_id: &str) {
        let Some(st) = self.card_states.snapshot(session_id).await else {
            return;
        };
        // 更新父面板标题：添加已折叠项数和经过时间（"🤔 折腾中 · 3项 · 45s"）。
        let elapsed = st.started_at.elapsed();
        let count = count_folded_items(&st.body);
        let mut body = st.body.clone();
        update_parent_title(&mut body, count, &elapsed);
        let usage = AppUsage {
            model: st.usage.model.clone(),
            total_input: st.usage.total_input,
            total_output: st.usage.total_output,
        };
        let card = ChannelCard {
            title: String::new(),
            theme: self.card_cfg.read().await.theme_color.clone(),
            elements: body,
            turn: Some(TurnChrome {
                prompt: st.user_prompt.clone(),
                session_id: session_id.to_string(),
                usage: Some(usage),
            }),
        };
        self.emit(Out::UpdateCard {
            session_id: session_id.to_string(),
            card,
        })
        .await;
    }

    /// 检查当前卡是否接近上限，需要换卡。
    pub async fn card_needs_rotation(&self, session_id: &str) -> bool {
        let Some(st) = self.card_states.snapshot(session_id).await else {
            return false;
        };
        card_needs_rotation(&st.body)
    }

    /// 换卡：冻结当前卡（UpdateCard），发一条新卡（SendCard），重置 body。
    /// 新卡以旧卡的 message_id 作为 root_id，在飞书里呈现为回复关系。
    /// 返回 true 表示成功换卡；false 表示无需换卡或无法换卡。
    pub async fn rotate_card(&self, session_id: &str) -> bool {
        let Some(st) = self.card_states.snapshot(session_id).await else {
            return false;
        };
        if !card_needs_rotation(&st.body) {
            return false;
        }
        let Some(key) = self.map.lookup_key_by_session(session_id).await else {
            return false;
        };
        let theme_color = self.card_cfg.read().await.theme_color.clone();

        // 1. 发射最终 UpdateCard（冻结当前卡，保留全部内容）
        let elapsed = st.started_at.elapsed();
        let count = count_folded_items(&st.body);
        let mut body = st.body.clone();
        update_parent_title(&mut body, count, &elapsed);
        let usage = AppUsage {
            model: st.usage.model.clone(),
            total_input: st.usage.total_input,
            total_output: st.usage.total_output,
        };
        self.emit(Out::UpdateCard {
            session_id: session_id.to_string(),
            card: ChannelCard {
                title: String::new(),
                theme: theme_color.clone(),
                elements: body,
                turn: Some(TurnChrome {
                    prompt: st.user_prompt.clone(),
                    session_id: session_id.to_string(),
                    usage: Some(usage),
                }),
            },
        })
        .await;

        // 2. 构造"接上条"提示，重置 body
        let continuation_note = crate::card_events::continuation_note();
        let fresh_body = vec![continuation_note.clone()];
        self.card_states
            .reset_body(session_id, vec![continuation_note])
            .await;

        // 3. 发射新卡（SendCard），附带旧卡 message_id 作为 root_id 实现回复关系
        let old_msg_id = self.msgid.get(session_id).await;
        let usage2 = AppUsage {
            model: st.usage.model.clone(),
            total_input: st.usage.total_input,
            total_output: st.usage.total_output,
        };
        self.emit(Out::SendCard {
            key,
            card: ChannelCard {
                title: String::new(),
                theme: theme_color,
                elements: fresh_body,
                turn: Some(TurnChrome {
                    prompt: st.user_prompt.clone(),
                    session_id: session_id.to_string(),
                    usage: Some(usage2),
                }),
            },
            msg_id: Some(session_id.to_string()),
            perm_request_id: None,
            perm_meta: None,
            root_id: old_msg_id,
        })
        .await;

        true
    }

    /// drop_card：session 死亡/通道关时清 CardState（防无界增长）。openspec/specs/feishu-cards/spec.md。
    pub async fn drop_card(&self, session_id: &str) {
        self.card_states.drop(session_id).await;
    }

    /// Record a `SessionKey -> session_id` mapping. Called by the dispatcher
    /// once `SessionManager::create_session` has minted the real session_id, so
    /// that continuations, permission-card routing (reverse lookup) and
    /// liveness checks can find the session.
    pub async fn insert_mapping(&self, key: ChannelKey, session_id: String) {
        let existed = self.map.get(&key).await.is_some();
        if let Err(e) = self
            .map
            .insert(key.clone(), crate::state::Mapping::active(session_id))
            .await
        {
            tracing::warn!(?e, "failed to insert session mapping");
            return;
        }
        if existed {
            self.publish_updated(&key).await;
        } else {
            self.publish_created(&key).await;
        }
    }

    /// 最近一次入站消息的回复目标（话题内 = 话题根消息 message_id）。
    /// 话题出站卡（初始 root 卡、spawn/resume 失败提示卡）用它作为
    /// `root_id`，保证回复聚合在原话题。主线 key 返回 `None`（Q7 现状）。
    pub async fn reply_target(&self, key: &ChannelKey) -> Option<String> {
        self.reply_targets.get(key).await
    }

    /// True if a live (Active) session is mapped for `key` (used to reject
    /// button callbacks that arrive after a session has ended, and to keep
    /// `/new` from double-spawning while a spawn is in flight).
    pub async fn session_alive(&self, key: &ChannelKey) -> bool {
        self.map
            .get(key)
            .await
            .map(|m| m.session_id().is_some())
            .unwrap_or(false)
    }

    /// Flip Spawning -> Active for `key` and drain queued prompts.
    /// Called by the dispatcher once `create_session` has minted the id.
    /// `acp_session_id` is the agent's real ACP session id (native-ACP
    /// agents) to persist for later resumes; `None` when the driving session
    /// has no distinct id (e.g. Claude). `model`（add-acp-model-selection）
    /// 是 spawn outcome 的模型选择面，写入映射供快照暴露；`None` = agent
    /// 无模型选项。
    pub async fn activate(
        &self,
        key: &ChannelKey,
        session_id: String,
        acp_session_id: Option<String>,
        model: Option<sebas_acp::AcpModelInfo>,
    ) -> Vec<String> {
        // fix-webui-qa-defects 2.1：resume 回退新路由 id 时（agent 无法 load
        // 旧对话），旧 id 名下的 transcript（归档恢复回放的条目）随激活迁移
        // 到新 id——对话历史不因路由 id 更换而「消失」。成功 load 时路由 id
        // 不变，迁移退化为同 id no-op。
        let previous_transcript_id = self
            .map
            .get(key)
            .await
            .and_then(|m| m.transcript_id().map(str::to_owned));
        let existed = self.map.get(key).await.is_some();
        let pending = self
            .map
            .activate(key, session_id.clone(), acp_session_id, model)
            .await;
        if let Some(old) = previous_transcript_id
            && old != session_id
        {
            self.transcript_migrate(&old, &session_id).await;
        }
        if existed {
            self.publish_updated(key).await;
        } else {
            self.publish_created(key).await;
        }
        pending
    }

    /// 把旧 session_id 名下的 transcript 条目整体迁移到新 id（position 顺序
    /// 保持，在新 id 下从 0 重排）。旧条目随映射换 id 本会被静默孤儿化——
    /// 归档恢复的会话续聊后历史仍在 detail 可见（fix-webui-qa-defects 2.1）。
    async fn transcript_migrate(&self, old: &str, new: &str) {
        let moved = {
            let mut g = self.turn_log.write().await;
            if let Some(mut entries) = g.remove(old) {
                for (i, e) in entries.iter_mut().enumerate() {
                    e.position = i as u64;
                }
                let n = entries.len();
                g.entry(new.to_string()).or_default().extend(entries);
                n
            } else {
                0
            }
        };
        if moved > 0 {
            tracing::info!(%old, %new, moved, "migrated transcript to the new routing id");
        }
    }

    /// 会话模型切换成功（AcpEvent::ModelChanged）后更新映射的 current model，
    /// 并发布 Updated 让快照/订阅者立即反映新模型。
    pub async fn apply_model_changed(&self, session_id: &str, model_id: &str) {
        if let Some(key) = self.map.lookup_key_by_session(session_id).await {
            self.map.set_current_model(&key, model_id.to_string()).await;
            self.publish_updated(&key).await;
        }
    }

    /// （add-agent-mode-selection）spawn 时 argv 应用 / `ModeChanged` 到达后
    /// 更新映射的 effective mode 并发布 Updated，快照/订阅者立即反映。
    /// `None` = 执行体不再声称某 mode 生效（当前不存在该路径，保留对称性）。
    pub async fn apply_mode_changed(&self, session_id: &str, mode: Option<&str>) {
        if let Some(key) = self.map.lookup_key_by_session(session_id).await {
            self.map
                .set_effective_mode(&key, mode.map(SessionMode::from_wire))
                .await;
            self.publish_updated(&key).await;
        }
    }

    /// Spawn failed/timeout（fail-fast-on-startup-errors 3.1/3.2）：占位
    /// 不再拆除、不发 Removed——会话转为 spawn-failed 终态（保持可见），
    /// transcript 立即追加一条带原因的错误事件，并发布 Updated 让前端即时
    /// 呈现。Removed 事件不再是 spawn failure 的首次呈现路径。
    pub async fn fail_spawn(&self, key: &ChannelKey, reason: &str) {
        // Only publish when a placeholder was actually transitioned —
        // fail_spawn is a no-op for Active/Dormant/failed mappings.
        let was_spawning = self
            .map
            .get(key)
            .await
            .map(|m| matches!(m.state, crate::state::MappingState::Spawning { .. }))
            .unwrap_or(false);
        let transcript_id = self.map.fail_spawn(key, reason).await;
        if was_spawning {
            if let Some(tid) = transcript_id {
                // （fix-webui-qa-defects 5.1，design D5）错误条目携带失败分类，
                // 前端气泡标签按类如实渲染（spawn failed），不再一律写死。
                let entry = TurnEntry::error(0, format!("**spawn failed**: {reason}"))
                    .with_failure_class(failure_class::SPAWN);
                self.transcript_push(&tid, entry).await;
            }
            self.publish_updated(key).await;
        }
    }

    /// session-lifecycle spec：rejected resume falls back to fresh AND the user
    /// is informed the old conversation is gone。把提示写进该会话 transcript 并
    /// 发布 Updated（同款 fail_spawn 模式，由 IM/webui 渲染），不只落日志。
    pub async fn notify_resume_fell_back(&self, key: &ChannelKey, session_id: &str) {
        self.transcript_push(
            session_id,
            TurnEntry::error(
                0,
                "旧会话已失效（agent 拒绝恢复），已为你开启全新会话。".to_string(),
            ),
        )
        .await;
        self.publish_updated(key).await;
    }

    /// Start a new session from the WebUI (no Feishu card operations).
    /// Uses `Out::WebSpawn` which the dispatcher handles without sending
    /// cards to Feishu. Returns the SessionKey for the new session.
    /// `project_dir` specifies the working directory for the agent; `kind`
    /// is the requested agent kind (None = configured default). `model`
    /// （add-acp-model-selection）是创建时请求的模型 id（None = 默认模型）。
    pub async fn web_spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
        kind: Option<String>,
        model: Option<String>,
        mode: Option<String>,
    ) -> ChannelKey {
        let key = ChannelKey::web_new();
        // workbench-agent-identity-and-process-folds 收尾：带 prompt 的直接
        // spawn 也把 kind/model/mode 记入映射（此前只记 project_dir，
        // pending_kind 恒 None → SessionInfo.agent_kind 为 null，前端 agent
        // 展示名链路永远走兜底）。`awaiting_first_prompt = false`：prompt 已
        // 随 Out::WebSpawn 直达，映射不是 0-turn 占位——spawn 窗口内的后续
        // 消息照常入队，dump 照常过滤 in-flight（D1：占位身份只属于占位）。
        // web_new 键必新，插入必然发生（Fresh），mode 随插入记为 desired
        // mode，无需再调 set_desired_mode（中途切换仍走该 setter）。
        match self
            .map
            .begin_spawn_with(
                key.clone(),
                kind.clone(),
                model.clone(),
                mode.clone(),
                false,
            )
            .await
        {
            Ok(outcome) => {
                // Record project_dir on the mapping before emitting, so the
                // WebUI can display it even before the session is active.
                self.map.set_project_dir(&key, project_dir.clone()).await;
                // Publish after set_project_dir so the Created snapshot
                // already carries it. AlreadySpawning changed nothing.
                if !matches!(outcome, crate::state::BeginSpawn::AlreadySpawning) {
                    self.publish_created(&key).await;
                }
                self.emit(Out::WebSpawn {
                    key: key.clone(),
                    prompt,
                    project_dir,
                    kind,
                    model,
                    mode,
                })
                .await;
                key
            }
            Err(e) => {
                tracing::warn!(?e, "web_spawn: begin_spawn failed");
                key
            }
        }
    }

    /// Create a 0-turn placeholder session WITHOUT spawning an agent child
    /// (P2 fix: a create-session request with no prompt must not send an
    /// empty `session/prompt` to the agent, which opencode hangs on). The
    /// requested project_dir/kind/model are remembered on the mapping so the
    /// first message spawns the right agent (via `Out::WebSpawn`).
    pub async fn web_create_placeholder(
        &self,
        project_dir: Option<String>,
        kind: Option<String>,
        model: Option<String>,
        mode: Option<String>,
    ) -> ChannelKey {
        let key = ChannelKey::web_new();
        match self
            .map
            .begin_spawn_with(key.clone(), kind.clone(), model.clone(), mode.clone(), true)
            .await
        {
            Ok(outcome) => {
                self.map.set_project_dir(&key, project_dir.clone()).await;
                if !matches!(outcome, crate::state::BeginSpawn::AlreadySpawning) {
                    self.publish_created(&key).await;
                }
                key
            }
            Err(e) => {
                tracing::warn!(?e, "web_create_placeholder: begin_spawn_with failed");
                key
            }
        }
    }

    /// 从归档条目重建会话（fix-webui-qa-defects 2.1，design D1）：以原 key
    /// 重建 `Dormant` 映射（惰性激活——首条消息走既有 resume 语义，agent
    /// 无法 load 时如实回退新会话），并把归档转写条目回放进 turn 存储，使
    /// `GET /api/sessions/{key}`（detail）恢复后立即可见全部 N 条条目。
    ///
    /// 这是「消费归档 ⇄ 重建会话」原子语义的引擎半边：调用方（webui restore
    /// handler）必须先调本方法、成功后才从归档删除条目——重建失败时归档
    /// 原样保留，数据不再有「三处皆空」的丢失形态。
    ///
    /// - `session_id`：归档时刻的原路由 id（`ArchiveEntry.session_id`）。恢复
    ///   后 Dormant 映射以它寻址 transcript，也供 resume 尝试加载原对话；
    ///   `None`（旧归档条目）时以 key reference 合成确定性 id，transcript
    ///   寻址不受影响（resume 会被 agent 拒绝并诚实回退）。
    /// - `identity`（fix-webui-approval-restore-and-session-identity 3.2，
    ///   design D3）：归档条目携带的会话身份（agent_kind / desired_mode /
    ///   current_model / available_models），恢复时原样带回——后续对话用回
    ///   原 agent 与模型面。旧归档条目（全空）维持既有默认（agent 显示回退、
    ///   模型目录清空），不做数据迁移。
    /// - `label` / `prompt_preview`（fix-webui-qa-defects-round4 3.1，design
    ///   M1「迁移而非重推导」）：归档时刻的命名来源（操作者 label 与首条
    ///   prompt 预览）随快照迁回映射——Dormant 重建没有卡态，不迁移则行名
    ///   退化为短 id。`None`/空串 = 旧档无该字段，回退现状（短 id）。
    /// - 拒绝：key 已有映射（活会话/占位与归档同 key 是状态矛盾）→
    ///   [`crate::error::DispatchError`]；容量满 → Capacity。
    pub async fn web_restore_session(
        &self,
        key: ChannelKey,
        session_id: Option<String>,
        project_dir: Option<String>,
        transcript: Vec<TurnEntry>,
        identity: crate::state::SessionIdentity,
        label: Option<String>,
        prompt_preview: Option<String>,
    ) -> Result<(), crate::error::DispatchError> {
        if self.map.get(&key).await.is_some() {
            return Err(crate::error::DispatchError::Conflict(format!(
                "会话映射已存在，拒绝从归档覆盖重建: {}",
                key.reference
            )));
        }
        let sid = session_id.unwrap_or_else(|| key.reference.clone());
        let mut mapping = crate::state::Mapping::dormant(sid.clone(), now_unix());
        mapping.project_dir = project_dir;
        // （3.2）身份带回：四项原样落入映射（`None` 维持现默认）。
        mapping.pending_kind = identity.agent_kind;
        mapping.current_model = identity.current_model;
        mapping.available_models = identity.available_models;
        if let Some(mode) = identity.desired_mode {
            mapping.desired_mode = SessionMode::from_wire(&mode);
        }
        // （round4 3.1）命名来源迁移：label 优先（行名第一顺位），预览兜底
        // （第二顺位）。空串归 None——不占 `??` 链的位。
        mapping.label = label.filter(|l| !l.is_empty());
        mapping.prompt_preview = prompt_preview.filter(|p| !p.is_empty());
        self.map.insert(key.clone(), mapping).await?;
        // 转写回放：条目按归档快照顺序重排 position（0..n 单调），随 Dormant
        // 的 transcript_id（2.1）在 detail / msg_count 投影完整可见。
        if !transcript.is_empty() {
            let mut g = self.turn_log.write().await;
            let log = g.entry(sid).or_default();
            log.reserve(transcript.len());
            for (i, mut e) in transcript.into_iter().enumerate() {
                e.position = i as u64;
                log.push(e);
            }
        }
        // 广播 Created：detached 前端的 rail / 会话列表不等轮询即收敛。
        self.publish_created(&key).await;
        tracing::info!(
            key = %key.reference,
            "restored archived session as a Dormant mapping with its transcript"
        );
        Ok(())
    }

    /// Send a message to an existing session from the WebUI.
    /// Routes the message through the session map (same logic as Feishu
    /// text messages) and emits the appropriate Out instruction. Command
    /// text is parsed like the Feishu path (B 档冒烟 2026-09-04：webui 直达
    /// 路径此前把 `/cancel` 当普通 prompt 发给 opencode，中断无效）。
    /// （extract-im-service 2.2）取消该 key 会话的在飞 turn（`/cancel` 的
    /// 通道面）。workbench-interaction-polish 1.1：返回从 bool 细化为三态
    /// ——在飞 turn 已派发中断（Dispatched）、会话无在飞 turn（Idle）、
    /// 未知会话（Unknown）。空闲/未知由调用方转 typed rejection，不再把
    /// 「无事可取消」伪装成成功。判定与回合队列的 in-flight 定义同源
    /// （card FSM 的 WORKING 态 ∨ 泊车审批在等——`turn_engaged` 同款，
    /// fix-webui-approval-restore-and-session-identity 2.1：泊车中的回合同样
    /// 可停），会话保留、可继续对话。
    ///
    /// （fix-webui-approval-restore-and-session-identity 2.1，design D2）
    /// interrupt 全程收尾：Cancel 派发后同步 fail-closed 释放该会话的**全部**
    /// 泊车审批（幂等，未决请求不再阻塞、`turn_engaged` 随 `parked_count=0`
    /// 回落），并把回合打上「被取消」标——`Finished` 到达时补一条停止条目
    /// （2.2），被停回合不再无声消失。释放只影响 engine 侧登记：driver 内部
    /// 的 hook 等待不受影响（子进程随后被断开），已释放请求的迟到批复走
    /// typed rejection（2.3）。
    pub async fn web_cancel_session(&self, key: &ChannelKey) -> CancelOutcome {
        use crate::card_state::phase::{SEED, WORKING};
        let sid = self
            .map
            .get(key)
            .await
            .and_then(|m| m.session_id().map(str::to_owned));
        let Some(sid) = sid else {
            return CancelOutcome::Unknown;
        };
        let working = matches!(
            self.card_states.status_emoji(&sid).await.as_deref(),
            Some(WORKING)
        );
        // （fix-webui-qa-defects-round4 2.3，design D1 修正项）接收回执阶段
        // 可停：回合已被服务端接受（prompt 已随开轮记入卡态）而 agent 首帧
        // 未落（相位 SEED）——该阶段 interrupt 准入原来只认 WORKING，停止
        // 会被 409（Idle）顶回，操作员在 600s 看门狗兜底前没有自助手段。
        // 空卡 prompt（resume 激活语义的 SEED）不算：没有已接受的提交。
        let receipt = match self.card_states.snapshot(&sid).await {
            Some(st) => st.status_emoji == SEED && !st.user_prompt.is_empty(),
            None => false,
        };
        let parked = self.stall.parked_count(&sid).await > 0;
        if !working && !receipt && !parked {
            return CancelOutcome::Idle;
        }
        self.mark_cancelled_turn(&sid).await;
        self.emit(Out::SendAcp {
            session_id: sid.clone(),
            cmd: AcpCommand::Cancel { session_id: sid.clone() },
        })
        .await;
        self.release_parked_approvals(&sid).await;
        CancelOutcome::Dispatched
    }

    /// （2.1）fail-closed 释放会话的全部泊车审批并广播状态翻转。幂等：无
    /// 泊车时 no-op。返回被释放的 request_id 列表（日志/测试断言用）。
    pub async fn release_parked_approvals(&self, session_id: &str) -> Vec<String> {
        let released = self.stall.release_session(session_id).await;
        if !released.is_empty()
            && let Some(key) = self.map.lookup_key_by_session(session_id).await
        {
            // 泊车集合清空 = (phase, parked) 投影翻转（waiting 消失、
            // turn_engaged 回落）——即刻广播，rail/提交控件不等轮询。
            self.publish_updated(&key).await;
        }
        released
    }

    /// （2.2）回合取消打标：cancel 派发前调用，`apply_event` 的 `Finished`
    /// 分支消费后补「回合被停止」条目。正常完成回合无标、无条目。
    async fn mark_cancelled_turn(&self, session_id: &str) {
        self.cancelled_turns
            .write()
            .await
            .insert(session_id.to_string());
    }

    /// Send a message to an existing session from the WebUI. Returns
    /// `Err(QueueFull)` when the spawn-window staging queue is at its cap —
    /// the caller must reject visibly (workbench-turn-queue 5.1, design D5).
    /// Routes the message through the session map (same logic as Feishu
    /// text messages) and emits the appropriate Out instruction. Command
    /// text is parsed like the Feishu path (B 档冒烟 2026-09-04：webui 直达
    /// 路径此前把 `/cancel` 当普通 prompt 发给 opencode，中断无效）。
    pub async fn web_send_message(
        &self,
        key: ChannelKey,
        message: String,
    ) -> Result<(), QueueFull> {
        match crate::commands::parse_command(&message) {
            // 命令臂：无活跃会话明确回复（与 feishu 路径一致，sebas-ixv）。
            Command::Cost | Command::Cancel | Command::Status => {
                let sid = self
                    .map
                    .get(&key)
                    .await
                    .and_then(|m| m.session_id().map(str::to_owned));
                if let Some(sid) = sid {
                    let cmd = match crate::commands::parse_command(&message) {
                        Command::Cost => AcpCommand::ContinueSession {
                            session_id: sid.clone(),
                            prompt: "/cost".into(),
                        },
                        Command::Status => AcpCommand::ContinueSession {
                            session_id: sid.clone(),
                            prompt: "/status".into(),
                        },
                        Command::Cancel => {
                            // （2.2）`/cancel` 与停止按钮同一收尾语义：打标 +
                            // fail-closed 释放泊车（append 停止条目走 Finished
                            // 分支）。
                            self.mark_cancelled_turn(&sid).await;
                            self.release_parked_approvals(&sid).await;
                            AcpCommand::Cancel {
                                session_id: sid.clone(),
                            }
                        }
                        _ => unreachable!(),
                    };
                    self.emit(Out::SendAcp {
                        session_id: sid,
                        cmd,
                    })
                    .await;
                } else {
                    let cmd = message.split_whitespace().next().unwrap_or("");
                    self.emit(Out::PlainText {
                        key,
                        content: format!(
                            "当前没有活跃会话，{cmd} 需要活跃会话。发送 /new 开始新会话。"
                        ),
                    })
                    .await;
                }
                return Ok(());
            }
            Command::Compact => {
                let sid = self
                    .map
                    .get(&key)
                    .await
                    .and_then(|m| m.session_id().map(str::to_owned));
                if let Some(sid) = sid {
                    // compact 回合与 submit_turn 的 settled 路径同语义：先把
                    // DONE/FAILED 翻成 WORKING 再转发。不翻的后果：compact
                    // 期间在飞检查视会话为空闲，新提交走 settled 臂提前 seed
                    // 下一个 prompt——迟到的前一回合 Finished 落在「最后一条
                    // prompt 之后为空」的窗口里，零输出检测误追加 notice
                    // （close-acceptance-blind-spots 门禁抓到的回合重叠）。
                    // compact 不写 transcript prompt（回复并入尾随气泡），
                    // 因此这里只翻相位、不 emit_turn_card。
                    self.card_states
                        .apply(&sid, |st| {
                            if matches!(
                                st.status_emoji.as_str(),
                                crate::card_state::phase::DONE
                                    | crate::card_state::phase::FAILED
                            ) {
                                st.status_emoji = crate::card_state::phase::WORKING.into();
                                true
                            } else {
                                false
                            }
                        })
                        .await;
                    self.emit(Out::SendAcp {
                        session_id: sid.clone(),
                        cmd: AcpCommand::ContinueSession {
                            session_id: sid,
                            prompt: "/compact".into(),
                        },
                    })
                    .await;
                } else {
                    self.emit(Out::PlainText {
                        key,
                        content: "当前没有活跃会话，/compact 需要活跃会话。发送 /new 开始新会话。"
                            .into(),
                    })
                    .await;
                }
                return Ok(());
            }
            // 其余命令/普通文本：webui 侧不处理的命令保持原行为（不静默
            // 截留），与 feishu 路径的 PassThrough 语义一致。
            _ => {}
        }
        match self.map.route_text(key.clone(), message.clone()).await {
            Ok(crate::state::TextRoute::Continue(sid)) => {
                // 共享提交入口（design D3）：WORKING → 入队（不写 transcript、
                // 不发 SendAcp）；否则开新轮。prompt 一律由 seed_card 在开轮
                // 时写入 transcript（design D4——提交即写会让在跑回合的输出
                // 被插话切开）。
                self.submit_turn(key, &sid, message, false, None, TurnOrigin::Web)
                    .await;
            }
            Ok(crate::state::TextRoute::SpawnNew) => {
                // route_text inserted a Spawning placeholder for `key`. A
                // 0-turn placeholder created with no prompt carries the
                // requested project_dir/kind/model on the mapping — read them
                // back so the first message spawns the right agent (P2 fix).
                let (project_dir, kind, model, mode) = self
                    .map
                    .get(&key)
                    .await
                    .map(|m| {
                        (
                            m.project_dir.clone(),
                            m.pending_kind.clone(),
                            m.pending_model.clone(),
                            m.pending_mode.clone(),
                        )
                    })
                    .unwrap_or((None, None, None, None));
                self.publish_created(&key).await;
                self.emit(Out::WebSpawn {
                    key,
                    prompt: message,
                    project_dir,
                    kind,
                    // 直达消息路径的模型参数来自 0-turn 创建时记住的
                    // pending_model（用户指定模型走创建表单）。
                    model,
                    // （add-agent-mode-selection）0-turn 创建时记住的 mode。
                    mode,
                })
                .await;
            }
            Ok(crate::state::TextRoute::Resume(old_sid)) => {
                // The Dormant mapping was claimed and swapped to Spawning.
                self.publish_updated(&key).await;
                self.emit(Out::SpawnResume {
                    key,
                    session_id: old_sid,
                    prompt: message,
                    // WebUI resume: no Feishu input message to thread to.
                    input_msg_id: None,
                })
                .await;
            }
            Ok(crate::state::TextRoute::Enqueued) => {
                tracing::debug!("web message staged (session spawning); visible as pending");
            }
            // 5.1（design D5）：满队列可见拒绝——提交面映射 4xx。
            Ok(crate::state::TextRoute::Overflow { cap }) => {
                return Err(QueueFull { cap });
            }
            Err(e) => {
                tracing::warn!(?e, "web_send_message: route_text failed");
            }
        }
        Ok(())
    }

    /// Mark `key` as the currently focused WebUI session. The dashboard
    /// highlights the active row and the sidebar shows it under "Active".
    /// Idempotent; safe to call on every page load.
    /// Set (or clear with `None`) the WebUI-focused session. Idempotent.
    pub async fn web_set_active(&self, key: Option<ChannelKey>) {
        let mut g = self.active_session.write().await;
        *g = key;
    }

    /// Snapshot of the currently focused session, if any.
    pub async fn active_session_snapshot(&self) -> Option<ChannelKey> {
        self.active_session.read().await.clone()
    }

    /// Tear down the session mapped to `key` from the WebUI.
    /// - Looks up the mapping; if Active, kills the child process via the
    ///   SessionManager.
    /// - Removes the mapping + drain queue (SessionMap does both).
    /// - Drops card state and root msg_id so recycled ids don't inherit
    ///   stale entries.
    /// - Clears the in-flight auto-mode switch (若有) and the per-key reply
    ///   target (topic root message_id) so recycled keys don't inherit
    ///   stale aggregation targets.
    /// - Clears `active_session` if this key was the focused one.
    pub async fn web_close_session(&self, key: ChannelKey) -> CloseOutcome {
        let Some(mapping) = self.map.get(&key).await else {
            return CloseOutcome::NotFound;
        };
        let session_id_opt = mapping.session_id().map(|s| s.to_string());

        // design D5：在移除映射之前盘点未执行的待生效提交并发出 PendingDropped
        // ——关闭带队列的会话绝不静默丢队，观察者收到逐条标注。
        let pending = self.map.pending_submissions(&key).await;
        let discarded_pending = pending.len();
        if !pending.is_empty() {
            self.publish(SessionEvent::PendingDropped {
                channel: key.channel_str().to_string(),
                key: key.reference.clone(),
                dropped: pending,
            });
        }

        // Active sessions have a live child — kill it before dropping state.
        // Dormant mappings (restored from disk) have no child; Spawning
        // placeholders have a child we never tracked, so we don't kill
        // anything there.
        if let Some(sid) = &session_id_opt {
            if let Some(mgr) = &self.mgr {
                mgr.kill(sid).await;
            }
            self.card_states.drop(sid).await;
            self.msgid.drop(sid).await;
            self.transcript_drop(sid).await;
            // （2.1）会话终结路径同样 fail-closed：泊车审批随终结释放（孤儿
            // 泊车不可越过会话存活）、时钟退役——复用 id 不继承 stale 事实。
            self.stall.drop_session(sid).await;
        }

        // Remove the mapping. Active/Dormant keys are indexed by session_id;
        // Spawning placeholders (no session_id) must be removed by key.
        if let Some(sid) = &session_id_opt {
            self.map.remove_by_session(sid).await;
        } else {
            self.map.remove_by_key(&key).await;
        }
        self.publish_removed(&key);

        // 在飞的自动模式切换随会话消亡：没有 driver 事件会再来，取走即弃
        // （防 recycled session_id 继承陈旧记录）。
        if let Some(sid) = &session_id_opt {
            let _ = self.auto_mode_switches.take(sid).await;
        }
        self.reply_targets.clear(&key).await;

        // Clear the active pointer if this was the focused session.
        let mut active = self.active_session.write().await;
        if active.as_ref() == Some(&key) {
            *active = None;
        }
        CloseOutcome::Closed { discarded_pending }
    }

    /// Emit a per-turn card and ContinueSession command.
    ///
    /// Shared by `inbound::continue_session` and `drain_queue_if_terminal`
    /// to eliminate the identical card-emission logic between them. Resets
    /// CardState, seeds a fresh card, emits SendCard, then emits SendAcp
    /// to drive the next turn.
    async fn emit_turn_card(
        &self,
        key: ChannelKey,
        session_id: &str,
        prompt: String,
        root_id: Option<String>,
    ) {
        self.card_states.drop(session_id).await;
        self.seed_card(session_id.to_string(), prompt.clone()).await;
        let theme_color = self.card_cfg.read().await.theme_color.clone();
        let turn_prompt = prompt.clone();
        let card = ChannelCard {
            title: String::new(),
            theme: theme_color,
            elements: Vec::new(),
            turn: Some(TurnChrome {
                prompt: turn_prompt,
                session_id: session_id.to_string(),
                usage: None,
            }),
        };
        self.emit(Out::SendCard {
            key: key.clone(),
            card,
            // Record the new card under the session so streaming UpdateCards
            // resolve to THIS turn's card (previous turn stays frozen).
            msg_id: Some(session_id.to_string()),
            perm_request_id: None,
            perm_meta: None,
            root_id,
        })
        .await;
        self.emit(Out::SendAcp {
            session_id: session_id.to_string(),
            cmd: AcpCommand::ContinueSession {
                session_id: session_id.to_string(),
                prompt,
            },
        })
        .await;
        // A new turn reset the card state (phase back to seed): publish so
        // detached frontends flip the row off done/working immediately.
        // Covers both the continue_session and drain_queue_if_terminal paths.
        self.publish_updated(&key).await;
    }

    /// 提交一次回合（workbench-turn-queue design D3）：所有通道共用的唯一
    /// in-flight 判定与投递入口。卡片仍在 WORKING → `enqueue_turn` 进
    /// back-pressure（D4：发一次 Updated，pending 即时可见、last_active 不变；
    /// prompt **不**写 transcript——它由 `emit_turn_card`→`seed_card` 在开轮
    /// 时写入）；否则翻转 DONE/FAILED → WORKING 并开新轮（出卡 + SendAcp）。
    /// Feishu 的 `inbound::continue_session` 与 web 的 `TextRoute::Continue`
    /// 分支都改调它，两条路径不再各写一份判定。
    async fn submit_turn(
        &self,
        key: ChannelKey,
        session_id: &str,
        prompt: String,
        priority: bool,
        reply_to: Option<String>,
        origin: TurnOrigin,
    ) {
        use crate::card_state::phase::{SEED, WORKING};

        // In-flight check: if the session's card is still streaming (WORKING),
        // don't reset/don't POST a new card/don't SendAcp. Enqueue this turn
        // instead (back-pressure); Feishu additionally signals it with a ⏳
        // reaction on the in-flight card.
        // （fix-webui-qa-defects-round4 2.3）接收回执相位（SEED 且 prompt 已
        // 随开轮记入卡态）同样在飞：此前该窗口的新提交会走 settled 臂把在飞
        // 回合的卡态整个重置（emit_turn_card drop+seed），在飞回合与卡面被
        // 二次提交顶掉。空 prompt 的 SEED（resume 激活语义）不算在飞——
        // 没有回合在跑，新提交照常开轮。
        let in_flight = match self.card_states.snapshot(session_id).await {
            Some(st) => {
                st.status_emoji == WORKING
                    || (st.status_emoji == SEED && !st.user_prompt.is_empty())
            }
            None => false,
        };
        if in_flight {
            self.map
                .enqueue_turn(
                    &key,
                    crate::state::QueuedTurn::new(prompt, reply_to, priority),
                )
                .await;
            if origin.is_feishu() {
                self.emit_reaction(session_id, "⏳").await;
            }
            // D4：入队即发布 Updated——pending 随 SessionInfo 首次出现在观察
            // 面，且 last_active 已随 route_text 触碰（recent 排序不变）。
            self.publish_updated(&key).await;
            return;
        }

        // Settled path: DONE/FAILED -> flip to WORKING, flush, react, then emit
        // per-turn card + SendAcp. The prompt lands in the transcript at turn
        // start (seed_card), never at submission time (D4).
        let flipped = self
            .card_states
            .apply(session_id, |st| {
                if matches!(
                    st.status_emoji.as_str(),
                    crate::card_state::phase::DONE | crate::card_state::phase::FAILED
                ) {
                    st.status_emoji = WORKING.into();
                    true
                } else {
                    false
                }
            })
            .await;
        if flipped {
            self.flush_card(session_id).await;
            if origin.is_feishu() {
                self.emit_reaction(session_id, WORKING).await;
            }
        }

        // Emit the per-turn card that becomes the new streaming target
        // (MsgIdMap flips to this card). Reset CardState so streaming
        // body accumulates fresh (not appended to previous turn's body).
        self.emit_turn_card(key, session_id, prompt, reply_to).await;
    }

    /// Drain ONE queued turn if the session is in a terminal state (DONE/FAILED)
    /// and the queue is non-empty. Resets CardState and emits SendCard + SendAcp
    /// for the next turn.
    ///
    /// Shared between [`inbound::continue_session`] and
    /// [`acp_events::apply_event_to_out`]: both non-terminal settle paths
    /// (Finished, incidental settle from streaming events) call this to pop
    /// the next queued turn. Terminal errors abandon the queue — the session
    /// is being torn down and queued turns are dropped alongside.
    pub(super) async fn drain_queue_if_terminal(&self, key: &ChannelKey, session_id: &str) {
        use crate::card_state::phase::{DONE, FAILED};

        // Only drain if status is terminal and queue has entries.
        let Some(emoji) = self.card_states.status_emoji(session_id).await else {
            return;
        };
        if !matches!(emoji.as_str(), DONE | FAILED) {
            return;
        }
        if self.map.queue_len(key).await == 0 {
            return;
        }

        // Pop the next turn (FIFO, /btw priority slot already applied at enqueue time).
        let Some(next) = self.map.pop_next_turn(key).await else {
            return;
        };

        // Reset CardState and emit the per-turn card + ContinueSession.
        self.emit_turn_card(key.clone(), session_id, next.prompt, next.reply_to)
            .await;
    }

    /// fix-pending-queue-liveness：看门狗阈值装配点（`[dispatch]
    /// turn_stall_timeout`，秒；0 = 关闭）。
    pub fn set_turn_stall_timeout(&self, secs: u64) {
        self.stall.set_timeout_secs(secs);
    }

    /// 停滞看门狗事实登记表（仅测试：回拨时钟模拟长停滞）。
    #[doc(hidden)]
    pub fn stall_registry(&self) -> &stall::StallRegistry {
        &self.stall
    }

    /// 看门狗扫描 + 强制收尾（fix-pending-queue-liveness 2.2，design D5）。
    ///
    /// 对每个「WORKING/SEED 相位、无泊车审批、超过 `turn_stall_timeout` 无
    /// 任何事件」的活跃会话，把停滞回合按「回合异常结束、会话存活」收尾
    /// （对齐 `AcpEvent::Error { terminal: false }` 臂的既有语义：SEED/
    /// WORKING → DONE + drain），随后发 [`SessionEvent::TurnStalled`] 点名
    /// 会话与释放的搁浅条目数。不引入新事件类型、不拆会话。
    ///
    /// 由核心进程的周期任务调用（`src/run.rs` 装配）；`timeout = 0` 时扫描
    /// 在登记表内短路，本方法即 no-op。返回本次收尾的 `(key, released)`
    /// 供测试断言。
    pub async fn force_settle_stalled_turns(&self) -> Vec<(ChannelKey, usize)> {
        let mut settled = Vec::new();
        for facts in self.stall.stalled_sessions().await {
            let sid = facts.session_id.clone();
            // 泊车豁免已按引擎事实在扫描层过滤（design D2）；这里再核对
            // 「仍有活跃映射 + 相位仍可收尾」，把扫描与收尾窗口内的竞态
            // （回合已自行结束/会话已拆除）挡在收尾之前。
            let Some(key) = self.map.lookup_key_by_session(&sid).await else {
                self.stall.drop_session(&sid).await;
                continue;
            };
            // fix-webui-qa-defects 3.1（design D2）：0-turn 占位不构成在飞回合
            // ——占位旗标仍在的映射（未经首条消息/激活消费）绝不强收、绝不向
            // 其 transcript 注入合成错误。真实回合的判定在旗标翻转后照常。
            if self.map.get(&key).await.is_some_and(|m| m.awaiting_first_prompt()) {
                continue;
            }
            let phase = self.card_states.status_emoji(&sid).await;
            if !matches!(
                phase.as_deref(),
                Some(crate::card_state::phase::SEED) | Some(crate::card_state::phase::WORKING)
            ) {
                // 终态驻留（正常 Finished 等）的旧时钟条目顺手回收：每轮扫描
                // 都会撞上 phase 检查，留着只是白耗一枚哈希槽（纯效率，时钟
                // 本就不再参与判定）。
                self.stall.drop_session(&sid).await;
                continue;
            }
            // fix-webui-qa-defects 3.1（design D2 补口）：从未开过轮的会话同样
            // 不是在飞回合——聚焦即拉起（激活 spawn）只握手、无 prompt，卡片
            // 经 apply_event 的 lazy seed 落在 SEED 且 transcript 为空；它在
            // turn_stall_timeout 后曾被强收并写入「回合停滞被强制收尾」合成
            // 错误（占位幽灵回合）。真实回合开轮必经 seed_card 落下 prompt
            // 条目，transcript 非空；空 transcript = 没有任何回合可收尾。
            // 不 drop 时钟：该会话首条消息开轮后由既有路径照常计时。
            {
                let g = self.turn_log.read().await;
                if g.get(&sid).is_none_or(|log| log.is_empty()) {
                    continue;
                }
            }
            // 收尾锚定（防误收尾）：仅 SEED/WORKING 转移到 DONE，与
            // non-terminal Error 臂同款契约。
            let settled_now = self
                .card_states
                .apply(&sid, |st| {
                    if matches!(
                        st.status_emoji.as_str(),
                        crate::card_state::phase::SEED | crate::card_state::phase::WORKING
                    ) {
                        st.status_emoji = crate::card_state::phase::DONE.into();
                        true
                    } else {
                        false
                    }
                })
                .await;
            if !settled_now {
                continue;
            }
            // transcript 就地点名（会话内可见「回合为何被收尾」）；数字是
            // 收尾时刻的搁浅条目数——drain 会让队头开轮，其余随之解除卡死。
            // （fix-webui-qa-defects 5.1，design D5）错误条目带 stall 分类，
            // 前端气泡标签渲染「回合停滞」而非通用 spawn failed。
            let released = self.map.queue_len(&key).await;
            self.transcript_push(
                &sid,
                TurnEntry::error(
                    0,
                    format!(
                        "**回合停滞被强制收尾**：超过 {} 秒无任何事件（最后事件 {} 秒前），队列中 {} 条待执行提交已解除卡死。",
                        self.stall.timeout_secs(),
                        facts.silent_for_secs,
                        released
                    ),
                )
                .with_failure_class(failure_class::STALL),
            )
            .await;
            self.flush_card(&sid).await;
            // 时钟退役：drain 开轮的下一回合由 seed_card 重置计时；队列空时
            // 会话停在终态，扫描不再看它。
            self.stall.drop_session(&sid).await;
            if released > 0 {
                self.drain_queue_if_terminal(&key, &sid).await;
            }
            self.publish_updated(&key).await;
            tracing::warn!(
                session_id = %sid,
                last_event_unix = facts.last_event_unix,
                silent_for_secs = facts.silent_for_secs,
                released,
                "turn stalled: no events for the configured timeout; force-settled to DONE and drained the queue (fix-pending-queue-liveness)"
            );
            self.publish(SessionEvent::TurnStalled {
                channel: key.channel_str().to_string(),
                key: key.reference.clone(),
                released,
            });
            settled.push((key, released));
        }
        settled
    }
}

/// 零输出回合合成提示的固定文案（close-acceptance-blind-spots 4.1）：说明
/// 「回合已结束且无输出」，措辞与停滞强收条目同风格（加粗导语 + 冒号说明）。
pub(crate) const ZERO_OUTPUT_NOTICE: &str =
    "**回合已结束且无输出**：本轮回合未产生任何可见输出（正文、thinking、工具、错误皆无）。";

/// 一段 transcript 片段是否包含**可见输出**条目（close-acceptance-blind-spots
/// 4.1，纯函数）：`kind = "content"` 且 `element_type ∈ {markdown, thinking,
/// tool, error}`、内容非空即算。空内容条目对前端不可见（`count_chat_messages`
/// 与前端分组同规则），不算可见输出。
fn turn_has_visible_output(segment: &[TurnEntry]) -> bool {
    segment.iter().any(|e| {
        e.kind == TurnKind::Content
            && matches!(
                e.element_type,
                TurnElementType::Markdown
                    | TurnElementType::Thinking
                    | TurnElementType::Tool
                    | TurnElementType::Error
            )
            && !e.content.is_empty()
    })
}

/// 该条目是否为零输出合成提示（close-acceptance-blind-spots 4.1，纯函数）：
/// 防御异常的重复 Finished——片段里已有 notice 就不再追加第二条。
fn is_zero_output_notice(e: &TurnEntry) -> bool {
    e.element_type == TurnElementType::Notice
}

pub fn compose_media_prompt(caption: &str, files: &[String]) -> String {
    let mut out = String::new();
    if !caption.is_empty() {
        out.push_str(caption);
        out.push('\n');
    }
    out.push_str("\n[attached: ");
    out.push_str(&files.join(", "));
    out.push(']');
    out
}

fn text_from_caption(c: &Option<String>) -> String {
    c.clone().unwrap_or_default()
}

// ---- 时间戳与会话键编解码：收敛到唯一实现（add-domain-layer 2.3/2.5） ----

pub use sebas_domain::prim::now_unix;

/// 编码 `ChannelKey` 为 URL-safe 形态。唯一实现在 `sebas_channels::key`
/// （add-domain-layer 2.2）；这里仅保留既有公开路径
/// （`native_dispatch_bridge` 等调用方零改动）。
pub use sebas_channels::key::encode_session_key as encode_key;

/// 解码 percent-encoded `ChannelKey`。canonical 严格解码之外保留旧的
/// feishu 回退（无 NUL 分隔的输入整段当飞书 reference）——该回退只对
/// 非 wire 形态的输入可达（wire 上的键全部出自 [`encode_key`]，必有 NUL），
/// 收敛后行为与旧手写实现逐字节一致（黄金样本钉）。
pub fn decode_key(encoded: &str) -> Option<ChannelKey> {
    sebas_channels::key::decode_session_key(encoded)
        .or_else(|| Some(ChannelKey::feishu(&sebas_channels::key::percent_decode(encoded)?, None)))
}

fn extract_session_id(event: &AcpEvent) -> &str {
    match event {
        AcpEvent::TextDelta { session_id, .. }
        | AcpEvent::ThinkingDelta { session_id, .. }
        | AcpEvent::ToolStart { session_id, .. }
        | AcpEvent::ToolProgress { session_id, .. }
        | AcpEvent::ToolEnd { session_id, .. }
        | AcpEvent::PermissionRequest { session_id, .. }
        | AcpEvent::Finished { session_id }
        | AcpEvent::Error { session_id, .. }
        | AcpEvent::UsageUpdate { session_id, .. }
        | AcpEvent::ModelChanged { session_id, .. }
        | AcpEvent::ModeChanged { session_id, .. }
        | AcpEvent::AvailableCommands { session_id, .. } => session_id,
    }
}

/// status emoji FSM（openspec/specs/feishu-cards/spec.md）。返回 Some(新emoji_type) 表示转移；None 表示
/// 不变。seed=SEED（"Typing"）；首个流式事件 -> WORKING（"OnIt"）；
/// Finished -> DONE（"DONE"）；terminal Error -> FAILED（"CrossMark"）；
/// 已 WORKING/DONE/FAILED 不回退 SEED。
///
/// 这些字符串是 Feishu reaction API 的合法 emoji_type（unicode 👀/🚧/✅/❌
/// 会被 Feishu 拒绝 231001）。它们以 root 卡上的 reaction 呈现会话状态；
/// 卡 header 标题则只显示主题（`cards::derive_topic`）。
fn next_emoji(current: &str, event: &AcpEvent) -> Option<&'static str> {
    use crate::card_state::phase::{DONE, FAILED, SEED, WORKING};
    match event {
        AcpEvent::Finished { .. } => Some(DONE),
        AcpEvent::Error { terminal: true, .. } => Some(FAILED),
        AcpEvent::TextDelta { .. }
        | AcpEvent::ThinkingDelta { .. }
        | AcpEvent::ToolStart { .. }
        | AcpEvent::ToolProgress { .. }
        | AcpEvent::ToolEnd { .. }
        | AcpEvent::Error {
            terminal: false, ..
        } => {
            if current == SEED {
                Some(WORKING)
            } else {
                None
            }
        }
        AcpEvent::PermissionRequest { .. } => None,
        AcpEvent::UsageUpdate { .. } => None,
        AcpEvent::ModelChanged { .. } => None,
        AcpEvent::ModeChanged { .. } => None,
        // （session-slash-commands）命令表刷新不是回合内容，FSM 不转移。
        AcpEvent::AvailableCommands { .. } => None,
    }
}
