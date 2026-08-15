//! 路由核心：`Out` 指令集 + `RouterHandle`（状态登记表 + 卡片状态操作）。
//!
//! impl 按职责拆到子模块（Rust 子模块可访问私有字段，无需放宽可见性）：
//! - 入站飞书事件（文本/按钮 → Out）: [`inbound`]
//! - ACP 事件 → Out: [`acp_events`]
//! - 登记表类型（MsgIdMap/PermCardMap/SessionAllowlist）: [`maps`]

mod acp_events;
mod inbound;
mod maps;

pub use maps::{MsgIdMap, PermCardEntry, PermCardMap, SessionAllowlist, tool_signature};

use crate::card_events::{apply_event_to_card, card_needs_rotation, count_folded_items, update_parent_title};
use crate::card_state::CardState;
use crate::crud::ProviderForms;
use crate::state::{Mapping, SessionMap};
use acp_claude::manager::SessionManager;
use acp_claude::session::{AcpCommand, AcpEvent};
use feishu::cards::{CardConfig, CardElement, DivText, render_accumulated_card};
use feishu::events::SessionKey;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLock, mpsc};

#[derive(Debug)]
pub enum Out {
    SpawnAcp {
        key: SessionKey,
        prompt: String,
        /// Feishu `message_id` of the input message that triggered this spawn
        /// (the user's own message, i.e. `FeishuIn::Text.reply_to`). Recorded
        /// so the session's state reactions (👀/🚧/✅/❌) land on that message
        /// instead of the card. `None` for `/new`, WebUI, or replay spawns.
        input_msg_id: Option<String>,
    },
    /// Lazily respawn a restored session (spec §3.3e): try `session/load`
    /// with `session_id`; the dispatcher falls back to a fresh session when
    /// the agent cannot load it.
    SpawnResume {
        key: SessionKey,
        session_id: String,
        prompt: String,
        /// Feishu `message_id` of the input message that triggered this resume
        /// (`FeishuIn::Text.reply_to`), threaded through so the resumed
        /// session's cards reply to that message. `None` for WebUI resumes.
        input_msg_id: Option<String>,
    },
    SendAcp {
        session_id: String,
        cmd: AcpCommand,
    },
    SendCard {
        key: SessionKey,
        card: serde_json::Value,
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
        key: SessionKey,
        msg_id: String,
        card: serde_json::Value,
    },
    UpdateCard {
        session_id: String,
        card: serde_json::Value,
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
        key: SessionKey,
    },
    /// Plain-text reply to the originating chat (e.g. `/settings`, `/help`).
    /// The dispatcher uses FeishuClient::send_text — not a card.
    PlainText {
        key: SessionKey,
        content: String,
    },
    WatchdogUpgrade {
        key: SessionKey,
        dev: bool,
        dry_run: bool,
    },
    WatchdogRollback {
        key: SessionKey,
    },
    WatchdogRestart {
        key: SessionKey,
    },
    WatchdogServices {
        key: SessionKey,
    },
    /// Spawn a session without sending a root card to Feishu (web-originated
    /// sessions). The dispatcher creates the ACP session and wires the pump,
    /// but skips the Feishu send_card / react operations. Card content is
    /// still accumulated in CardStateMap and readable via the WebUI.
    /// `project_dir` specifies the working directory for the Claude Code
    /// process (if None, falls back to the config default).
    WebSpawn {
        key: SessionKey,
        prompt: String,
        project_dir: Option<String>,
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
    /// Scope: per-SessionKey (= per Feishu chat/thread). Cleared when the
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
    /// The session currently focused by the WebUI. The dashboard uses this
    /// to highlight the active row and to decide which session's detail
    /// page to deep-link into. `None` until the user clicks Switch on a
    /// row (or opens a session detail page).
    active_session: Arc<RwLock<Option<SessionKey>>>,
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
            active_session: self.active_session.clone(),
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
        Self::new_with_provider_form(map, card_cfg, 256, None, Some(mgr))
    }

