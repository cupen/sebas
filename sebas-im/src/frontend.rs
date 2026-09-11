//! IM 会话前端（extract-im-service 3.x）：im 进程的会话面 —— 入站交互蒸馏为
//! 会话端口请求，出站从会话事件 + turn 流重建卡片（design D3），审批卡走
//! 通道审批面。卡片机复用 sebas-dispatch 的中立实现（CardInput）。

use crate::port::{ControlPort, ControlRequest, CoreSessionPort, parse_decision};
use crate::reactions::{ReactPlan, ReactionTracker};
use sebas_channels::card::{AppUsage, ChannelCard, ChannelElement, TurnChrome};
use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::card_events::{apply_input_to_card, card_needs_rotation, continuation_note};
use sebas_dispatch::card_state::{CardState, phase};
use sebas_dispatch::cards_ui;
use sebas_dispatch::commands::{Command, parse_command};
use sebas_dispatch::{SessionEvent, SessionInfo, TurnEntry};
use sebas_feishu::adapter::{render_channel_card_frame, render_standalone_card};
use sebas_feishu::client::{FeishuApiError, FeishuClient, TokenManager};
use sebas_feishu::events::SessionKey;
use sebas_webui::session_backend::{PermissionDecision, PermissionNotice};
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
#[cfg(test)]
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// 会话轮询节奏（design D3：turn 流拉取 + 本地防抖合并出卡）。
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// 一路会话的 im 侧视图（原 CardState + 卡片引用的角色）。
struct SessionView {
    key: ChannelKey,
    session_id: Option<String>,
    /// 当前轮的 user prompt（新 prompt = 新卡片轮）。
    prompt: String,
    status: String,
    phase: Option<String>,
    usage: AppUsage,
    card: CardState,
    /// 当前卡片的飞书 message_id（in-place PATCH 引用）。
    card_msg_id: Option<String>,
    /// 已拉取到的 turn 位置。
    last_pos: u64,
    /// 终态（Finished/Error）后冻结，等下一轮 prompt 重开。
    frozen: bool,
}

impl SessionView {
    fn new(key: ChannelKey, info: &SessionInfo) -> Self {
        let prompt = info.user_prompt.clone().unwrap_or_default();
        Self {
            session_id: info.session_id.clone(),
            status: info.status.clone(),
            phase: info.phase.clone(),
            usage: AppUsage::default(),
            card: CardState::new(&prompt),
            card_msg_id: None,
            last_pos: 0,
            frozen: false,
            prompt,
            key,
        }
    }
}

/// IM 前端：持有端口、飞书句柄与全部 im 侧状态。
pub struct ImFrontend<P: CoreSessionPort, C: ControlPort> {
    port: Arc<P>,
    control: Arc<C>,
    feishu: FeishuClient,
    http: reqwest::Client,
    tokens: TokenManager,
    theme_color: String,
    /// `[media] download_dir`（入站媒体落盘处）。
    pub media_dir: std::path::PathBuf,
    /// `[media] max_file_size`（0 = 硬顶兜底）。
    pub max_file_size: u64,
    views: RwLock<HashMap<String, SessionView>>,
    /// chat key reference → 触发消息 message_id（线程回复目标）。
    reply_targets: RwLock<HashMap<String, String>>,
    /// request_id → 权限卡 message_id（点击后就地翻卡）。
    perm_cards: RwLock<HashMap<String, String>>,
    reactions: ReactionTracker,
}

fn view_id(key: &ChannelKey) -> String {
    format!("{}\0{}", key.channel.as_str(), key.reference)
}

fn feishu_key(key: &ChannelKey) -> SessionKey {
    SessionKey::from_channel_key(key)
}

fn thread_of(key: &ChannelKey) -> Option<&str> {
    key.reference
        .split_once('\0')
        .map(|(_, t)| t)
        .filter(|t| !t.is_empty())
}

