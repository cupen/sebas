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

use crate::card_events::apply_event_to_card;
use crate::state::SessionMap;
use acp_claude::session::{AcpCommand, AcpEvent};
use feishu::cards::{CardConfig, phase_visual, render_accumulated_card};
use feishu::events::SessionKey;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Out {
    SpawnAcp {
        key: SessionKey,
        prompt: String,
    },
    /// Lazily respawn a restored session (spec §3.3e): try `session/load`
    /// with `session_id`; the dispatcher falls back to a fresh session when
    /// the agent cannot load it.
    SpawnResume {
        key: SessionKey,
        session_id: String,
        prompt: String,
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
        /// alongside `perm_request_id` so the click handler can register the
        /// call in the session allowlist when the user picks "Allow session".
        /// Ignored for non-permission cards.
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
    HelpText {
        key: SessionKey,
    },
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
    card_states: crate::card_state::CardStateMap,
    card_cfg: CardConfig,
    /// Tracks the Feishu `message_id` of each outstanding permission card,
    /// keyed by the card's `request_id`. Used to flip the card in place when
    /// the user clicks (or to mark it expired on a stale click). Entries are
    /// removed once resolved so a duplicate click doesn't re-update.
    perm_cards: PermCardMap,
    /// Per-session allowlist of `(tool_name, args)` signatures the user
    /// approved with "Allow session" / "Allow for this chat". When a new
    /// `PermissionRequest` arrives, the router checks this list and auto-
    /// approves matching calls without rendering a card. The bridge sees
    /// the same approve/deny either way; the difference is purely UX.
    /// Scope: per-SessionKey (= per Feishu chat/thread). Cleared when the
    /// session is removed (`/new`, terminal error, daemon restart).
    allowlist: SessionAllowlist,
}

impl Clone for RouterHandle {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            tx: self.tx.clone(),
            msgid: self.msgid.clone(),
            card_states: self.card_states.clone(),
            card_cfg: self.card_cfg.clone(),
            perm_cards: self.perm_cards.clone(),
            allowlist: self.allowlist.clone(),
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
        let (tx, rx) = mpsc::channel(channel_buffer);
        (
            Self {
                map,
                tx,
                msgid: MsgIdMap::default(),
                card_states: crate::card_state::CardStateMap::default(),
                card_cfg,
                perm_cards: PermCardMap::default(),
                allowlist: SessionAllowlist::default(),
            },
            rx,
        )
    }

    /// Send an `Out` to the outbound pump. Per spec §4.1 ("Channel send
    /// fail"): a closed channel is a bug in dev (panic via debug_assert)
    /// and an error-log-and-continue in prod — never a silent drop.
    async fn emit(&self, out: Out) {
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
    /// PATCH the card and, on "Allow session", register the call in the
    /// session allowlist. Returns `None` if no live card (already resolved,
    /// or never existed).
    pub async fn take_perm_card(&self, request_id: &str) -> Option<PermCardEntry> {
        self.perm_cards.take(request_id).await
    }

    /// Per-chat allowlist of (tool, args) signatures the user approved with
    /// "Allow session". Tests use this to seed and inspect entries; the
    /// production path goes through `apply_event_to_out` (auto-approve)
    /// and `on_button` (grant on click) without reaching for the field
    /// directly.
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
        let cfg = &self.card_cfg;
        self.card_states
            .apply(session_id, |st| {
                // FSM（spec §5）
                let next = next_emoji(&st.status_emoji, event);
                if let Some(e) = next {
                    st.status_emoji = e.into();
                }
                apply_event_to_card(&mut st.body, event, cfg);
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
        let card = render_accumulated_card(
            &st.user_prompt,
            session_id,
            phase_visual(&st.status_emoji),
            &st.body,
            &self.card_cfg.theme_color,
        );
        self.emit(Out::UpdateCard {
            session_id: session_id.to_string(),
            card: serde_json::to_value(&card).expect("accumulated card serializes"),
        })
        .await;
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

        // Reset CardState: drop the old turn's state, seed fresh for the new turn.
        self.card_states.drop(session_id).await;
        self.seed_card(session_id.to_string(), next.prompt.clone())
            .await;

        // Emit per-turn card with root_id = next.reply_to (threading via reply_to).
        let seed_emoji = phase_visual(crate::card_state::phase::SEED);
        let card = render_accumulated_card(
            &next.prompt,
            session_id,
            seed_emoji,
            &[],
            &self.card_cfg.theme_color,
        );
        self.emit(Out::SendCard {
            key: key.clone(),
            card: serde_json::to_value(&card).unwrap(),
            // Record the new card under the session so streaming UpdateCards
            // resolve to THIS turn's card (previous turn stays frozen).
            msg_id: Some(session_id.to_string()),
            perm_request_id: None,
            perm_meta: None,
            root_id: next.reply_to,
        })
        .await;

        // Emit ContinueSession for the new turn.
        self.emit(Out::SendAcp {
            session_id: session_id.to_string(),
            cmd: AcpCommand::ContinueSession {
                session_id: session_id.to_string(),
                prompt: next.prompt,
            },
        })
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
        | AcpEvent::Error { session_id, .. } => session_id,
    }
}

/// status emoji FSM（spec §5）。返回 Some(新emoji_type) 表示转移；None 表示
/// 不变。seed=SEED（"Typing"）；首个流式事件 -> WORKING（"OnIt"）；
/// Finished -> DONE（"DONE"）；terminal Error -> FAILED（"CrossMark"）；
/// 已 WORKING/DONE/FAILED 不回退 SEED。
///
/// 这些字符串是 Feishu reaction API 的合法 emoji_type（unicode 👀/🚧/✅/❌
/// 会被 Feishu 拒绝 231001）。`cards::phase_visual` 把它们映射成 card 头部
/// 显示用的 Unicode 字符。
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
    }
}
