//! 入站通道事件处理（中立事件模型）：文本/媒体/按钮回调 → `Out` 指令。
//!
//! `RouterHandle` 的 impl 延续块（子模块可访问私有字段），从 router.rs 拆出。
//! `drain_queue_if_terminal` 被 [`acp_events`] 复用，故放在 mod.rs 里。
//!
//! 入站统一走 `sebas_channels::ChannelEvent`（Text/Media/ButtonCb/FormCb）：
//! 通道相关的门禁（chat_type / 群聊 @ bot 检测 / 去重）已下沉到各通道适配器，
//! 核心只看到中立事件与 `ChannelKey`。

use super::{Out, RouterHandle, compose_media_prompt, text_from_caption};
use crate::cards::{CardConfig, ThinkingDisplay};
use crate::cards_ui;
use crate::commands::{Command, GatewayAction, parse_command};
use crate::settings;
use sebas_acp::claude::session::{AcpCommand, Decision};
use sebas_channels::card::ChannelCard;
use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// Sentinel value passed as `msg_id` in `Out::SendCard` for help cards.
/// The dispatcher recognizes this tag and records the real Feishu message_id
/// via `RouterHandle::record_help_card_msgid` for later in-place updates.
const HELP_CARD_TAG: &str = "__help_card__";

/// 判断两个 `ChannelKey` 是否属于同一个"会话上下文"（`/sessions` 列表按此
/// 归组）：必须同通道，且引用属于同一个 chat。引用是不透明字符串，对话层
/// 只做展示性切分——飞书引用是 `chat\0thread` 复合，`\0` 前的 chat 部分归组
/// （主线恰好没有 `\0`，即整个引用）；其它通道引用相等即同一上下文。
fn same_chat_context(a: &ChannelKey, b: &ChannelKey) -> bool {
    if a.channel != b.channel {
        return false;
    }
    chat_part(&a.reference) == chat_part(&b.reference)
}

/// 引用的 chat 部分（`\0` 前；无 `\0` 时整个引用即 chat）。
fn chat_part(reference: &str) -> &str {
    reference.split_once('\0').map(|(c, _)| c).unwrap_or(reference)
}

/// `/sessions` 列表里一行的话题后缀（` thread=<tid>`；无话题时空串）。
fn thread_label(reference: &str) -> String {
    reference
        .split_once('\0')
        .map(|(_, t)| format!(" thread={t}"))
        .unwrap_or_default()
}

impl RouterHandle {
    /// Dispatch one inbound channel event (channel-neutral). The channel
    /// adapter has already translated the wire payload and applied its own
    /// gating (chat type, mention, dedup); the core just routes.
    pub async fn dispatch(&self, evt: ChannelEvent) {
        match evt {
            ChannelEvent::Text {
                key,
                text,
                reply_target,
            } => {
                // Acknowledge receipt immediately with an emoji reaction on
                // the user's message, before any processing. "Get" = 👌 =
                // "已收到" 语义（Feishu `emoji_type`），不再是 "EYES"/"Typing"
                // —— 后者暗示"正在输入"。
                if let Some(ref msg_id) = reply_target {
                    self.emit(Out::AckMsg {
                        message_id: msg_id.clone(),
                        emoji: crate::card_state::phase::SEED.into(),
                    })
                    .await;
                }
                self.on_text(key, text, reply_target).await;
            }
            ChannelEvent::Media {
                key,
                files,
                caption,
                reply_target,
            } => {
                let prompt = compose_media_prompt(&text_from_caption(&caption), &files);
                self.on_text(key, prompt, reply_target).await;
            }
            ChannelEvent::ButtonCb { key, action } => self.on_button(key, action).await,
            ChannelEvent::FormCb {
                key,
                value,
                form_value,
                card_ref,
            } => self.on_form_cb(key, value, form_value, card_ref).await,
        }
    }

