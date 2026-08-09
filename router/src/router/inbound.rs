//! 入站飞书事件处理：文本/媒体/按钮回调 → `Out` 指令。
//!
//! `RouterHandle` 的 impl 延续块（子模块可访问私有字段），从 router.rs 拆出。
//! `drain_queue_if_terminal` 被 [`acp_events`] 复用，故放在 mod.rs 里。

use super::{Out, RouterHandle, compose_media_prompt, text_from_caption};
use crate::commands::{Command, HELP_TEXT, parse_command};
use crate::settings;
use acp_claude::session::{AcpCommand, Decision};
use feishu::cards::{
    CardConfig, ThinkingDisplay, phase_visual, render_accumulated_card,
    render_dead_session_card, render_expired_permission_card,
    render_resolved_permission_card,
};
use feishu::events::{CardAction, FeishuIn, SessionKey};

impl RouterHandle {
    pub async fn dispatch(&self, evt: FeishuIn) {
        match evt {
            FeishuIn::Text {
                key,
                text,
                reply_to,
            } => self.on_text(key, text, reply_to).await,
            FeishuIn::Media {
                key,
                files,
                caption,
            } => {
                let prompt = compose_media_prompt(&text_from_caption(&caption), &files);
                self.on_text(key, prompt, None).await;
            }
            FeishuIn::ButtonCb { key, action } => self.on_button(key, action).await,
        }
    }

