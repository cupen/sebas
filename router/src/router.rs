use crate::commands::{parse_command, Command};
use crate::state::SessionMap;
use acp_claude::session::{AcpCommand, AcpEvent, Decision};
use feishu::cards::{phase_visual, CardConfig};
use feishu::cards::{
    apply_event_to_card, render_accumulated_card, render_dead_session_card,
    render_expired_permission_card, render_permission_card, render_resolved_permission_card,
};
use feishu::events::{CardAction, FeishuIn, SessionKey};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, RwLock};
use serde_json::Value;

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
            card: serde_json::to_value(&card).unwrap(),
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

    pub async fn dispatch(&self, evt: FeishuIn) {
        match evt {
            FeishuIn::Text { key, text, .. } => self.on_text(key, text).await,
            FeishuIn::Media {
                key,
                files,
                caption,
            } => {
                let prompt = compose_media_prompt(&text_from_caption(&caption), &files);
                self.on_text(key, prompt).await;
            }
            FeishuIn::ButtonCb { key, action } => self.on_button(key, action).await,
        }
    }

    /// Dispatch an inbound `AcpEvent`, extracting the session_id from the
    /// event payload and forwarding to `apply_event_to_out`.
    pub async fn dispatch_acp_event(&self, event: AcpEvent) {
        let session_id = extract_session_id(&event).to_owned();
        self.apply_event_to_out(session_id, &event).await;
    }

    /// apply_event_to_out：同步薄封装（apply_event + flush_card 即时出卡）。
    ///
    /// **Spec §6 偏差**：spec §6 说「dispatch_acp_event 改为调 apply_event 不发
    /// Out」，但 spec §9 要求 router_test/e2e_test/terminal_error_test 零改动通过
    /// —— 这些测试调 dispatch_acp_event 并断言立即收到 UpdateCard。故本计划保留
    /// dispatch_acp_event → apply_event_to_out（同步 flush），仅把 **pump** 从
    /// dispatch_acp_event 改为 apply_event + debounce + flush_card（Task 6）。
    pub async fn apply_event_to_out(&self, session_id: String, event: &AcpEvent) {
        match event {
            AcpEvent::PermissionRequest {
                session_id,
                request_id,
                tool_name,
                args,
            } => {
                // Resolve the SessionKey that owns this session so Feishu has a
                // real `receive_id`. Without this the card would carry an empty
                // chat_id and Feishu rejects it.
                let Some(key) = self.map.lookup_key_by_session(session_id).await else {
                    tracing::warn!(%session_id, "no SessionKey for permission request; dropping card");
                    return;
                };
                // Auto-approve if this (tool, args) was previously granted with
                // "Allow session" in this chat. No card, no user click — the
                // bridge gets the same AllowSession reply as a manual click
                // would have produced.
                if self.allowlist.is_allowed(&key, tool_name, args).await {
                    tracing::info!(
                        %session_id, %tool_name, %request_id,
                        "permission auto-approved by session allowlist"
                    );
                    self.emit(Out::SendAcp {
                        session_id: session_id.clone(),
                        cmd: AcpCommand::PermissionReply {
                            session_id: session_id.clone(),
                            request_id: request_id.clone(),
                            decision: Decision::AllowSession,
                        },
                    })
                    .await;
                    return;
                }
                let card = render_permission_card(session_id, request_id, tool_name, args);
                self.emit(Out::SendCard {
                    key,
                    card: serde_json::to_value(&card).unwrap(),
                    msg_id: None,
                    // Mark this card for in-place update on click. The dispatcher
                    // records the Feishu message_id keyed by request_id so a
                    // later button click can flip the card to "已允许/已拒绝"
                    // or "请求已过期" instead of leaving the user staring at
                    // a stale prompt they keep re-clicking.
                    perm_request_id: Some(request_id.clone()),
                    // Stash the call metadata so the click handler can
                    // register the (tool, args) signature in the session
                    // allowlist when the user picks "Allow session".
                    perm_meta: Some((tool_name.clone(), args.clone())),
                    // Permission cards are fire-and-forget (no threading).
                    root_id: None,
                })
                .await;
            }
            AcpEvent::Error { terminal: true, .. } => {
                // terminal Error 并入累积模型（spec §8）：apply_event（置 ❌ + append
                // 错误正文，保留死前 transcript）→ flush_card → 换 reaction →
                // remove_by_session → drop_card。
                let react = self.apply_event(session_id.as_str(), event).await;
                self.flush_card(session_id.as_str()).await;
                if let Some(emoji) = react {
                    self.emit_reaction(session_id.as_str(), emoji).await;
                }
                // Look up the chat key before removal so we can also drop
                // the session allowlist (per-chat "Allow session" memory).
                if let Some(key) = self.map.lookup_key_by_session(session_id.as_str()).await {
                    self.allowlist.clear(&key).await;
                }
                self.map.remove_by_session(session_id.as_str()).await;
                self.drop_card(session_id.as_str()).await;
            }
            _ => {
                // 流式事件 + Finished + 非 terminal Error：apply_event（状态）+ flush_card（同步出卡）。
                // FSM emoji 转移时紧跟一个 React（先出卡，后换 reaction）。
                let react = self.apply_event(session_id.as_str(), event).await;
                self.flush_card(session_id.as_str()).await;
                if let Some(emoji) = react {
                    self.emit_reaction(session_id.as_str(), emoji).await;
                }
            }
        }
    }

    async fn on_text(&self, key: SessionKey, text: String) {
        match parse_command(&text) {
            Command::New => {
                match self.map.begin_spawn(key.clone()).await {
                    Ok(crate::state::BeginSpawn::AlreadySpawning) => {
                        // A spawn is already in flight for this chat; a second
                        // /new would orphan the in-flight session.
                        tracing::debug!("spawn already in flight; ignoring duplicate /new");
                    }
                    Ok(_) => self.spawn_new(key, String::new()).await,
                    Err(e) => {
                        tracing::warn!(?e, "begin_spawn failed");
                        self.emit(Out::HelpText { key }).await;
                    }
                }
            }
            Command::Help => {
                self.emit(Out::HelpText { key }).await;
            }
            Command::PassThrough(p) => {
                match self.map.route_text(key.clone(), p.clone()).await {
                    Ok(crate::state::TextRoute::Continue(sid)) => {
                        self.continue_session(sid, p).await
                    }
                    Ok(crate::state::TextRoute::SpawnNew) => self.spawn_new(key, p).await,
                    Ok(crate::state::TextRoute::Resume(old_sid)) => {
                        // Restored mapping claimed for lazy respawn (spec §3.3e).
                        self.emit(Out::SpawnResume {
                            key,
                            session_id: old_sid,
                            prompt: p,
                        })
                        .await;
                    }
                    Ok(crate::state::TextRoute::Enqueued) => {}
                    Err(e) => {
                        tracing::warn!(?e, "route_text failed");
                        self.emit(Out::HelpText { key }).await;
                    }
                }
            }
            Command::Compact | Command::Cost | Command::Cancel | Command::Status => {
                let sid = self
                    .map
                    .get(&key)
                    .await
                    .and_then(|m| m.session_id().map(str::to_owned));
                if let Some(sid) = sid {
                    self.forward_to_session(&sid, text).await;
                } else {
                    self.emit(Out::HelpText { key }).await;
                }
            }
            _ => {
                self.emit(Out::HelpText { key }).await;
            }
        }
    }

    async fn on_button(&self, key: SessionKey, action: CardAction) {
        // If the session is gone (process exited / daemon restarted), the
        // permission reply has nowhere to go — tell the user instead of sending
        // a command into the void.
        if !self.session_alive(&key).await {
            let card = render_dead_session_card();
            self.emit(Out::SendCard {
                key,
                card: serde_json::to_value(&card).unwrap(),
                msg_id: None,
                perm_request_id: None,
                perm_meta: None,
                // Dead-session card is fire-and-forget (no thread reply).
                root_id: None,
            })
            .await;
            return;
        }
        let decision = match action.decision.as_deref() {
            Some("allow_once") => Decision::AllowOnce,
            Some("allow_session") => Decision::AllowSession,
            // Fail closed: unknown or missing decision is a deny.
            _ => Decision::Deny,
        };
        // Optimistically flip the card in place to "已处理" so a follow-up
        // click on the same card from a misclick doesn't show a stale prompt.
        // The dispatcher will see no perm-card entry on stale clicks (we
        // take it below) and render "请求已过期" instead.
        if let Some(rid) = action.request_id.as_deref() {
            if let Some(entry) = self.take_perm_card(rid).await {
                // "Allow session" registers the (tool, args) signature so
                // subsequent matching calls auto-approve without prompting.
                // The bridge side can't see the difference (AllowSession
                // maps to "allow_always" which is just approve-per-call) —
                // the allowlist lives on the sebas side and intercepts
                // before the user is even asked.
                if matches!(decision, Decision::AllowSession) {
                    self.allowlist
                        .grant(&entry.key, &entry.tool_name, &entry.args)
                        .await;
                }
                let label = match decision {
                    Decision::AllowOnce => "✅ 已允许（仅此一次）",
                    Decision::AllowSession => "✅ 已允许（本会话）",
                    Decision::Deny => "❌ 已拒绝",
                };
                let card = render_resolved_permission_card(label);
                self.emit(Out::UpdateCardByMsgId {
                    key: entry.key,
                    msg_id: entry.msg_id,
                    card: serde_json::to_value(&card).unwrap(),
                })
                .await;
            } else {
                // Stale click — the request was already resolved (by a prior
                // click) or the card was never tracked. Show expired so the
                // user knows their click had no effect.
                let card = render_expired_permission_card();
                self.emit(Out::SendCard {
                    key: key.clone(),
                    card: serde_json::to_value(&card).unwrap(),
                    msg_id: None,
                    perm_request_id: None,
                    perm_meta: None,
                    // Expired permission card is fire-and-forget.
                    root_id: None,
                })
                .await;
                return;
            }
        }
        match (action.session_id.clone(), action.request_id.clone()) {
            (sid, Some(rid)) => {
                self.emit(Out::SendAcp {
                    session_id: sid.clone(),
                    cmd: AcpCommand::PermissionReply {
                        session_id: sid,
                        request_id: rid,
                        decision,
                    },
                })
                .await;
            }
            _ => {
                self.emit(Out::HelpText { key }).await;
            }
        }
    }

    async fn spawn_new(&self, key: SessionKey, prompt: String) {
        // Only emit SpawnAcp. The root card is sent by the dispatcher *after*
        // `create_session` mints the real session_id, so the card's MsgIdMap
        // entry (and later streaming UpdateCards) key off that session_id.
        self.emit(Out::SpawnAcp { key, prompt }).await;
    }

    async fn continue_session(&self, session_id: String, prompt: String) {
        // 新 turn 回切（spec §5 回边）：上一 turn 已 settled（DONE/FAILED）时
        // 重置为 WORKING，先刷出回切后的卡再换 reaction，让用户看到会话重新
        // 进入工作状态。
        let flipped = self
            .card_states
            .apply(&session_id, |st| {
                if matches!(
                    st.status_emoji.as_str(),
                    crate::card_state::phase::DONE | crate::card_state::phase::FAILED
                ) {
                    st.status_emoji = crate::card_state::phase::WORKING.into();
                    true
                } else {
                    false
                }
            })
            .await;
        if flipped {
            self.flush_card(&session_id).await;
            self.emit_reaction(&session_id, crate::card_state::phase::WORKING).await;
        }
        self.emit(Out::SendAcp {
            session_id: session_id.clone(),
            cmd: AcpCommand::ContinueSession { session_id, prompt },
        })
        .await;
    }

    async fn forward_to_session(&self, session_id: &str, text: String) {
        let cmd = match parse_command(&text) {
            Command::Compact => AcpCommand::ContinueSession {
                session_id: session_id.into(),
                prompt: "/compact".into(),
            },
            Command::Cost => AcpCommand::ContinueSession {
                session_id: session_id.into(),
                prompt: "/cost".into(),
            },
            Command::Cancel => AcpCommand::Cancel {
                session_id: session_id.into(),
            },
            _ => return,
        };
        self.emit(Out::SendAcp {
            session_id: session_id.into(),
            cmd,
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

/// Tracks root-card message_ids per session so `UpdateCard` can resolve a
/// `session_id` to a `message_id` (Feishu's PATCH endpoint needs the
/// message_id, not the session_id).
#[derive(Default, Clone)]
pub struct MsgIdMap {
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl MsgIdMap {
    pub async fn record(&self, session_id: String, msg_id: String) {
        self.inner.write().await.insert(session_id, msg_id);
    }

    pub async fn get(&self, session_id: &str) -> Option<String> {
        self.inner.read().await.get(session_id).cloned()
    }
}

/// One outstanding permission card: the chat to PATCH, the Feishu message_id
/// to PATCH by, and the (tool_name, args) needed to register the call in
/// the session allowlist when the user picks "Allow session".
#[derive(Debug, Clone)]
pub struct PermCardEntry {
    pub key: SessionKey,
    pub msg_id: String,
    pub tool_name: String,
    pub args: Value,
}

/// Tracks outstanding permission cards by `request_id` so the router can flip
/// them in place when the user clicks (or mark them expired on a stale click).
/// Keyed by request_id.
#[derive(Default, Clone)]
pub struct PermCardMap {
    inner: Arc<RwLock<HashMap<String, PermCardEntry>>>,
}

impl PermCardMap {
    pub async fn record(
        &self,
        request_id: String,
        key: SessionKey,
        msg_id: String,
        tool_name: String,
        args: Value,
    ) {
        self.inner
            .write()
            .await
            .insert(request_id, PermCardEntry { key, msg_id, tool_name, args });
    }

    /// Take the entry for a given request_id. The entry is removed on
    /// `take` so a duplicate click finds nothing and is a no-op (Feishu still
    /// shows the resolved card; we don't re-update it).
    pub async fn take(&self, request_id: &str) -> Option<PermCardEntry> {
        self.inner.write().await.remove(request_id)
    }
}

/// Per-session tool allowlist. When a user clicks "Allow session" / "Allow
/// for this chat" on a permission card, the (tool_name, args) signature is
/// added here. Subsequent `PermissionRequest`s for the same signature in the
/// same chat are auto-approved without a card.
///
/// Signature is `format!("{tool_name}|{args_json}")` where `args_json` is
/// `serde_json::to_string` of the args value. Exact match — if Claude asks
/// with slightly different args (e.g. different cwd), it's a different entry.
#[derive(Default, Clone)]
pub struct SessionAllowlist {
    inner: Arc<RwLock<HashMap<SessionKey, std::collections::HashSet<String>>>>,
}

impl SessionAllowlist {
    /// Check whether a (tool_name, args) call is allowed for the given chat.
    /// Exact match on the canonical signature.
    pub async fn is_allowed(&self, key: &SessionKey, tool_name: &str, args: &Value) -> bool {
        let sig = tool_signature(tool_name, args);
        self.inner
            .read()
            .await
            .get(key)
            .map(|s| s.contains(&sig))
            .unwrap_or(false)
    }

    /// Record an "Allow session" approval. Idempotent.
    pub async fn grant(&self, key: &SessionKey, tool_name: &str, args: &Value) {
        let sig = tool_signature(tool_name, args);
        self.inner
            .write()
            .await
            .entry(key.clone())
            .or_default()
            .insert(sig);
    }

    /// Drop the allowlist for a chat (session ended). Called from
    /// `remove_by_session` and similar lifecycle hooks.
    pub async fn clear(&self, key: &SessionKey) {
        self.inner.write().await.remove(key);
    }
}

/// Canonical signature for matching tool calls. Canonicalizes `args` so
/// that two semantically-equal (tool, args) calls hash to the same string
/// regardless of:
///   - key order in objects (Claude may serialise the same object with
///     keys in different order on different invocations)
///   - null fields (Claude sometimes emits `parent_tool_use_id: null`
///     or other optional wrappers)
/// Array order is preserved (semantically meaningful for command args,
/// env, etc.).
pub fn tool_signature(tool_name: &str, args: &Value) -> String {
    let canonical = canonicalize_value(args);
    let args_str = serde_json::to_string(&canonical).unwrap_or_else(|_| "{}".to_string());
    format!("{tool_name}|{args_str}")
}

/// Recursively canonicalize a JSON value for stable hashing:
/// - Objects: drop `null` fields, sort remaining keys, recurse.
/// - Arrays: preserve order, recurse.
/// - Other: unchanged.
fn canonicalize_value(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), canonicalize_value(v)))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().collect())
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_value).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn msgid_record_and_get_round_trip() {
        let m = MsgIdMap::default();
        assert!(m.get("s1").await.is_none());
        m.record("s1".into(), "om_abc".into()).await;
        assert_eq!(m.get("s1").await.as_deref(), Some("om_abc"));
        // overwrite
        m.record("s1".into(), "om_def".into()).await;
        assert_eq!(m.get("s1").await.as_deref(), Some("om_def"));
        // isolation
        m.record("s2".into(), "om_xyz".into()).await;
        assert_eq!(m.get("s2").await.as_deref(), Some("om_xyz"));
    }
}
