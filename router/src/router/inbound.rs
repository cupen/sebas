//! 入站飞书事件处理：文本/媒体/按钮回调 → `Out` 指令。
//!
//! `RouterHandle` 的 impl 延续块（子模块可访问私有字段），从 router.rs 拆出。
//! `drain_queue_if_terminal` 被 [`acp_events`] 复用，故放在 mod.rs 里。

use super::{Out, RouterHandle, compose_media_prompt, text_from_caption};
use crate::commands::{Command, HELP_TEXT, parse_command};
use crate::settings;
use acp_claude::session::{AcpCommand, Decision};
use feishu::cards::{
    CardConfig, CardElement, DivText, ThinkingDisplay, render_accumulated_card,
    render_dead_session_card, render_expired_permission_card,
    render_resolved_permission_card,
};
use feishu::events::{CardAction, FeishuIn, SessionKey};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

impl RouterHandle {
    pub async fn dispatch(&self, evt: FeishuIn) {
        match evt {
            FeishuIn::Text {
                key,
                text,
                reply_to,
            } => {
                // Acknowledge receipt immediately with an emoji reaction on
                // the user's message, before any processing.
                if let Some(ref msg_id) = reply_to {
                    self.emit(Out::AckMsg {
                        message_id: msg_id.clone(),
                        emoji: "EYES".into(),
                    })
                    .await;
                }
                self.on_text(key, text, reply_to).await;
            }
            FeishuIn::Media {
                key,
                files,
                caption,
                reply_to,
            } => {
                let prompt = compose_media_prompt(&text_from_caption(&caption), &files);
                self.on_text(key, prompt, reply_to).await;
            }
            FeishuIn::ButtonCb { key, action } => self.on_button(key, action).await,
            FeishuIn::FormCb {
                key,
                value,
                form_value,
                message_id,
            } => self.on_form_cb(key, value, form_value, message_id).await,
        }
    }

    /// 表单容器提交回调：按负载里的 `form` 字段路由到已接线的 CRUD 表单
    /// （provider-preset / provider-custom 共两张）。未接线的表单仅记日志，
    /// 不静默吞掉。
    async fn on_form_cb(
        &self,
        key: SessionKey,
        value: Value,
        form_value: BTreeMap<String, Value>,
        message_id: Option<String>,
    ) {
        tracing::debug!(?key, ?value, "form callback received");
        let routed = match &self.provider_forms {
            Some(forms) => {
                let form_name = value.get("form").and_then(Value::as_str).unwrap_or("");
                if let Some(form) = forms.dispatch(form_name) {
                    let out = form.handle(key, &value, &form_value, message_id).await;
                    self.emit(out).await;
                    true
                } else {
                    false
                }
            }
            None => false,
        };
        if !routed {
            tracing::debug!("form callback for unwired form; ignored");
        }
    }