    /// 带 provider CRUD 表单的完整构造（root crate 启动时注入）。
    pub fn new_with_provider_form(
        map: SessionMap,
        card_cfg: CardConfig,
        channel_buffer: usize,
        provider_forms: Option<Arc<ProviderForms>>,
        mgr: Option<Arc<SessionManager>>,
    ) -> (Self, mpsc::Receiver<Out>) {
        let (tx, rx) = mpsc::channel(channel_buffer);
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
                active_session: Arc::new(RwLock::new(None)),
            },
            rx,
        )
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
    pub async fn session_snapshot(&self) -> Vec<(SessionKey, Mapping)> {
        self.map.snapshot_all().await
    }

    /// Snapshot all card states (for WebUI session detail).
    pub async fn card_state_snapshot(&self) -> HashMap<String, CardState> {
        self.card_states.snapshot_all().await
    }

    /// Snapshot the MsgIdMap (for message_id lookup).
    pub async fn msgid_snapshot(&self) -> HashMap<String, String> {
        self.msgid.snapshot_all().await
    }

    /// Send an `Out` to the outbound pump. Per spec §4.1 ("Channel send
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
        key: SessionKey,
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

    /// seed_card：SpawnAcp 臂发完 root 卡后调用（dispatch_out）。
    /// 幂等：已存在则保留（防 SpawnAcp 重入冲掉已累积状态）。spec §4.2。
    pub async fn seed_card(&self, session_id: String, user_prompt: String) {
        self.card_states.seed(session_id, user_prompt).await;
    }

    /// apply_event：纯状态变更（FSM emoji + apply_event_to_card append/截断/总量）。
    /// 不发 Out。session 无 CardState 时 lazy seed（prompt="" 兜底）。spec §4.2。
    ///
    /// 返回 `Some(新 emoji)` 表示 FSM 发生转移 —— 由调用方决定是否发
    /// `Out::React`（本方法保持纯状态契约），见 `emit_reaction`。
    pub async fn apply_event(&self, session_id: &str, event: &AcpEvent) -> Option<&'static str> {
        let cfg = self.card_cfg.read().await;
        self.card_states
            .apply(session_id, |st| {
                // Handle usage updates separately — they don't affect the FSM
                // or the card body, but update accumulated token counts.
                if let AcpEvent::UsageUpdate { usage, .. } = event {
                    if let Some(model) = &usage.model {
                        st.usage.model = Some(model.clone());
                    }
                    if let Some(input) = usage.input_tokens {
                        st.usage.round_input += input;
                        st.usage.total_input += input;
                    }
                    if let Some(output) = usage.output_tokens {
                        st.usage.round_output += output;
                        st.usage.total_output += output;
                    }
                    return None;
                }
                // FSM（spec §5）
                let next = next_emoji(&st.status_emoji, event);
                if let Some(e) = next {
                    st.status_emoji = e.into();
                }
                // On Finished, reset round counters for the next turn.
                if matches!(event, AcpEvent::Finished { .. }) {
                    st.usage.round_input = 0;
                    st.usage.round_output = 0;
                }
                apply_event_to_card(&mut st.body, event, &cfg);
                next
            })
            .await
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

    /// flush_card：快照 → render_accumulated_card → Out::UpdateCard。
    /// 无 CardState 则 no-op。spec §4.2。节流契约保证 flush 只在 debounce 到点或
    /// Finished/terminal 即时被调，故不维护 dirty flag。
    pub async fn flush_card(&self, session_id: &str) {
        let Some(st) = self.card_states.snapshot(session_id).await else {
            return;
        };
        let theme_color = self.card_cfg.read().await.theme_color.clone();
        // 更新父面板标题：添加已折叠项数和经过时间（"🤔 折腾中 · 3项 · 45s"）。
        let elapsed = st.started_at.elapsed();
        let count = count_folded_items(&st.body);
        let mut body = st.body.clone();
        update_parent_title(&mut body, count, &elapsed);
        let card = render_accumulated_card(
            &st.user_prompt,
            session_id,
            &body,
            &theme_color,
            Some(&st.usage),
        );
        self.emit(Out::UpdateCard {
            session_id: session_id.to_string(),
            card: serde_json::to_value(&card).expect("accumulated card serializes"),
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
        let card = render_accumulated_card(
            &st.user_prompt,
            session_id,
            &body,
            &theme_color,
            Some(&st.usage),
        );
        self.emit(Out::UpdateCard {
            session_id: session_id.to_string(),
            card: serde_json::to_value(&card).expect("accumulated card serializes"),
        })
        .await;

        // 2. 构造"接上条"提示，重置 body
        let continuation_note = CardElement::Div {
            text: DivText {
                tag: "plain_text".into(),
                content: "📎 接上条，内容继续".into(),
                text_size: Some("notation".into()),
                text_color: Some("grey".into()),
            },
        };
        let fresh_body = vec![continuation_note.clone()];
        self.card_states
            .reset_body(session_id, vec![continuation_note])
            .await;

        // 3. 发射新卡（SendCard），附带旧卡 message_id 作为 root_id 实现回复关系
        let old_msg_id = self.msgid.get(session_id).await;
        let new_card = render_accumulated_card(
            &st.user_prompt,
            session_id,
            &fresh_body,
            &theme_color,
            Some(&st.usage),
        );
        self.emit(Out::SendCard {
            key,
            card: serde_json::to_value(&new_card).expect("new card serializes"),
            msg_id: Some(session_id.to_string()),
            perm_request_id: None,
            perm_meta: None,
            root_id: old_msg_id,
        })
        .await;

        true
    }

    /// drop_card：session 死亡/通道关时清 CardState（防无界增长）。spec §4.2。
    pub async fn drop_card(&self, session_id: &str) {
        self.card_states.drop(session_id).await;
    }

    /// Record a `SessionKey -> session_id` mapping. Called by the dispatcher
    /// once `SessionManager::create_session` has minted the real session_id, so
    /// that continuations, permission-card routing (reverse lookup) and
    /// liveness checks can find the session.
    pub async fn insert_mapping(&self, key: SessionKey, session_id: String) {
        if let Err(e) = self
            .map
            .insert(key, crate::state::Mapping::active(session_id))
            .await
        {
            tracing::warn!(?e, "failed to insert session mapping");
        }
    }

    /// True if a live (Active) session is mapped for `key` (used to reject
    /// button callbacks that arrive after a session has ended, and to keep
    /// `/new` from double-spawning while a spawn is in flight).
    pub async fn session_alive(&self, key: &SessionKey) -> bool {
        self.map
            .get(key)
            .await
            .map(|m| m.session_id().is_some())
            .unwrap_or(false)
    }

    /// Flip Spawning -> Active for `key` and drain queued prompts.
    /// Called by the dispatcher once `create_session` has minted the id.
    pub async fn activate(&self, key: &SessionKey, session_id: String) -> Vec<String> {
        self.map.activate(key, session_id).await
    }

    /// Spawn failed/timeout: remove the Spawning placeholder for `key`.
    pub async fn fail_spawn(&self, key: &SessionKey) {
        self.map.fail_spawn(key).await;
    }

    /// Start a new session from the WebUI (no Feishu card operations).
    /// Uses `Out::WebSpawn` which the dispatcher handles without sending
    /// cards to Feishu. Returns the SessionKey for the new session.
    /// `project_dir` specifies the working directory for Claude Code.
    pub async fn web_spawn(&self, prompt: String, project_dir: Option<String>) -> SessionKey {
        let key = SessionKey::web_key();
        match self.map.begin_spawn(key.clone()).await {
            Ok(_) => {
                // Record project_dir on the mapping before emitting, so the
                // WebUI can display it even before the session is active.
                self.map.set_project_dir(&key, project_dir.clone()).await;
                self.emit(Out::WebSpawn {
                    key: key.clone(),
                    prompt,
                    project_dir,
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
    /// text messages) and emits the appropriate Out instruction.
    pub async fn web_send_message(&self, key: SessionKey, message: String) {
        match self.map.route_text(key.clone(), message.clone()).await {
            Ok(crate::state::TextRoute::Continue(sid)) => {
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
                self.emit(Out::WebSpawn {
                    key,
                    prompt: message,
                    project_dir: None,
                })
                .await;
            }
            Ok(crate::state::TextRoute::Resume(old_sid)) => {
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
    pub async fn web_set_active(&self, key: SessionKey) {
        let mut g = self.active_session.write().await;
        *g = Some(key);
    }

    /// Snapshot of the currently focused session, if any.
    pub async fn active_session_snapshot(&self) -> Option<SessionKey> {
        self.active_session.read().await.clone()
    }

    /// Tear down the session mapped to `key` from the WebUI.
    /// - Looks up the mapping; if Active, kills the child process via the
    ///   SessionManager.
    /// - Removes the mapping + drain queue (SessionMap does both).
    /// - Drops card state and root msg_id so recycled ids don't inherit
    ///   stale entries.
    /// - Clears the chat-level permission allowlist.
    /// - Clears `active_session` if this key was the focused one.
    pub async fn web_close_session(&self, key: SessionKey) -> CloseOutcome {
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
        }

        // Remove the mapping. Active/Dormant keys are indexed by session_id;
        // Spawning placeholders (no session_id) must be removed by key.
        if let Some(sid) = &session_id_opt {
            self.map.remove_by_session(sid).await;
        } else {
            self.map.remove_by_key(&key).await;
        }

        self.allowlist.clear(&key).await;

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
        key: SessionKey,
        session_id: &str,
        prompt: String,
        root_id: Option<String>,
    ) {
        self.card_states.drop(session_id).await;
        self.seed_card(session_id.to_string(), prompt.clone()).await;
        let theme_color = self.card_cfg.read().await.theme_color.clone();
        let card = render_accumulated_card(&prompt, session_id, &[], &theme_color, None);
        self.emit(Out::SendCard {
            key,
            card: serde_json::to_value(&card).unwrap(),
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
    pub(super) async fn drain_queue_if_terminal(&self, key: &SessionKey, session_id: &str) {
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
        | AcpEvent::UsageUpdate { session_id, .. } => session_id,
    }
}

/// status emoji FSM（spec §5）。返回 Some(新emoji_type) 表示转移；None 表示
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
    }
}
