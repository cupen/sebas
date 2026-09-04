//! 路由核心：`Out` 指令集 + `RouterHandle`（状态登记表 + 卡片状态操作）。
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

pub use events::{SessionEvent, SessionInfo, TurnEntry};
pub use maps::{
    MsgIdMap, PermCardEntry, PermCardMap, ReplyTargetMap, SessionAllowlist, tool_signature,
};

use crate::card_events::{
    apply_event_to_card, card_needs_rotation, count_folded_items, update_parent_title,
};
use crate::card_state::CardState;
use crate::cards::CardConfig;
use crate::commands::{Command, GatewayAction};
use crate::crud::ProviderForms;
use crate::state::{Mapping, SessionMap};
use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use sebas_channels::card::{AppUsage, ChannelCard, TurnChrome};
use sebas_channels::key::ChannelKey;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLock, broadcast, mpsc};

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
    /// `/gateway on|off|restart|status` — 管理 gateway 服务（openspec/specs/router-commands/spec.md）。
    WatchdogGateway {
        key: ChannelKey,
        action: GatewayAction,
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
    WebSpawn {
        key: ChannelKey,
        prompt: String,
        project_dir: Option<String>,
        kind: Option<String>,
        model: Option<String>,
    },
}

/// Result of `RouterHandle::web_close_session` — callers use this to
/// render a useful error message when the key isn't found (vs. silently
/// no-op'ing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseOutcome {
    /// Session was found and torn down (mapping dropped, child killed).
    Closed,
    /// No mapping exists for `key` (already closed, or stale URL).
    NotFound,
}

pub struct RouterHandle {
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
    /// Per-chat approval state for "本会话不再询问". When a new
    /// `PermissionRequest` arrives, the router checks this and auto-
    /// approves without rendering a card. The bridge sees the same
    /// approve/deny either way; the difference is purely UX.
    /// Scope: per-ChannelKey (= per feishu chat/thread). Cleared when the
    /// session is removed (`/new`, terminal error, daemon restart).
    allowlist: SessionAllowlist,
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
    /// [`RouterHandle::reply_target`] 读取。纯内存、不持久化。
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
    /// Per-session rendered transcript (`session_id` → ordered entries),
    /// the source for the WebUI/channel turn-content retrieval. Dropped
    /// with the mapping so a recycled session_id cannot inherit stale
    /// content. In-memory only.
    turn_log: Arc<RwLock<HashMap<String, Vec<TurnEntry>>>>,
}

impl Clone for RouterHandle {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            tx: self.tx.clone(),
            msgid: self.msgid.clone(),
            input_msg: self.input_msg.clone(),
            card_states: self.card_states.clone(),
            card_cfg: self.card_cfg.clone(),
            perm_cards: self.perm_cards.clone(),
            allowlist: self.allowlist.clone(),
            provider_forms: self.provider_forms.clone(),
            mgr: self.mgr.clone(),
            native: self.native.clone(),
            active_session: self.active_session.clone(),
            reply_targets: self.reply_targets.clone(),
            help_card_msgid: self.help_card_msgid.clone(),
            events: self.events.clone(),
            perm_events: self.perm_events.clone(),
            turn_log: self.turn_log.clone(),
        }
    }
}