    async fn on_text(&self, key: SessionKey, text: String, reply_to: Option<String>) {
        // 记录最近入站回复目标：话题内 = 话题根消息 message_id（events 层已
        // 归一化），主线 = 触发消息 message_id。话题出站卡（权限卡/初始卡）
        // 用它作为 root_id，保证回复聚合在原话题。
        if let Some(target) = &reply_to {
            self.reply_targets.set(key.clone(), target.clone()).await;
        }
        match parse_command(&text) {
            Command::New => {
                match self.map.begin_spawn(key.clone()).await {
                    Ok(crate::state::BeginSpawn::AlreadySpawning) => {
                        // A spawn is already in flight for this chat; a second
                        // /new would orphan the in-flight session.
                        tracing::debug!("spawn already in flight; ignoring duplicate /new");
                    }
                    Ok(_) => self.spawn_new(key, String::new(), reply_to).await,
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
            Command::Upgrade { dev, dry_run } => {
                self.request_watchdog_upgrade(key, dev, dry_run).await;
            }
            Command::Rollback => {
                self.request_watchdog_rollback(key).await;
            }
            Command::Restart => {
                self.request_watchdog_restart(key).await;
            }
            Command::Services => {
                self.request_watchdog_services(key).await;
            }
            Command::Provider => self.on_provider(key).await,
            Command::PassThrough(p) => {
                match self.map.route_text(key.clone(), p.clone()).await {
                    Ok(crate::state::TextRoute::Continue(sid)) => {
                        self.continue_session(sid, p, reply_to, key.clone(), false)
                            .await
                    }
                    Ok(crate::state::TextRoute::SpawnNew) => self.spawn_new(key, p, reply_to).await,
                    Ok(crate::state::TextRoute::Resume(old_sid)) => {
                        // Restored mapping claimed for lazy respawn (spec §3.3e).
                        self.emit(Out::SpawnResume {
                            key,
                            session_id: old_sid,
                            prompt: p,
                            input_msg_id: reply_to,
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
                    Ok(crate::state::TextRoute::SpawnNew) => self.spawn_new(key, text, reply_to).await,
                    Ok(crate::state::TextRoute::Resume(old_sid)) => {
                        self.emit(Out::SpawnResume {
                            key,
                            session_id: old_sid,
                            prompt: text,
                            input_msg_id: reply_to,
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
            Command::Compact => {
                let sid = self
                    .map
                    .get(&key)
                    .await
                    .and_then(|m| m.session_id().map(str::to_owned));
                if let Some(sid) = sid {
                    self.forward_compact(&sid, key.clone()).await;
                } else {
                    self.emit(Out::HelpText { key }).await;
                }
            }
            Command::Cost | Command::Cancel | Command::Status => {
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

    async fn request_watchdog_upgrade(&self, key: SessionKey, dev: bool, dry_run: bool) {
        self.emit(Out::WatchdogUpgrade { key, dev, dry_run }).await;
    }

    async fn request_watchdog_rollback(&self, key: SessionKey) {
        self.emit(Out::WatchdogRollback { key }).await;
    }

    async fn request_watchdog_restart(&self, key: SessionKey) {
        self.emit(Out::WatchdogRestart { key }).await;
    }

    async fn request_watchdog_services(&self, key: SessionKey) {
        self.emit(Out::WatchdogServices { key }).await;
    }

    async fn on_button(&self, key: SessionKey, action: CardAction) {
        // Provider CRUD 按钮（新增/编辑/删除）与 ACP 会话无关，优先路由，
        // 避免权限卡的 session 存活检查误伤（例如聊天里没有活跃会话时仍可管理 provider）。
        if let Some(forms) = &self.provider_forms {
            let payload = action.value.pointer("/action/value").cloned();
            let op = payload
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|p| p.get("op"))
                .and_then(Value::as_str);
            let target = match op {
                // create 按钮：payload.form 直接指明走 preset 还是 custom。
                Some("create") => payload
                    .as_ref()
                    .and_then(Value::as_object)
                    .and_then(|p| p.get("form"))
                    .and_then(Value::as_str)
                    .and_then(|n| forms.dispatch(n))
                    .map(Arc::clone),
                // edit/delete：按存储里 item.preset 是否设置判定走哪张表单。
                Some("edit") | Some("delete") => {
                    let id = payload
                        .as_ref()
                        .and_then(Value::as_object)
                        .and_then(|p| p.get("id"))
                        .and_then(Value::as_str);
                    match id {
                        Some(id) => forms.pick_for_edit(id).await,
                        None => None,
                    }
                }
                _ => None,
            };
            if let Some(form) = target {
                let message_id = action
                    .value
                    .pointer("/context/open_message_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let out = form
                    .handle(
                        key,
                        payload.as_ref().unwrap_or(&Value::Null),
                        &BTreeMap::new(),
                        message_id,
                    )
                    .await;
                self.emit(out).await;
                return;
            }
            // 「取消」单独走 ProviderForms::cancel：直接渲染双入口列表卡
            // （保留「＋ 新增（预设/自定义）」），不走单表单的 handle()，
            // 否则会出现「取消后只剩一个 ＋ 新增按钮」的回归。
            if op == Some("cancel") {
                let message_id = action
                    .value
                    .pointer("/context/open_message_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let out = forms.cancel(key, message_id).await;
                self.emit(out).await;
                return;
            }
        }
        // （下方保留原有的 ACP 按钮处理兜底：权限卡、ACP 操作按钮等）
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

    /// `/provider`：打开 provider CRUD 列表卡（展示当前 provider + 两套
    /// 新增/编辑/删除按钮）。未接线时退回帮助。
    async fn on_provider(&self, key: SessionKey) {
        match &self.provider_forms {
            Some(forms) => self.emit(forms.open(key).await).await,
            None => self.emit(Out::HelpText { key }).await,
        }
    }

    async fn spawn_new(&self, key: SessionKey, prompt: String, input_msg_id: Option<String>) {
        // A fresh session must not inherit "本会话不再询问" grants from the
        // previous session in this chat — the user approved those for the
        // session that asked, not for whatever comes next.
        self.allowlist.clear(&key).await;
        // 新会话也不继承上一条入站的回复目标（话题内 root_id）。和 allowlist
        // 一样随会话终止清理，防止 ReplyTargetMap 无界增长。
        self.reply_targets.clear(&key).await;
        // Only emit SpawnAcp. The root card is sent by the dispatcher *after*
        // `create_session` mints the real session_id, so the card's MsgIdMap
        // entry (and later streaming UpdateCards) key off that session_id.
        // `input_msg_id` rides along so the dispatcher can point the session's
        // state reactions at the user's input message.
        self.emit(Out::SpawnAcp {
            key,
            prompt,
            input_msg_id,
        })
        .await;
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

        // Emit the per-turn card that becomes the new streaming target
        // (MsgIdMap flips to this card). Reset CardState so streaming
        // body accumulates fresh (not appended to previous turn's body).
        self.emit_turn_card(key, &session_id, prompt, root_id).await;
    }

    async fn forward_to_session(&self, session_id: &str, text: String) {
        let cmd = match parse_command(&text) {
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

    /// `/compact` 命令：发送进度卡片，每 1s×5 次 → 3s 间隔更新，完成后自动停止。
    async fn forward_compact(&self, session_id: &str, key: SessionKey) {
        let prompt = "⚙️ 压缩上下文...";
        self.card_states
            .seed(session_id.to_string(), prompt.into())
            .await;

        // Send initial card
        let theme_color = self.card_cfg.read().await.theme_color.clone();
        let card = render_accumulated_card(prompt, session_id, &[], &theme_color, None);
        self.emit(Out::SendCard {
            key: key.clone(),
            card: serde_json::to_value(&card).unwrap(),
            msg_id: Some(session_id.to_string()),
            perm_request_id: None,
            perm_meta: None,
            root_id: None,
        })
        .await;

        // Spawn background progress task
        let handle = self.clone();
        let sid = session_id.to_string();
        tokio::spawn(async move {
            // 1s × 5, then 3s for remaining iterations
            for i in 0..30 {
                let secs = if i < 5 { 1u64 } else { 3 };
                tokio::time::sleep(Duration::from_secs(secs)).await;

                // Check if done or card state dropped
                let emoji = handle.card_states.status_emoji(&sid).await;
                let done = match emoji.as_deref() {
                    Some(e) => {
                        e == crate::card_state::phase::DONE
                            || e == crate::card_state::phase::FAILED
                    }
                    None => true,
                };
                if done {
                    break;
                }

                // Update body with elapsed time
                handle
                    .card_states
                    .apply(&sid, |st| {
                        let elapsed = st.started_at.elapsed();
                        let secs = elapsed.as_secs();
                        st.body = vec![CardElement::Div {
                            text: DivText {
                                tag: "plain_text".into(),
                                content: format!("⏱ 正在压缩... {secs}s"),
                                text_size: Some("standard".into()),
                                text_color: None,
                            },
                        }];
                    })
                    .await;

                handle.flush_card(&sid).await;
            }
        });

        // Send compact command to the CLI
        self.emit(Out::SendAcp {
            session_id: session_id.to_string(),
            cmd: AcpCommand::ContinueSession {
                session_id: session_id.to_string(),
                prompt: "/compact".into(),
            },
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
            "max_tool_output_chars" => {
                format!("max_tool_output_chars = {}", cfg.max_tool_output_chars)
            }
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
                other => return Err(format!("thinking 可选值: show, hide（拒绝: {other})")),
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