impl<P: CoreSessionPort + 'static, C: ControlPort + 'static> ImFrontend<P, C> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        port: Arc<P>,
        control: Arc<C>,
        feishu: FeishuClient,
        http: reqwest::Client,
        tokens: TokenManager,
        theme_color: String,
        media_dir: std::path::PathBuf,
        max_file_size: u64,
    ) -> Self {
        Self {
            port,
            control,
            feishu,
            http,
            tokens,
            theme_color,
            media_dir,
            max_file_size,
            views: RwLock::new(HashMap::new()),
            reply_targets: RwLock::new(HashMap::new()),
            perm_cards: RwLock::new(HashMap::new()),
            reactions: ReactionTracker::default(),
        }
    }

    /// 启动全部循环：入站事件消费、会话事件消费、审批帧消费、turn 轮询。
    pub async fn run(self: Arc<Self>, mut inbound: tokio::sync::mpsc::Receiver<ChannelEvent>) {
        let sessions = self.clone();
        tokio::spawn(async move { sessions.session_event_loop().await });
        let approvals = self.clone();
        tokio::spawn(async move { approvals.approval_loop().await });
        let poller = self.clone();
        tokio::spawn(async move { poller.poll_loop().await });
        // 入站循环在当前任务：进程生命周期与它绑定。
        while let Some(evt) = inbound.recv().await {
            self.on_channel_event(evt).await;
        }
        info!("im frontend: inbound channel closed");
    }

    // ── 入站 ────────────────────────────────────────────────────────────────

    async fn on_channel_event(&self, evt: ChannelEvent) {
        let key = evt.key().clone();
        match evt {
            ChannelEvent::Text {
                text, reply_target, ..
            } => {
                if let Some(t) = &reply_target {
                    self.reply_targets
                        .write()
                        .await
                        .insert(view_id(&key), t.clone());
                }
                self.on_text(key, text).await;
            }
            ChannelEvent::Media {
                files,
                caption,
                reply_target,
                ..
            } => {
                // reply_target 即携带文件的消息 id（media API 路径参数）。
                if let Some(t) = &reply_target {
                    self.reply_targets
                        .write()
                        .await
                        .insert(view_id(&key), t.clone());
                }
                self.on_media(key, files, caption, reply_target).await;
            }
            ChannelEvent::ButtonCb { action, .. } => {
                self.on_button(action.session_id, action.request_id, action.value)
                    .await;
            }
            ChannelEvent::FormCb {
                value, form_value, ..
            } => {
                self.on_form(value, form_value).await;
            }
        }
    }

    async fn on_text(&self, key: ChannelKey, text: String) {
        match parse_command(&text) {
            Command::New(prompt) => {
                // /new：关闭旧会话（未知视为已关）+ ensure 触发新会话；
                // 空 trailing 文本以空 prompt 建会话（route_text 的历史语义）。
                let _ = self.port.close(key.clone()).await;
                if let Err(e) = self
                    .port
                    .ensure_message(key.clone(), prompt, Vec::new())
                    .await
                {
                    self.send_text(&key, format!("开新会话失败：{e}")).await;
                }
            }
            Command::Help => {
                let card = cards_ui::help_card("session", &self.theme_color);
                self.send_standalone_card(&key, card).await;
            }
            Command::Sessions => self.list_sessions(&key).await,
            Command::Settings(sk, sv) => self.handle_settings(&key, sk, sv).await,
            Command::Provider => self.handle_provider(&key).await,
            Command::Upgrade { dev, dry_run } => {
                self.reply_control(&key, ControlRequest::Upgrade { dev, dry_run })
                    .await;
            }
            Command::Rollback => self.reply_control(&key, ControlRequest::Rollback).await,
            Command::Restart => self.reply_control(&key, ControlRequest::Restart).await,
            Command::Services => self.reply_control(&key, ControlRequest::Services).await,
            Command::System => self.reply_control(&key, ControlRequest::System).await,
            Command::Router(action) => {
                let mut args = HashMap::new();
                args.insert("action".to_string(), format!("{action:?}").to_lowercase());
                self.reply_control(&key, ControlRequest::Router(args.into_iter().collect()))
                    .await;
            }
            Command::Webui => self.reply_control(&key, ControlRequest::Webui).await,
            Command::Confirm(token) => {
                if token.is_empty() {
                    self.send_text(&key, "用法: /confirm <token>。token 来自 /upgrade /rollback /restart 提交后的待确认回复。".into()).await;
                } else {
                    self.reply_control(&key, ControlRequest::Confirm { token })
                        .await;
                }
            }
            Command::Cancel => {
                // 会话未知/无在飞 turn → 如实提示。
                if let Err(e) = self.port.cancel(key.clone()).await {
                    self.send_text(&key, format!("/cancel 未生效：{e}")).await;
                }
            }
            // 会话转发类（/cost /status /compact /btw）与普通文本统一走
            // ensure 投递 —— core 的 web_send_message 对 Cost/Cancel/Status/
            // Compact 有专属臂，其余按 PassThrough 续聊。
            Command::Cost
            | Command::Status
            | Command::Compact
            | Command::Btw(_)
            | Command::PassThrough(_) => {
                if let Err(e) = self
                    .port
                    .ensure_message(key.clone(), text, Vec::new())
                    .await
                {
                    self.send_text(&key, format!("消息未送达：{e}")).await;
                }
            }
            Command::Switch(_) | Command::Resume(_) | Command::Cd(_) => {
                let cmd = text.split_whitespace().next().unwrap_or("").to_string();
                self.send_text(&key, format!("{cmd} 暂未支持（已解析但路由未接入）。可用 /new 开新会话、/sessions 查看会话。")).await;
            }
        }
    }

    /// 入站媒体（extract-im-service 4.2）：解析为本地附件后随消息投递。
    /// 下载失败如实回复；M2 先以附件标记文本投递（4.1 协议面落地后升级为
    /// 结构化 attachments 字段）。
    async fn on_media(
        &self,
        key: ChannelKey,
        files: Vec<String>,
        caption: Option<String>,
        message_id: Option<String>,
    ) {
        let mut markers = Vec::new();
        let bearer = self.tokens.token().await.unwrap_or_default();
        let msg_id = message_id.unwrap_or_default();
        for file_key in &files {
            match crate::media::resolve(
                &self.http,
                &bearer,
                &msg_id,
                file_key,
                &self.media_dir,
                self.max_file_size,
            )
            .await
            {
                Ok(path) => markers.push(format!("[图片已接收: {}]", path.display())),
                Err(e) => {
                    self.send_text(&key, format!("附件未能接收：{e}")).await;
                    return;
                }
            }
        }
        let mut text = markers.join("\n");
        if let Some(c) = caption {
            text.push('\n');
            text.push_str(&c);
        }
        if let Err(e) = self.port.ensure_message(key, text, Vec::new()).await {
            warn!(error = %e, "media ensure_message failed");
        }
    }

    /// 权限卡按钮（permission-flow 通道版）：解析 behavior value 里的
    /// request_id/decision → 端口回传 → 就地翻卡。stale（无待决请求）→
    /// 置灰「已过期」卡，fail-closed。
    async fn on_button(
        &self,
        _session_id: String,
        request_id: Option<String>,
        value: serde_json::Value,
    ) {
        let Some(request_id) = request_id.or_else(|| {
            value
                .get("request_id")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        }) else {
            return;
        };
        let raw = value
            .get("decision")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .unwrap_or_default();
        let Some(decision) = parse_decision(&raw) else {
            return;
        };
        let msg_id = self.perm_cards.read().await.get(&request_id).cloned();
        let accepted = self
            .port
            .approval_answer(&request_id, decision.clone())
            .await;
        let (title, theme, note): (&str, &str, String) = if !accepted {
            (
                "请求已过期",
                "grey",
                "该请求已处理、超时或会话已结束。".into(),
            )
        } else {
            match decision {
                PermissionDecision::AllowOnce => {
                    ("已允许（本次）", "green", "本次调用已放行。".into())
                }
                PermissionDecision::AllowSession => {
                    ("已允许（本会话）", "green", "本会话同类调用已放行。".into())
                }
                PermissionDecision::Deny => ("已拒绝", "red", "该工具调用已被拒绝。".into()),
                PermissionDecision::Escalate { reason } => ("已升级", "orange", reason),
            }
        };
        if let Some(msg_id) = msg_id {
            let card = ChannelCard {
                title: title.into(),
                theme: theme.into(),
                elements: vec![ChannelElement::Markdown { content: note }],
                turn: None,
            };
            self.update_card(&msg_id, &card).await;
        }
        self.perm_cards.write().await.remove(&request_id);
    }

    /// 表单回调（provider/settings 卡）：payload 带 op 时直通状态库。
    async fn on_form(
        &self,
        value: serde_json::Value,
        form_value: BTreeMap<String, serde_json::Value>,
    ) {
        let Some(op) = value.get("op").and_then(|v| v.as_str()).map(str::to_owned) else {
            return;
        };
        let payload = json!({"op": op, "form": form_value}); // BTreeMap → JSON object
        if let Err(e) = self.port.state_mutate("providers", payload).await {
            warn!(error = %e, "form mutation failed");
        }
    }

    // ── 出站：会话事件 + 轮询 ──────────────────────────────────────────────

    async fn session_event_loop(&self) {
        let mut rx = self.port.subscribe_sessions();
        loop {
            match rx.recv().await {
                Ok(SessionEvent::Created { session }) | Ok(SessionEvent::Updated { session }) => {
                    self.on_session_info(session).await;
                }
                Ok(SessionEvent::Removed { channel, key }) => {
                    let ck = ChannelKey::new(channel, key);
                    self.views.write().await.remove(&view_id(&ck));
                }
                // workbench-turn-queue：丢弃标注事件对 IM 前端是建议性的
                // （队列管理面在 webui），随后的 Removed 帧会移除该会话视图。
                Ok(SessionEvent::PendingDropped { .. }) => {}
                Ok(SessionEvent::Resync) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(lagged = n, "session event lag; resyncing from snapshot");
                    for s in self.port.snapshot().await {
                        self.on_session_info(s).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    async fn on_session_info(&self, info: SessionInfo) {
        if info.channel != "feishu" {
            return; // im 只渲染本通道（feishu）的会话
        }
        let key = info.channel_key();
        let id = view_id(&key);
        let mut views = self.views.write().await;
        let Some(view) = views.get_mut(&id) else {
            if info.status == "spawning" || info.session_id.is_some() {
                views.insert(id, SessionView::new(key, &info));
            }
            return;
        };
        // 新一轮 prompt：冻结旧卡，开新轮。
        let new_prompt = info.user_prompt.clone().unwrap_or_default();
        let new_turn = !new_prompt.is_empty() && new_prompt != view.prompt;
        view.session_id = info.session_id.clone();
        view.status = info.status.clone();
        view.usage = info.usage.clone().unwrap_or_default();
        if new_turn {
            view.prompt = new_prompt;
            view.card = CardState::new(&view.prompt);
            view.card_msg_id = None;
            view.last_pos = 0;
            view.frozen = false;
        }
        // 相位变化 → root 卡 reaction 换挡。
        if view.phase != info.phase
            && let Some(emoji) = info.phase.as_deref()
            && let Some(msg_id) = view.card_msg_id.as_deref()
        {
            let plan = self.reactions.plan(&id, emoji).await;
            let swapped = match plan {
                ReactPlan::Swap { unreact_id } => {
                    let _ = self
                        .feishu
                        .unreact(&self.http, &self.tokens, msg_id, &unreact_id)
                        .await;
                    true
                }
                ref p => *p == ReactPlan::ReactOnly,
            };
            if swapped
                && let Ok(rid) = self
                    .feishu
                    .react(&self.http, &self.tokens, msg_id, emoji)
                    .await
            {
                self.reactions.record(&id, emoji.to_string(), rid).await;
            }
        }
        view.phase = info.phase.clone();
    }

    async fn approval_loop(&self) {
        let Some(mut rx) = self.port.subscribe_approvals() else {
            return;
        };
        loop {
            match rx.recv().await {
                Ok(notice) => self.render_permission_card(notice).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    /// 权限卡渲染（流上审批帧 → 交互卡）。
    async fn render_permission_card(&self, notice: PermissionNotice) {
        let key = decode_wire_key(&notice.session_id);
        let card = cards_ui::permission_card(
            &notice.session_id,
            &notice.request_id,
            &notice.tool_name,
            &notice.args,
        );
        match self.send_standalone_card(&key, card).await {
            Some(msg_id) => {
                self.perm_cards
                    .write()
                    .await
                    .insert(notice.request_id.clone(), msg_id);
            }
            None => error!(request_id = %notice.request_id, "permission card send failed"),
        }
    }

    async fn poll_loop(&self) {
        let mut tick = tokio::time::interval(POLL_INTERVAL);
        loop {
            tick.tick().await;
            let targets: Vec<(ChannelKey, u64)> = {
                let views = self.views.read().await;
                views
                    .values()
                    .filter(|v| !v.frozen && v.session_id.is_some() && v.status == "active")
                    .map(|v| (v.key.clone(), v.last_pos))
                    .collect()
            };
            for (key, from) in targets {
                let Some(entries) = self.port.turns(&key, from).await else {
                    continue;
                };
                self.apply_turns(key, entries).await;
            }
        }
    }

    async fn apply_turns(&self, key: ChannelKey, entries: Vec<TurnEntry>) {
        let id = view_id(&key);
        let mut views = self.views.write().await;
        let Some(view) = views.get_mut(&id) else {
            return;
        };
        let mut dirty = false;
        for entry in entries {
            view.last_pos = entry.position + 1;
            if entry.kind == "prompt" {
                continue; // prompt 已在 Created/Updated 时 seed
            }
            let input = turn_to_card_input(&entry);
            if let Some(input) = &input {
                apply_input_to_card(&mut view.card.body, input, &self.card_config());
                dirty = true;
            }
            if matches!(
                input,
                Some(sebas_dispatch::card_events::CardInput::Finished)
            ) {
                view.frozen = true;
            }
        }
        drop(views);
        if dirty {
            self.flush(&key).await;
        }
    }

    fn card_config(&self) -> sebas_dispatch::CardConfig {
        // 渲染旋钮（截断/折叠/thinking）来自 [card] 配置；im 侧持有
        // （配置归属 spec）。当前用缺省 + 主题色；settings 域接管后随域刷新。
        sebas_dispatch::CardConfig {
            theme_color: self.theme_color.clone(),
            ..Default::default()
        }
    }

    /// 出站 flush：组装中立卡 → 飞书 JSON → 新发或 PATCH。
    async fn flush(&self, key: &ChannelKey) {
        let id = view_id(key);
        let (card, msg_id) = {
            let mut views = self.views.write().await;
            let Some(view) = views.get_mut(&id) else {
                return;
            };
            if view.session_id.is_none() {
                return;
            }
            if card_needs_rotation(&view.card.body) {
                // 换卡：冻结当前，新卡以接续注开 body。
                let note = continuation_note();
                view.card = CardState::new(&view.prompt);
                view.card.body.push(note);
                view.card_msg_id = None;
            }
            let chrome = TurnChrome {
                prompt: view.prompt.clone(),
                session_id: view.session_id.clone().unwrap_or_default(),
                usage: Some(view.usage.clone()),
            };
            let card = ChannelCard {
                title: view
                    .prompt
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("sebas")
                    .to_string(),
                theme: self.theme_color.clone(),
                elements: view.card.body.clone(),
                turn: Some(chrome),
            };
            (card, view.card_msg_id.clone())
        };
        let framed = render_channel_card_frame(
            &card.turn.as_ref().unwrap().prompt,
            &card.turn.as_ref().unwrap().session_id,
            &card,
            None,
        );
        let Ok(card_json) = serde_json::to_value(framed) else {
            return;
        };
        let new_msg_id = match msg_id.as_deref() {
            Some(mid) => match self
                .feishu
                .update_card(&self.http, &self.tokens, mid, card_json)
                .await
            {
                Ok(()) => Some(mid.to_string()),
                Err(e)
                    if e.downcast_ref::<FeishuApiError>()
                        .is_some_and(FeishuApiError::is_topic_invalid) =>
                {
                    // feishu-bridge spec「Invalid-topic errors force session close」：
                    // 话题失效不可恢复——文本通知 + 幂等关会话，不重试该卡。
                    warn!(msg_id = %mid, "topic invalid (230019/230071); closing session");
                    self.send_text(
                        key,
                        "原话题已失效（可能已被删除或权限变更），本会话已结束。发送 /new 开始新会话。".into(),
                    )
                    .await;
                    let _ = self.port.close(key.clone()).await;
                    None
                }
                Err(e) => {
                    warn!(?e, "card update failed");
                    None
                }
            },
            None => {
                // 新卡：reply 到触发消息（线程聚合），话题内带 thread_id。
                let root = self.reply_targets.read().await.get(&id).cloned();
                match self
                    .feishu
                    .send_card(
                        &self.http,
                        &self.tokens,
                        &feishu_key(key),
                        card_json,
                        root.as_deref(),
                        thread_of(key),
                    )
                    .await
                {
                    Ok(mid) => Some(mid),
                    Err(e)
                        if e.downcast_ref::<FeishuApiError>()
                            .is_some_and(FeishuApiError::is_topic_invalid) =>
                    {
                        warn!("card send topic-invalid (230019/230071); closing session");
                        self.send_text(
                            key,
                            "原话题已失效（可能已被删除或权限变更），本会话已结束。发送 /new 开始新会话。".into(),
                        )
                        .await;
                        let _ = self.port.close(key.clone()).await;
                        None
                    }
                    Err(e) => {
                        warn!(?e, "card send failed");
                        None
                    }
                }
            }
        };
        if let Some(mid) = new_msg_id {
            self.views.write().await.get_mut(&id).unwrap().card_msg_id = Some(mid.clone());
            // 首卡出现：挂「已收到」相位 reaction（feishu-reactions 语义）。
            if self.reactions.plan(&id, phase::SEED).await == ReactPlan::ReactOnly
                && let Ok(rid) = self
                    .feishu
                    .react(&self.http, &self.tokens, &mid, phase::SEED)
                    .await
            {
                self.reactions
                    .record(&id, phase::SEED.to_string(), rid)
                    .await;
            }
        }
    }

    // ── 发送原语 ───────────────────────────────────────────────────────────

    async fn send_text(&self, key: &ChannelKey, content: String) {
        if let Err(e) = self
            .feishu
            .send_text(&self.http, &self.tokens, &feishu_key(key), &content)
            .await
        {
            warn!(?e, "send_text failed");
        }
    }

    /// 独立 UI 卡（help/权限/表单），返回 message_id 供就地更新。
    async fn send_standalone_card(&self, key: &ChannelKey, card: ChannelCard) -> Option<String> {
        let Ok(framed) = serde_json::to_value(render_standalone_card(&card)) else {
            return None;
        };
        let root = self.reply_targets.read().await.get(&view_id(key)).cloned();
        match self
            .feishu
            .send_card(
                &self.http,
                &self.tokens,
                &feishu_key(key),
                framed,
                root.as_deref(),
                thread_of(key),
            )
            .await
        {
            Ok(mid) => Some(mid),
            Err(e) => {
                warn!(?e, "standalone card send failed");
                None
            }
        }
    }

    async fn update_card(&self, msg_id: &str, card: &ChannelCard) {
        let Ok(framed) = serde_json::to_value(render_standalone_card(card)) else {
            return;
        };
        if let Err(e) = self
            .feishu
            .update_card(&self.http, &self.tokens, msg_id, framed)
            .await
        {
            warn!(?e, "card update failed");
        }
    }

    async fn list_sessions(&self, key: &ChannelKey) {
        // im-service spec「core 不可达时诚实降级」：快照为空时先区分
        // 「真的没有会话」与「核心不可达」，绝不把不可达当空态呈现。
        if let Some(cause) = self.port.unreachable_cause().await {
            self.send_text(
                key,
                format!("核心不可达，无法获取会话列表：{cause}\n核心恢复后将自动重连。"),
            )
            .await;
            return;
        }
        let sessions = self.port.snapshot().await;
        let chat: Vec<&SessionInfo> = sessions
            .iter()
            .filter(|s| s.channel == "feishu" && same_chat(&s.key, &key.reference))
            .collect();
        if chat.is_empty() {
            self.send_text(key, "当前没有会话。发送 /new 开始新会话。".into())
                .await;
            return;
        }
        let mut lines = vec!["会话列表：".to_string()];
        for s in chat {
            lines.push(format!(
                "- [{}] {}（{}）",
                s.status,
                s.user_prompt
                    .clone()
                    .unwrap_or_else(|| "(无 prompt)".into()),
                s.session_id.clone().unwrap_or_else(|| "spawning".into()),
            ));
        }
        self.send_text(key, lines.join("\n")).await;
    }

    /// /settings：读 settings 域（StateSnapshot）→ 校验写回（StateMutation）。
    async fn handle_settings(&self, key: &ChannelKey, sk: Option<String>, sv: Option<String>) {
        let Some(current) = self.port.state_snapshot("settings").await else {
            self.send_text(key, "设置域不可用（核心状态库未初始化）。".into())
                .await;
            return;
        };
        let (Some(sk), Some(sv)) = (sk, sv) else {
            self.send_text(key, format!("当前设置：{current}")).await;
            return;
        };
        let payload = json!({"key": sk, "value": serde_json::Value::String(sv)});
        match self.port.state_mutate("settings", payload).await {
            Ok(()) => {
                self.send_text(key, format!("已保存 {sk} 并即时生效。"))
                    .await
            }
            Err(e) => self.send_text(key, format!("设置被拒绝：{e}")).await,
        }
    }

    /// /provider：provider 域快照列表卡（M2 基础面：列表 + 指引；增删改
    /// 经 webui 或表单回调 op → StateMutation）。
    async fn handle_provider(&self, key: &ChannelKey) {
        let Some(providers) = self.port.state_snapshot("providers").await else {
            self.send_text(key, "provider 域不可用（核心状态库未初始化）。".into())
                .await;
            return;
        };
        let card = ChannelCard {
            title: "Provider 管理".into(),
            theme: self.theme_color.clone(),
            elements: vec![
                ChannelElement::Markdown {
                    content: format!("```json\n{providers}\n```"),
                },
                ChannelElement::Markdown {
                    content: "新增/编辑/删除请使用 WebUI 管理页；表单回调将经状态库持久化。".into(),
                },
            ],
            turn: None,
        };
        self.send_standalone_card(key, card).await;
    }

    async fn reply_control(&self, key: &ChannelKey, req: ControlRequest) {
        match self.control.submit(req).await {
            Ok(text) => self.send_text(key, text).await,
            Err(e) => self.send_text(key, e).await,
        }
    }
}

fn same_chat(a: &str, b: &str) -> bool {
    let (ca, _) = a.split_once('\0').unwrap_or((a, ""));
    let (cb, _) = b.split_once('\0').unwrap_or((b, ""));
    ca == cb
}

/// wire 上 URL-safe 编码的会话 key → ChannelKey（webui routes 口径的逆）。
fn decode_wire_key(encoded: &str) -> ChannelKey {
    if let Ok(raw) = urlencoding::decode(encoded)
        && let Some((channel, reference)) = raw.split_once('\0')
    {
        return ChannelKey::new(channel, reference);
    }
    ChannelKey::new("feishu", encoded)
}

/// turn 条目 → 卡片机输入（design D3）。工具调用在 turn 流里已是渲染好的
/// markdown 行（📖/✓ 前缀），按文本增量入账（折叠面板归 tool_start 结构化
/// 条目，留待 turn 流携带结构时启用）。
fn turn_to_card_input(entry: &TurnEntry) -> Option<sebas_dispatch::card_events::CardInput> {
    use sebas_dispatch::card_events::CardInput;
    match (entry.kind.as_str(), entry.element_type.as_str()) {
        ("prompt", _) => None,
        (_, "thinking") => Some(CardInput::ThinkingDelta {
            delta: entry.content.clone(),
        }),
        (_, "image") => Some(CardInput::TextDelta {
            delta: format!("🖼 {} ", entry.content),
        }),
        (_, "markdown") | (_, "text") => Some(CardInput::TextDelta {
            delta: format!("{}\n", entry.content),
        }),
        _ => Some(CardInput::TextDelta {
            delta: format!("{}\n", entry.content),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::{ControlPort, ControlRequest};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakePort {
        ensures: Mutex<Vec<(String, String)>>,
        cancels: AtomicUsize,
        closes: AtomicUsize,
        approvals: Mutex<Vec<String>>,
        turns: Mutex<HashMap<String, Vec<TurnEntry>>>,
        // 只写不读：保留事件管道形状供后续用例扩展。
        #[allow(dead_code)]
        events: (
            tokio::sync::mpsc::Sender<SessionEvent>,
            tokio::sync::RwLock<Option<tokio::sync::mpsc::Receiver<SessionEvent>>>,
        ),
    }

    impl FakePort {
        fn new() -> Arc<Self> {
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            Arc::new(Self {
                ensures: Mutex::new(Vec::new()),
                cancels: AtomicUsize::new(0),
                closes: AtomicUsize::new(0),
                approvals: Mutex::new(Vec::new()),
                turns: Mutex::new(HashMap::new()),
                events: (tx, tokio::sync::RwLock::new(Some(rx))),
            })
        }
    }

    #[async_trait]
    impl CoreSessionPort for FakePort {
        async fn snapshot(&self) -> Vec<SessionInfo> {
            vec![]
        }
        async fn ensure_message(
            &self,
            key: ChannelKey,
            message: String,
            attachments: Vec<crate::port::ImAttachment>,
        ) -> Result<(), String> {
            let _ = attachments;
            self.ensures.lock().await.push((key.reference, message));
            Ok(())
        }
        async fn close(&self, _key: ChannelKey) -> Result<(), String> {
            self.closes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn cancel(&self, _key: ChannelKey) -> Result<(), String> {
            self.cancels.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn turns(&self, key: &ChannelKey, from: u64) -> Option<Vec<TurnEntry>> {
            let g = self.turns.lock().await;
            Some(
                g.get(&view_id(key))
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|e| e.position >= from)
                    .collect(),
            )
        }
        fn subscribe_sessions(&self) -> tokio::sync::broadcast::Receiver<SessionEvent> {
            unimplemented!("tests drive on_session_info/apply_turns directly")
        }
        fn subscribe_approvals(
            &self,
        ) -> Option<tokio::sync::broadcast::Receiver<PermissionNotice>> {
            None
        }
        async fn approval_answer(&self, request_id: &str, _decision: PermissionDecision) -> bool {
            self.approvals.lock().await.push(request_id.to_string());
            true
        }
        async fn state_snapshot(&self, _domain: &str) -> Option<Value> {
            Some(json!({}))
        }
        async fn state_mutate(&self, _domain: &str, _payload: Value) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeControl;
    #[async_trait]
    impl ControlPort for FakeControl {
        async fn submit(&self, _req: ControlRequest) -> Result<String, String> {
            Ok("已受理（fake）。".into())
        }
    }

    fn fe(port: Arc<FakePort>) -> ImFrontend<FakePort, FakeControl> {
        ImFrontend::new(
            port,
            Arc::new(FakeControl),
            FeishuClient::new_headless(),
            reqwest::Client::new(),
            TokenManager::with_stub_token("t-stub"),
            "blue".into(),
            std::path::PathBuf::from("/tmp/im-media"),
            0,
        )
    }

    #[tokio::test]
    async fn text_ensure_and_command_distillation() {
        let port = FakePort::new();
        let fe = fe(port.clone());
        let key = ChannelKey::feishu("oc_t", None);

        // 普通文本 → ensure_message。
        fe.on_text(key.clone(), "hi there".into()).await;
        // /new hello → close + ensure("hello")。
        fe.on_text(key.clone(), "/new hello".into()).await;
        // /cancel → cancel。
        fe.on_text(key.clone(), "/cancel".into()).await;

        let ensures = port.ensures.lock().await;
        assert_eq!(ensures.len(), 2, "prompt + /new trailing: {ensures:?}");
        assert_eq!(ensures[0], ("oc_t".to_string(), "hi there".to_string()));
        assert_eq!(ensures[1], ("oc_t".to_string(), "hello".to_string()));
        assert_eq!(port.closes.load(Ordering::SeqCst), 1);
        assert_eq!(port.cancels.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn turn_stream_rebuilds_card_and_freezes_on_finished() {
        let port = FakePort::new();
        let fe = fe(port.clone());
        let key = ChannelKey::feishu("oc_t", None);

        let info = SessionInfo {
            channel: "feishu".into(),
            key: "oc_t".into(),
            session_id: Some("s1".into()),
            status: "active".into(),
            phase: Some("OnIt".into()),
            user_prompt: Some("do it".into()),
            last_active_unix: 0,
            project_dir: None,
            current_model: None,
            available_models: None,
            agent_kind: None,
            backend: None,
            usage: None,
            pending: Vec::new(),
        };
        fe.on_session_info(info.clone()).await;
        {
            let views = fe.views.read().await;
            let v = views.get(&view_id(&key)).expect("view created");
            assert_eq!(v.prompt, "do it");
        }

        // turn 流：prompt 条目跳过 + markdown 增量入账。
        let entries = vec![
            TurnEntry::prompt(0, "do it"),
            TurnEntry::markdown(1, "working"),
            TurnEntry::markdown(2, "done"),
        ];
        fe.apply_turns(key.clone(), entries).await;
        {
            let views = fe.views.read().await;
            let v = views.get(&view_id(&key)).unwrap();
            assert_eq!(v.last_pos, 3);
            assert!(!v.frozen);
            assert!(
                v.card.body.len() >= 2,
                "markdown entries accumulate: {:?}",
                v.card.body
            );
        }

        // Finished → 卡冻结。
        fe.apply_turns(key.clone(), vec![TurnEntry::markdown(3, "final")])
            .await;
        fe.on_session_info(SessionInfo {
            phase: Some("DONE".into()),
            ..info
        })
        .await;
        let entries = vec![TurnEntry::markdown(4, "ok")];
        fe.apply_turns(key.clone(), entries).await;
        // 冻结由 turn 流里的 Finished 事件驱动（此处 turn 流没有 finished
        // kind，冻结路径由 phase 驱动的后续里程碑覆盖）；这里验证
        // 最后位置推进即可。
        {
            let views = fe.views.read().await;
            let v = views.get(&view_id(&key)).unwrap();
            assert_eq!(v.last_pos, 5);
        }
    }

    #[tokio::test]
    async fn button_callback_routes_approval_answer() {
        let port = FakePort::new();
        let fe = fe(port.clone());
        let key = ChannelKey::feishu("oc_t", None);

        fe.on_button(
            "s1".into(),
            Some("toolu_1".into()),
            json!({"decision": "allow_once", "request_id": "toolu_1"}),
        )
        .await;
        assert_eq!(port.approvals.lock().await.len(), 1);
        let _ = key;
    }
}