impl RouterHandle {
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
        (
            Self {
                map,
                tx,
                msgid: MsgIdMap::default(),
                input_msg: MsgIdMap::default(),
                card_states: crate::card_state::CardStateMap::default(),
                card_cfg: Arc::new(RwLock::new(card_cfg)),
                perm_cards: PermCardMap::default(),
                allowlist: SessionAllowlist::default(),
                provider_forms,
                mgr,
                native: Arc::new(RwLock::new(native)),
                active_session: Arc::new(RwLock::new(None)),
                turn_log: Arc::new(RwLock::new(HashMap::new())),
                reply_targets: ReplyTargetMap::default(),
                help_card_msgid: MsgIdMap::default(),
                events,
                perm_events,
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
        let (status, session_id) = match &m.state {
            crate::state::MappingState::Active { session_id } => {
                ("active", Some(session_id.clone()))
            }
            crate::state::MappingState::Dormant { session_id } => {
                ("dormant", Some(session_id.clone()))
            }
            crate::state::MappingState::Spawning { .. } => ("spawning", None),
        };
        let (phase, user_prompt) = match session_id.as_ref() {
            Some(sid) => match self.card_states.snapshot(sid).await {
                Some(st) => (Some(st.status_emoji), Some(st.user_prompt)),
                None => (None, None),
            },
            None => (None, None),
        };
        Some(SessionInfo {
            channel: key.channel_str().to_string(),
            key: key.reference.clone(),
            session_id,
            status: status.into(),
            phase,
            user_prompt,
            last_active_unix: m.last_active_unix,
            project_dir: m.project_dir.clone(),
            current_model: m.current_model.clone(),
            available_models: m.available_models.clone(),
        })
    }

    /// The session's transcript after `from` (monotonic positions).
    /// `None` when no mapping exists for `key`; a session without a
    /// transcript (Spawning, or no content yet) yields an empty vec.
    pub async fn session_turns(&self, key: &ChannelKey, from: u64) -> Option<Vec<TurnEntry>> {
        let m = self.map.get(key).await?;
        let Some(sid) = m.session_id() else {
            return Some(Vec::new());
        };
        let g = self.turn_log.read().await;
        Some(
            g.get(sid)
                .map(|log| log.iter().filter(|e| e.position >= from).cloned().collect())
                .unwrap_or_default(),
        )
    }

    /// Append one entry to a session's transcript. Position is assigned
    /// monotonically from the log length.
    async fn transcript_push(&self, session_id: &str, mut entry: TurnEntry) {
        let mut g = self.turn_log.write().await;
        let log = g.entry(session_id.to_string()).or_default();
        entry.position = log.len() as u64;
        log.push(entry);
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

    /// Subscribe to the ACP permission broadcast (design D6): every
    /// `AcpEvent::PermissionRequest` the router applies is forwarded here as
    /// an independent side-channel, parallel to the Feishu card path. Lagging
    /// subscribers see `RecvError::Lagged` (advisory — a missed card can be
    /// recovered from the session transcript, never a correctness loss).
    pub fn subscribe_acp_permission_requests(&self) -> broadcast::Receiver<AcpEvent> {
        self.perm_events.subscribe()
    }

    /// Publish a native-kernel permission request onto the same ACP permission
    /// broadcast (make-feishu-optional-webui-primary, design D3). The webui
    /// `InProcessBackend` relays `AcpEvent::PermissionRequest` into its
    /// review-card feed already — reusing the shape means feishu-originated
    /// native sessions surface permission cards for free, encoded key lookup
    /// included. `session_id` is the URL-safe encoded `ChannelKey`.
    pub fn publish_native_permission(
        &self,
        session_id: String,
        request_id: String,
        tool_name: String,
        args: serde_json::Value,
    ) {
        let _ = self.perm_events.send(AcpEvent::PermissionRequest {
            session_id,
            request_id,
            tool_name,
            args,
        });
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
    pub async fn emit(&self, out: Out) {
        if let Err(e) = self.tx.send(out).await {
            tracing::error!(?e, "router→outbound channel closed; dropping message");
            debug_assert!(false, "router→outbound channel send failed: {e}");
        }
    }

    pub async fn dump_json(&self) -> serde_json::Result<String> {
        self.map.dump_json().await
    }

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

    /// Per-chat approval state set by "本会话不再询问". Tests use this to
    /// seed and inspect entries; the production path goes through
    /// `apply_event_to_out` (auto-approve) and `on_button` (grant on click)
    /// without reaching for the field directly.
    pub fn allowlist(&self) -> &SessionAllowlist {
        &self.allowlist
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
        let cfg = self.card_cfg.read().await;
        // transcript 条目在 apply 闭包外追加（锁序：card_states → turn_log，
        // 与其他路径不交叉）。TextDelta/Thinking/Tool 事件是内容流，逐条入账。
        // ModelChanged（add-acp-model-selection）：更新映射 current model 并
        // 发布 Updated，让快照立即反映中程切换 —— 覆盖流式 pump（apply_event）
        // 与即时路径（apply_event_to_out）两条到达线。
        if let AcpEvent::ModelChanged { model_id, .. } = event {
            if let Some(key) = self.map.lookup_key_by_session(session_id).await {
                self.map.set_current_model(&key, model_id.clone()).await;
                self.publish_updated(&key).await;
            }
        }
        match event {
            AcpEvent::TextDelta { delta, .. } => {
                self.transcript_push(session_id, TurnEntry::markdown(0, delta.clone()))
                    .await;
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
                self.transcript_push(
                    session_id,
                    TurnEntry::markdown(0, format!("📖 **{tool_name}**\n```json\n{args_str}\n```")),
                )
                .await;
            }
            AcpEvent::ToolEnd {
                tool_name, result, ..
            } => {
                self.transcript_push(
                    session_id,
                    TurnEntry::markdown(0, format!("✓ **{tool_name}**\n{result}")),
                )
                .await;
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
        if next.is_some()
            && let Some(key) = self.map.lookup_key_by_session(session_id).await
        {
            self.publish_updated(&key).await;
        }
        next
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
        let existed = self.map.get(key).await.is_some();
        let pending = self.map.activate(key, session_id, acp_session_id, model).await;
        if existed {
            self.publish_updated(key).await;
        } else {
            self.publish_created(key).await;
        }
        pending
    }

    /// 会话模型切换成功（AcpEvent::ModelChanged）后更新映射的 current model，
    /// 并发布 Updated 让快照/订阅者立即反映新模型。
    pub async fn apply_model_changed(&self, session_id: &str, model_id: &str) {
        if let Some(key) = self.map.lookup_key_by_session(session_id).await {
            self.map.set_current_model(&key, model_id.to_string()).await;
            self.publish_updated(&key).await;
        }
    }

    /// Spawn failed/timeout: remove the Spawning placeholder for `key`.
    pub async fn fail_spawn(&self, key: &ChannelKey) {
        // Only publish when a placeholder was actually removed — fail_spawn
        // is a no-op for Active/Dormant mappings.
        let was_spawning = self
            .map
            .get(key)
            .await
            .map(|m| matches!(m.state, crate::state::MappingState::Spawning { .. }))
            .unwrap_or(false);
        self.map.fail_spawn(key).await;
        if was_spawning {
            self.publish_removed(key);
        }
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
    ) -> ChannelKey {
        let key = ChannelKey::web_new();
        match self.map.begin_spawn(key.clone()).await {
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

    /// Send a message to an existing session from the WebUI.
    /// Routes the message through the session map (same logic as Feishu
    /// text messages) and emits the appropriate Out instruction. Command
    /// text is parsed like the Feishu path (B 档冒烟 2026-09-04：webui 直达
    /// 路径此前把 `/cancel` 当普通 prompt 发给 opencode，中断无效）。
    pub async fn web_send_message(&self, key: ChannelKey, message: String) {
        match crate::commands::parse_command(&message) {
            // 命令臂：无活跃会话明确回复（与 feishu 路径一致，sebas-ixv）。
            Command::Cost | Command::Cancel | Command::Status => {
                let sid = self.map.get(&key).await.and_then(|m| m.session_id().map(str::to_owned));
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
                        Command::Cancel => AcpCommand::Cancel { session_id: sid.clone() },
                        _ => unreachable!(),
                    };
                    self.emit(Out::SendAcp { session_id: sid, cmd }).await;
                } else {
                    let cmd = message.split_whitespace().next().unwrap_or("");
                    self.emit(Out::PlainText {
                        key,
                        content: format!("当前没有活跃会话，{cmd} 需要活跃会话。发送 /new 开始新会话。"),
                    })
                    .await;
                }
                return;
            }
            Command::Compact => {
                let sid = self.map.get(&key).await.and_then(|m| m.session_id().map(str::to_owned));
                if let Some(sid) = sid {
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
                        content: "当前没有活跃会话，/compact 需要活跃会话。发送 /new 开始新会话。".into(),
                    })
                    .await;
                }
                return;
            }
            // 其余命令/普通文本：webui 侧不处理的命令保持原行为（不静默
            // 截留），与 feishu 路径的 PassThrough 语义一致。
            _ => {}
        }
        match self.map.route_text(key.clone(), message.clone()).await {
            Ok(crate::state::TextRoute::Continue(sid)) => {
                // 记录本轮用户 prompt 到 transcript（与 feishu 路径的
                // seed_card 对齐）：否则 WebUI composer 发出的消息不出现在
                // 会话记录里，turn-content 检索漏掉这一轮的提问。
                self.transcript_push(&sid, TurnEntry::prompt(0, message.clone()))
                    .await;
                // The route touched last_active on the mapping — push an
                // Updated so subscribers refresh recency (and phase).
                self.publish_updated(&key).await;
                self.emit(Out::SendAcp {
                    session_id: sid.clone(),
                    cmd: AcpCommand::ContinueSession {
                        session_id: sid,
                        prompt: message,
                    },
                })
                .await;
            }
            Ok(crate::state::TextRoute::SpawnNew) => {
                // route_text inserted a Spawning placeholder for `key`.
                self.publish_created(&key).await;
                self.emit(Out::WebSpawn {
                    key,
                    prompt: message,
                    project_dir: None,
                    kind: None,
                    // 直达消息路径没有模型参数（用户指定模型走创建表单）。
                    model: None,
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
                tracing::debug!("web message queued (session already spawning)");
            }
            Err(e) => {
                tracing::warn!(?e, "web_send_message: route_text failed");
            }
        }
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
    /// - Clears the chat-level permission allowlist and the per-key reply
    ///   target (topic root message_id) so recycled keys don't inherit
    ///   stale aggregation targets.
    /// - Clears `active_session` if this key was the focused one.
    pub async fn web_close_session(&self, key: ChannelKey) -> CloseOutcome {
        let Some(mapping) = self.map.get(&key).await else {
            return CloseOutcome::NotFound;
        };
        let session_id_opt = mapping.session_id().map(|s| s.to_string());

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
        }

        // Remove the mapping. Active/Dormant keys are indexed by session_id;
        // Spawning placeholders (no session_id) must be removed by key.
        if let Some(sid) = &session_id_opt {
            self.map.remove_by_session(sid).await;
        } else {
            self.map.remove_by_key(&key).await;
        }
        self.publish_removed(&key);

        self.allowlist.clear(&key).await;
        self.reply_targets.clear(&key).await;

        // Clear the active pointer if this was the focused session.
        let mut active = self.active_session.write().await;
        if active.as_ref() == Some(&key) {
            *active = None;
        }
        CloseOutcome::Closed
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

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Encode a `ChannelKey` for URLs / the channel wire: `channel\0reference`
/// percent-encoded. A channel-prefixed pair round-trips exactly; the feishu
/// reference keeps its `chat\0thread` composite inside the reference field.
/// No external dependency — the charset is small (channel names and
/// references are alnum plus a few separators) so a targeted percent-encoder
/// suffices.
pub fn encode_key(key: &ChannelKey) -> String {
    let raw = format!("{}\0{}", key.channel_str(), &key.reference);
    percent_encode(&raw)
}

/// Decode a percent-encoded `ChannelKey` (inverse of [`encode_key`]).
pub fn decode_key(encoded: &str) -> Option<ChannelKey> {
    let decoded = percent_decode(encoded)?;
    match decoded.split_once('\0') {
        Some((channel, reference)) => Some(ChannelKey::new(channel, reference)),
        None => Some(ChannelKey::feishu(&decoded, None)),
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1])?;
                let lo = hex_val(bytes[i + 2])?;
                out.push(hi << 4 | lo);
                i += 3;
            }
            b'%' => return None,
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
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
        | AcpEvent::ModelChanged { session_id, .. } => session_id,
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
    }
}