    /// 表单容器提交回调：按负载里的 `form` 字段路由到已接线的 CRUD 表单
    /// （provider-preset / provider-custom 共两张）或「Provider 管理」主卡的
    /// 新交互（bead sebas-63f.5：provider-mode / provider-default-direct /
    /// provider-list-select / provider-set-default-direct /
    /// provider-delete-confirm / provider-create-preset / provider-create-custom）。
    /// 未接线的表单仅记日志，不静默吞掉。
    async fn on_form_cb(
        &self,
        key: ChannelKey,
        value: Value,
        form_value: BTreeMap<String, Value>,
        message_id: Option<String>,
    ) {
        tracing::debug!(?key, ?value, "form callback received");
        let form_name = value.get("form").and_then(Value::as_str).unwrap_or("");

        // Provider 管理主卡的新 form 名（select_static / 按钮的 callback
        // value 共用此判别字段）。先于既有 provider_forms 路由尝试——
        // 任何本模块接管的 form 名都不会落到既有表单上。
        if let Some(out) =
            super::provider_card::dispatch(self, &key, &value, &form_value, message_id.clone())
                .await
        {
            self.emit(out).await;
            return;
        }

        let routed = match &self.provider_forms {
            Some(forms) => {
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

    async fn on_text(&self, key: ChannelKey, text: String, reply_to: Option<String>) {
        // 记录最近入站回复目标：话题内 = 话题根消息 message_id（events 层已
        // 归一化），主线 = 触发消息 message_id。话题出站卡（权限卡/初始卡）
        // 用它作为 root_id，保证回复聚合在原话题。
        if let Some(target) = &reply_to {
            self.reply_targets.set(key.clone(), target.clone()).await;
        }
        match parse_command(&text) {
            Command::New(prompt) => {
                match self.map.begin_spawn(key.clone()).await {
                    Ok(crate::state::BeginSpawn::AlreadySpawning) => {
                        // A spawn is already in flight for this chat; a second
                        // /new would orphan the in-flight session.
                        tracing::debug!("spawn already in flight; ignoring duplicate /new");
                    }
                    // trailing text 作为初始 prompt：`derive_topic(prompt)`
                    // 派生卡片标题，引用块渲染 prompt；空 trailing 走旧行为
                    // （无 prompt、卡标题回退 "Claude Code" 占位）。
                    Ok(_) => {
                        // The placeholder replaced whatever was mapped:
                        // publish before `key` moves into spawn_new.
                        self.publish_created(&key).await;
                        self.spawn_new(key, prompt, reply_to).await;
                    }
                    Err(e) => {
                        tracing::warn!(?e, "begin_spawn failed");
                        self.emit(Out::HelpText { key }).await;
                    }
                }
            }
            Command::Help => {
                let theme = self.card_cfg.read().await.theme_color.clone();
                let card = cards_ui::help_card("session", &theme);
                self.emit(Out::SendCard {
                    key,
                    card,
                    msg_id: Some(HELP_CARD_TAG.into()),
                    perm_request_id: None,
                    perm_meta: None,
                    root_id: None,
                })
                .await;
            }
            Command::Sessions => {
                self.list_sessions(key).await;
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
            Command::Confirm(token) => {
                // 确认兑换与 /upgrade 同级：chat 级控制操作，不需要活跃会话。
                // 裸 /confirm 无 token 时回用法提示，不静默。
                if token.is_empty() {
                    self.emit(Out::PlainText {
                        key,
                        content: "用法: /confirm <token>。token 来自 /upgrade /rollback /restart 提交后的待确认回复。".into(),
                    })
                    .await;
                } else {
                    self.emit(Out::WatchdogConfirm { key, token }).await;
                }
            }
            Command::Services => {
                self.request_watchdog_services(key).await;
            }
            Command::System => {
                self.request_watchdog_system(key).await;
            }
            Command::Gateway(action) => {
                self.request_watchdog_gateway(key, action).await;
            }
            Command::Webui => {
                self.request_watchdog_webui(key).await;
            }
            Command::Provider => self.on_provider(key).await,
            Command::PassThrough(p) => {
                // 原生会话续聊（make-feishu-optional-webui-primary）：key 已是
                // agent-* 前缀 → 走桥，不经 acp 的 route_text/continue_session。
                if let Some(bridge) = self.native.read().await.clone()
                    && bridge.is_native(&key)
                {
                    bridge.prompt(key, p);
                    return;
                }
                match self.map.route_text(key.clone(), p.clone()).await {
                    Ok(crate::state::TextRoute::Continue(sid)) => {
                        // emit_turn_card publishes the new-turn phase reset.
                        self.continue_session(sid, p, reply_to, key.clone(), false)
                            .await;
                    }
                    Ok(crate::state::TextRoute::SpawnNew) => {
                        self.publish_created(&key).await;
                        self.spawn_new(key, p, reply_to).await;
                    }
                    Ok(crate::state::TextRoute::Resume(old_sid)) => {
                        // Restored mapping claimed for lazy respawn (openspec/specs/session-lifecycle/spec.md).
                        self.publish_updated(&key).await;
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
                // 原生会话同样直通桥（make-feishu-optional-webui-primary）。
                if let Some(bridge) = self.native.read().await.clone()
                    && bridge.is_native(&key)
                {
                    bridge.prompt(key, text);
                    return;
                }
                match self.map.route_text(key.clone(), text.clone()).await {
                    Ok(crate::state::TextRoute::Continue(sid)) => {
                        // emit_turn_card publishes the new-turn phase reset.
                        self.continue_session(sid, text, reply_to, key.clone(), true)
                            .await;
                    }
                    Ok(crate::state::TextRoute::SpawnNew) => {
                        self.publish_created(&key).await;
                        self.spawn_new(key, text, reply_to).await;
                    }
                    Ok(crate::state::TextRoute::Resume(old_sid)) => {
                        self.publish_updated(&key).await;
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
                    // 无会话明确报错（sebas-ixv）：HelpText 在 dispatch 层是
                    // no-op，等于静默丢弃。按错误回复约定发 PlainText。
                    self.no_session_reply(key, "/compact").await;
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
                    // 同上：无会话不静默（sebas-ixv）。
                    let cmd = text.split_whitespace().next().unwrap_or("");
                    self.no_session_reply(key, cmd).await;
                }
            }
            // `/switch` `/resume` `/cd` 已解析但路由未实现（多会话切换/指定恢复/
            // 目录切换需要 dispatcher 与 state 层支持，超出 router 单侧能力）。原落到
            // `_ => HelpText` 臂静默丢弃（sebas-ixv）；按错误回复约定发 PlainText
            // 明确告知，不再零反馈。
            Command::Switch(_) | Command::Resume(_) | Command::Cd(_) => {
                let cmd = text.split_whitespace().next().unwrap_or("");
                self.emit(Out::PlainText {
                    key,
                    content: format!(
                        "{cmd} 暂未支持（已解析但路由未接入）。可用 /new 开新会话、/sessions 查看会话。"
                    ),
                })
                .await;
            } // match 已穷尽所有 Command 变体（sebas-ixv）：无需兜底臂。
        }
    }

    /// 无活跃会话时的统一错误回复（sebas-ixv）：需要会话的命令在 key 无映射
    /// 时给用户明确的 PlainText，而不是落到 dispatch 层为 no-op 的 HelpText。
    /// 文案与 `list_sessions` 空列表口径一致。
    async fn no_session_reply(&self, key: ChannelKey, cmd: &str) {
        self.emit(Out::PlainText {
            key,
            content: format!("当前没有活跃会话，{cmd} 需要活跃会话。发送 /new 开始新会话。"),
        })
        .await;
    }

    async fn request_watchdog_upgrade(&self, key: ChannelKey, dev: bool, dry_run: bool) {
        self.emit(Out::WatchdogUpgrade { key, dev, dry_run }).await;
    }

    async fn request_watchdog_rollback(&self, key: ChannelKey) {
        self.emit(Out::WatchdogRollback { key }).await;
    }

    async fn request_watchdog_restart(&self, key: ChannelKey) {
        self.emit(Out::WatchdogRestart { key }).await;
    }

    async fn list_sessions(&self, key: ChannelKey) {
        use crate::state::MappingState;

        let all = self.map.snapshot_all().await;
        // 会话按「同一会话上下文」分组列出：同通道 + 同 chat（飞书引用里的
        // `chat\0thread` 复合以 `\0` 前的 chat 部分归组；主线/话题同属一个
        // 聊天）。跨通道会话不混列（web 会话不进飞书列表）。
        let sessions: Vec<_> = all
            .into_iter()
            .filter(|(k, _)| same_chat_context(k, &key))
            .collect();

        if sessions.is_empty() {
            self.emit(Out::PlainText {
                key,
                content: "当前聊天没有活跃会话。发送 /new 开始新会话。".into(),
            })
            .await;
            return;
        }

        let mut lines = vec!["当前会话:".to_string()];
        for (i, (sk, m)) in sessions.iter().enumerate() {
            let (sid, label) = match &m.state {
                MappingState::Active { session_id } => (session_id.as_str(), "active"),
                MappingState::Spawning { .. } => ("(spawning)", "spawning"),
                MappingState::Dormant { session_id } => (session_id.as_str(), "dormant"),
            };
            let thread = thread_label(&sk.reference);
            let ts = m.last_active_unix;
            lines.push(format!(
                "  {}. {sid} [{label}]{thread} 上次活跃={ts}",
                i + 1
            ));
        }
        self.emit(Out::PlainText {
            key,
            content: lines.join("\n"),
        })
        .await;
    }

    async fn request_watchdog_services(&self, key: ChannelKey) {
        self.emit(Out::WatchdogServices { key }).await;
    }

    async fn request_watchdog_system(&self, key: ChannelKey) {
        self.emit(Out::WatchdogSystem { key }).await;
    }

    async fn request_watchdog_gateway(&self, key: ChannelKey, action: GatewayAction) {
        self.emit(Out::WatchdogGateway { key, action }).await;
    }

    async fn request_watchdog_webui(&self, key: ChannelKey) {
        self.emit(Out::WatchdogWebui { key }).await;
    }

    async fn on_button(&self, key: ChannelKey, action: ChannelAction) {
        // 帮助卡片交互：分组 tab 切换 / 命令执行。优先于其他所有路由。
        let payload = action.value.pointer("/action/value").cloned();
        if let Some(p) = payload.as_ref() {
            // Tab 切换：原地更新卡片
            if let Some(tab) = p.get("help_tab").and_then(Value::as_str) {
                let theme = self.card_cfg.read().await.theme_color.clone();
                let card = cards_ui::help_card(tab, &theme);
                // 查找已有帮助卡 msg_id → 原地更新；没有则发新卡
                if let Some(msg_id) = self.help_card_msg_id(&key).await {
                    self.emit(Out::UpdateCardByMsgId {
                        key,
                        msg_id,
                        card,
                    })
                    .await;
                } else {
                    self.emit(Out::SendCard {
                        key,
                        card,
                        msg_id: Some(HELP_CARD_TAG.into()),
                        perm_request_id: None,
                        perm_meta: None,
                        root_id: None,
                    })
                    .await;
                }
                return;
            }
            // 命令执行：复用文本命令处理流程
            if let Some(cmd) = p.get("help_cmd").and_then(Value::as_str) {
                self.on_text(key, cmd.to_string(), None).await;
                return;
            }
        }
        // Provider 管理主卡的新按钮（mode / 设默认 / 删除 / ＋ 新增预设/自定义
        // / 探测 model 列表 / 探测 apply / 返回）。优先路由于既有 provider_forms，
        // 避免模式按钮被误投到既有 form 的 `{form, op, id}` 分发上。
        if let Some(p) = payload.as_ref()
            && p.get("form").and_then(Value::as_str).is_some_and(|f| {
                matches!(
                    f,
                    super::provider_card::FORM_MODE
                        | super::provider_card::FORM_SET_DEFAULT_DIRECT
                        | super::provider_card::FORM_DELETE_CONFIRM
                        | super::provider_card::FORM_CREATE_PRESET
                        | super::provider_card::FORM_CREATE_CUSTOM
                        | super::provider_card::FORM_PROBE
                        | super::provider_card::FORM_PROBE_APPLY
                        | super::provider_card::FORM_BACK
                )
            })
        {
            let message_id = action
                .value
                .pointer("/context/open_message_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if let Some(out) =
                super::provider_card::dispatch(self, &key, p, &BTreeMap::new(), message_id).await
            {
                self.emit(out).await;
                return;
            }
        }

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
            let card = cards_ui::dead_session_card();
            self.emit(Out::SendCard {
                key,
                card,
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
                let card = cards_ui::resolved_permission_card(label);
                self.emit(Out::UpdateCardByMsgId {
                    key: entry.key,
                    msg_id: entry.msg_id,
                    card,
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
            let card = cards_ui::expired_permission_card();
            self.emit(Out::SendCard {
                key: key.clone(),
                card,
                msg_id: None,
                perm_request_id: None,
                perm_meta: None,
                // Expired permission card is fire-and-forget.
                root_id: None,
            })
            .await;
        }
    }

    /// `/provider`：渲染 Provider 管理主卡（mode + default-direct + 列表 +
    /// 详情/新建面板，bead sebas-63f.5）。未接线时退回帮助。
    async fn on_provider(&self, key: ChannelKey) {
        if self.provider_forms.is_some() {
            let out = super::provider_card::render_main_card(self, &key).await;
            self.emit(out).await;
        } else {
            self.emit(Out::HelpText { key }).await;
        }
    }

    async fn spawn_new(&self, key: ChannelKey, prompt: String, input_msg_id: Option<String>) {
        // A fresh session must not inherit "本会话不再询问" grants from the
        // previous session in this chat — the user approved those for the
        // session that asked, not for whatever comes next.
        self.allowlist.clear(&key).await;
        // 新会话也不继承上一条入站的回复目标（话题内 root_id）。和 allowlist
        // 一样随会话终止清理，防止 ReplyTargetMap 无界增长。
        self.reply_targets.clear(&key).await;
        // 原生执行体路由（make-feishu-optional-webui-primary，design D2）：
        // 该 chat 已是原生会话（agent-* 前缀）→ 走桥；否则走 acp 桥（默认）。
        if let Some(bridge) = self.native.read().await.clone()
            && bridge.is_native(&key)
        {
            bridge.prompt(key, prompt);
            return;
        }
        // Only emit SpawnAcp. The root card is sent by the dispatcher *after*
        // `create_session` mints the real session_id, so the card's MsgIdMap
        // entry (and later streaming UpdateCards) key off that session_id.
        // `input_msg_id` rides along so the dispatcher can point the session's
        // state reactions at the user's input message.
        self.emit(Out::SpawnAcp {
            key: key.clone(),
            prompt,
            input_msg_id,
        })
        .await;
        // Spawning placeholder 已入表（route_text/begin_spawn），对外发 Created。
        self.publish_created(&key).await;
    }

    async fn continue_session(
        &self,
        session_id: String,
        prompt: String,
        root_id: Option<String>,
        key: ChannelKey,
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
            Command::Status => AcpCommand::ContinueSession {
                session_id: session_id.into(),
                prompt: "/status".into(),
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
    async fn forward_compact(&self, session_id: &str, key: ChannelKey) {
        let prompt = "⚙️ 压缩上下文...";
        self.card_states
            .seed(session_id.to_string(), prompt.into())
            .await;

        // Send initial card
        let theme_color = self.card_cfg.read().await.theme_color.clone();
        let card = ChannelCard {
            title: String::new(),
            theme: theme_color.clone(),
            elements: Vec::new(),
            turn: Some(sebas_channels::card::TurnChrome {
                prompt: prompt.to_string(),
                session_id: session_id.to_string(),
                usage: None,
            }),
        };
        self.emit(Out::SendCard {
            key: key.clone(),
            card,
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
                        e == crate::card_state::phase::DONE || e == crate::card_state::phase::FAILED
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
                        st.body = vec![crate::card_events::compact_progress_note(secs)];
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
        key: ChannelKey,
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