    async fn on_text(&self, key: SessionKey, text: String, reply_to: Option<String>) {
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
                self.emit(Out::PlainText {
                    key,
                    content: HELP_TEXT.into(),
                })
                .await;
            }
            Command::Settings(setting_key, val) => {
                self.handle_settings(key, setting_key, val, &settings::settings_path())
                    .await;
            }
            Command::PassThrough(p) => {
                match self.map.route_text(key.clone(), p.clone()).await {
                    Ok(crate::state::TextRoute::Continue(sid)) => {
                        self.continue_session(sid, p, reply_to, key.clone(), false)
                            .await
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
            Command::Btw(text) => {
                // /btw: same routing as PassThrough, but priority=true so it jumps the queue.
                match self.map.route_text(key.clone(), text.clone()).await {
                    Ok(crate::state::TextRoute::Continue(sid)) => {
                        self.continue_session(sid, text, reply_to, key.clone(), true)
                            .await
                    }
                    Ok(crate::state::TextRoute::SpawnNew) => self.spawn_new(key, text).await,
                    Ok(crate::state::TextRoute::Resume(old_sid)) => {
                        self.emit(Out::SpawnResume {
                            key,
                            session_id: old_sid,
                            prompt: text,
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
                card: serde_json::to_value(&card).expect("dead-session card serializes"),
                msg_id: None,
                perm_request_id: None,
                perm_meta: None,
                // Permission cards are fire-and-forget (no threading).
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
        // Stale click = no tracked perm-card entry (already resolved by a
        // prior click, or the click raced the dispatcher's msg_id record).
        // We still send the PermissionReply below — the bridge drops replies
        // for unknown request_ids, but withholding the reply when the hook
        // IS still parked would leave the tool call hanging forever.
        let mut stale_click = false;
        if let Some(rid) = action.request_id.as_deref() {
            if let Some(entry) = self.take_perm_card(rid).await {
                // "本会话不再询问" puts the chat in allow-all mode so every
                // subsequent permission request auto-approves without
                // prompting. The bridge side can't see the difference
                // (AllowSession maps to "allow_always" which is just
                // approve-per-call) — the allowlist lives on the sebas side
                // and intercepts before the user is even asked.
                if matches!(decision, Decision::AllowSession) {
                    self.allowlist.grant_all(&entry.key).await;
                }
                let label = match decision {
                    Decision::AllowOnce => "✅ 已允许（仅此一次）",
                    Decision::AllowSession => "✅ 已允许（本会话不再询问）",
                    Decision::Deny => "❌ 已拒绝",
                };
                let card = render_resolved_permission_card(label);
                self.emit(Out::UpdateCardByMsgId {
                    key: entry.key,
                    msg_id: entry.msg_id,
                    card: serde_json::to_value(&card).expect("resolved-permission card serializes"),
                })
                .await;
            } else {
                stale_click = true;
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
                self.emit(Out::HelpText { key: key.clone() }).await;
            }
        }
        if stale_click {
            // The card couldn't be flipped in place — show expired so the
            // user knows it was already handled. Emitted after the reply so
            // the bridge hook unblocks first.
            let card = render_expired_permission_card();
            self.emit(Out::SendCard {
                key: key.clone(),
                card: serde_json::to_value(&card).expect("expired-permission card serializes"),
                msg_id: None,
                perm_request_id: None,
                perm_meta: None,
                // Expired permission card is fire-and-forget.
                root_id: None,
            })
            .await;
        }
    }

    async fn spawn_new(&self, key: SessionKey, prompt: String) {
        // A fresh session must not inherit "本会话不再询问" grants from the
        // previous session in this chat — the user approved those for the
        // session that asked, not for whatever comes next.
        self.allowlist.clear(&key).await;
        // Only emit SpawnAcp. The root card is sent by the dispatcher *after*
        // `create_session` mints the real session_id, so the card's MsgIdMap
        // entry (and later streaming UpdateCards) key off that session_id.
        self.emit(Out::SpawnAcp { key, prompt }).await;
    }

    async fn continue_session(
        &self,
        session_id: String,
        prompt: String,
        root_id: Option<String>,
        key: SessionKey,
        priority: bool,
    ) {
        use crate::card_state::phase::WORKING;

        // In-flight check: if the session's card is still streaming (WORKING),
        // don't reset/don't POST a new card/don't SendAcp. Instead enqueue this
        // turn and emit a ⏳ reaction on the in-flight card to signal back-pressure.
        let in_flight = matches!(
            self.card_states.status_emoji(&session_id).await.as_deref(),
            Some(WORKING)
        );
        if in_flight {
            self.map
                .enqueue_turn(
                    &key,
                    crate::state::QueuedTurn {
                        prompt,
                        reply_to: root_id,
                        priority,
                    },
                )
                .await;
            self.emit_reaction(&session_id, "⏳").await;
            return;
        }

        // Settled path: DONE/FAILED -> flip to WORKING, flush, react, then emit
        // per-turn card + SendAcp.
        let flipped = self
            .card_states
            .apply(&session_id, |st| {
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
            self.flush_card(&session_id).await;
            self.emit_reaction(&session_id, WORKING).await;
        }

        // Emit per-turn card with root_id so the dispatcher sends a fresh card
        // that becomes the new streaming target (MsgIdMap flips to this card).
        // Reset CardState so streaming body accumulates fresh (not appended to
        // previous turn's body).
        self.card_states.drop(&session_id).await;
        self.seed_card(session_id.clone(), prompt.clone()).await;
        let seed_emoji = phase_visual(crate::card_state::phase::SEED);
        let theme_color = self.card_cfg.read().await.theme_color.clone();
        let card = render_accumulated_card(
            &prompt,
            &session_id,
            seed_emoji,
            &[],
            &theme_color,
        );
        self.emit(Out::SendCard {
            key,
            card: serde_json::to_value(&card).unwrap(),
            // Record the new card under the session so streaming UpdateCards
            // resolve to THIS turn's card (previous turn stays frozen).
            msg_id: Some(session_id.clone()),
            perm_request_id: None,
            perm_meta: None,
            root_id,
        })
        .await;

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

    pub async fn handle_settings(
        &self,
        key: SessionKey,
        setting_key: Option<String>,
        val: Option<String>,
        path: &std::path::Path,
    ) {
        let mut cfg = self.card_cfg.read().await.clone();

        let content = match (setting_key, val) {
            (None, _) => self.render_settings_list(&cfg, path),
            (Some(k), None) => self.render_setting(&cfg, &k),
            (Some(k), Some(v)) => match self.apply_setting(&mut cfg, &k, &v) {
                Ok(()) => {
                    // Persist + apply live.
                    if let Err(e) = settings::save_settings(path, &cfg) {
                        self.emit(Out::PlainText {
                            key,
                            content: format!("保存失败: {e}"),
                        })
                        .await;
                        return;
                    }
                    self.set_card_config(cfg.clone()).await;
                    format!("{k} = {v} (已写入 {})", path.display())
                }
                Err(msg) => msg,
            },
        };
        self.emit(Out::PlainText { key, content }).await;
    }

    pub fn render_settings_list(&self, cfg: &CardConfig, path: &std::path::Path) -> String {
        format!(
            "当前设置（来源：{}）:\n\
             thinking = {}\n\
             max_user_text_chars = {}\n\
             max_tool_output_chars = {}\n\
             fold_long_output = {}\n\
             theme_color = {}",
            path.display(),
            Self::thinking_label(cfg.thinking),
            cfg.max_user_text_chars,
            cfg.max_tool_output_chars,
            cfg.fold_long_output,
            cfg.theme_color,
        )
    }

    fn render_setting(&self, cfg: &CardConfig, k: &str) -> String {
        match k {
            "thinking" => format!("thinking = {}", Self::thinking_label(cfg.thinking)),
            "max_user_text_chars" => format!("max_user_text_chars = {}", cfg.max_user_text_chars),
            "max_tool_output_chars" => format!("max_tool_output_chars = {}", cfg.max_tool_output_chars),
            "fold_long_output" => format!("fold_long_output = {}", cfg.fold_long_output),
            "theme_color" => format!("theme_color = {}", cfg.theme_color),
            other => format!(
                "未知键: {other}\n可用键: thinking, max_user_text_chars, max_tool_output_chars, fold_long_output, theme_color"
            ),
        }
    }

    fn apply_setting(&self, cfg: &mut CardConfig, k: &str, v: &str) -> Result<(), String> {
        match k {
            "thinking" => match v {
                "show" => cfg.thinking = ThinkingDisplay::Show,
                "hide" => cfg.thinking = ThinkingDisplay::Hide,
                other => return Err(format!(
                    "thinking 可选值: show, hide（拒绝: {other})"
                )),
            },
            "max_user_text_chars" => {
                cfg.max_user_text_chars = v.parse().map_err(|e| format!("数字解析失败: {e}"))?
            }
            "max_tool_output_chars" => {
                cfg.max_tool_output_chars = v.parse().map_err(|e| format!("数字解析失败: {e}"))?
            }
            "fold_long_output" => match v {
                "true" => cfg.fold_long_output = true,
                "false" => cfg.fold_long_output = false,
                other => return Err(format!("布尔值应为 true / false（拒绝: {other}）")),
            },
            "theme_color" => cfg.theme_color = v.into(),
            other => return Err(format!("未知键: {other}")),
        }
        Ok(())
    }

    fn thinking_label(t: ThinkingDisplay) -> &'static str {
        match t {
            ThinkingDisplay::Show => "show",
            ThinkingDisplay::Hide => "hide",
        }
    }
}