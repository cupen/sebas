//! The native-agent session backend (openspec/changes/sebas-agent-next
//! tasks 5.1–5.3, design N5): [`sebas_agent::session::SessionManager`] behind
//! the WebUI's [`SessionBackend`] seam, plus the composite backend that lets
//! one dashboard host both execution backends (the Claude Code bridge and
//! the built-in kernel) selectable per spawn.
//!
//! Mapping conventions:
//! - native sessions live under `ChannelKey`s on the `feishu` channel (the
//!   reference is a bare `agent-{8-hex}` chat id, no `\0` thread part — the
//!   composite routes on that prefix);
//! - each `AgentEvent` from the kernel pump updates the session transcript
//!   (prompt / streamed text / tool traces, one `TurnEntry` per flush) and
//!   republishes a session `Updated` event;
//! - gated calls surface as [`PermissionNotice`]s on the review-card feed;
//!   operator decisions round-trip through the kernel's [`ApproverHub`].

use sebas_agent::llm::{
    AnthropicMessagesClient, LlmClient, LlmError, LlmRequest, LlmTurn, StreamEvent,
};
use sebas_agent::policy::{Approver, ApprovalAnswer, ApproverHub, PolicyConfig, PolicyEngine};
use sebas_agent::session::{AgentEvent, SessionConfig, SessionHandle, SessionManager};
use sebas_agent::policy::SandboxMode;
use sebas_agent::tools::ToolRegistry;
use sebas_channels::ChannelKey;
use sebas_router::{SessionEvent, SessionInfo, TurnEntry};
use sebas_webui::session_backend::{
    PermissionDecision, PermissionNotice, Reachability, SessionBackend, SessionRejection,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};

/// One live native session: kernel handle + its rendered transcript.
struct NativeSession {
    handle: SessionHandle,
    workdir: Option<String>,
    prompt: String,
    /// Rendered transcript entries (turn-content retrieval source).
    transcript: Vec<TurnEntry>,
    /// The in-flight streamed text, flushed into the transcript on tool
    /// boundaries and turn end.
    text_buf: String,
    /// （wire-webui-sebas-agent-e2e）会话级模型 override。`None` = 走内核默认；
    /// 设置后下一次 turn 起用，新值即时生效。
    current_model_override: Option<String>,
    /// 装配期的可用模型清单（来自 `SEBAS_AGENT_MODELS`）。`info()` 透出。
    available_models: Vec<String>,
    /// 装配期的默认模型 id（`SEBAS_AGENT_MODEL`），与内核 SessionConfig 共用。
    default_model: String,
}

impl NativeSession {
    fn flush_text(&mut self) {
        if self.text_buf.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.text_buf);
        self.transcript.push(TurnEntry {
            position: self.transcript.len() as u64,
            kind: "content".into(),
            element_type: "markdown".into(),
            content: text,
            created_at_unix: chrono::Utc::now().timestamp().max(0) as u64,
        });
    }

    fn push_markdown(&mut self, content: String) {
        self.flush_text();
        self.transcript.push(TurnEntry {
            position: self.transcript.len() as u64,
            kind: "content".into(),
            element_type: "markdown".into(),
            content,
            created_at_unix: chrono::Utc::now().timestamp().max(0) as u64,
        });
    }

    fn info(&self, key: &ChannelKey) -> SessionInfo {
        // Native keys are feishu-channel `agent-{8-hex}` references with no
        // thread part; feed the flattened SessionInfo directly off the key.
        SessionInfo {
            channel: key.channel_str().to_string(),
            key: key.reference.clone(),
            session_id: Some(self.handle.key.clone()),
            status: "active".into(),
            phase: None,
            user_prompt: Some(self.prompt.clone()),
            last_active_unix: chrono::Utc::now().timestamp(),
            project_dir: self.workdir.clone(),
            // wire-webui-sebas-agent-e2e: 原生内核可用模型来自装配期的环境
            // 变量清单；当前模型取会话级 override，缺省为内核默认。
            current_model: self.current_model_override.clone().or(Some(self.default_model.clone())),
            available_models: Some(self.available_models.clone()),
        }
    }
}

/// The in-process backend over the native agent kernel.
pub struct NativeAgentBackend {
    manager: Arc<SessionManager>,
    /// 内核实际使用的审批回答者（`manager.approver()`；无则补一个 hub）。
    /// 统一走 trait object，`answer_permission` 直接 `answer()` 到内核。
    hub: Arc<dyn Approver>,
    /// Encoded key → session. Encoded keys are the URL-safe form the WebUI
    /// routes already use.
    sessions: Arc<RwLock<HashMap<String, NativeSession>>>,
    /// Lifecycle + review-card events for the WebUI relay.
    events: broadcast::Sender<SessionEvent>,
    /// Gated-call feed (review cards).
    notices: broadcast::Sender<PermissionNotice>,
    /// Why the native backend is unavailable (missing LLM credentials), if so.
    unavailable_cause: Option<String>,
    /// （wire-webui-sebas-agent-e2e）原生内核可供选择的模型 id 列表。来自
    /// `SEBAS_AGENT_MODELS`（逗号分隔），缺省仅含 `SEBAS_AGENT_MODEL`。
    /// WebUI composer 的模型下拉数据源。
    available_models: Vec<String>,
    /// （wire-webui-sebas-agent-e2e）默认模型 id（`SEBAS_AGENT_MODEL`），
    /// 在 native 会话尚未设置任何覆盖前对所有 turn 生效。
    default_model: String,
}

/// 无凭据时的占位 LLM 客户端：任何调用都以 terminal 错误失败。
/// 生产路径调不到它——`NativeAgentBackend::spawn` 先按
/// `unavailable_cause` 拒绝；它的存在让"manager 可建但不可用"的
/// 文档语义成立，替代曾经的构造期 panic（bead sebas-rqv）。
struct DeadLlmClient;

#[async_trait::async_trait]
impl LlmClient for DeadLlmClient {
    async fn stream_turn(
        &self,
        _req: &LlmRequest,
        _sink: &(dyn Fn(StreamEvent) + Send + Sync),
    ) -> Result<LlmTurn, LlmError> {
        Err(LlmError::terminal(
            "native backend has no LLM credentials \
             (set SEBAS_AGENT_PROVIDER_API_KEY or SEBAS_AGENT_GATEWAY_URL)",
        ))
    }
}

impl NativeAgentBackend {
    /// 从环境装配原生内核 manager（design N9）。优先直连
    /// `SEBAS_AGENT_PROVIDER_BASE_URL` + `SEBAS_AGENT_PROVIDER_API_KEY`
    /// （默认端点 `https://api.anthropic.com`），或
    /// `SEBAS_AGENT_GATEWAY_URL`（+ 可选 `SEBAS_AGENT_GATEWAY_AUTH`）走 gateway。
    ///
    /// 无凭据时返回 `(manager, Some(cause), …)`——manager 可建但每个 spawn
    /// 会拒绝并报 cause；可用模型与默认模型从 `SEBAS_AGENT_MODELS` /
    /// `SEBAS_AGENT_MODEL` 推导，与管理器共享同一装配面（wire-webui-sebas-agent-e2e D5）。
    pub fn build_native_manager(
        bash_timeout: Duration,
    ) -> (
        Arc<SessionManager>,
        Option<String>,
        Vec<String>,
        String,
    ) {
        let (client, cause): (Option<Arc<dyn LlmClient>>, Option<String>) =
            if let Ok(url) = std::env::var("SEBAS_AGENT_GATEWAY_URL") {
                let auth = std::env::var("SEBAS_AGENT_GATEWAY_AUTH")
                    .unwrap_or_else(|_| "sk-gw-local-dev".into());
                (
                    Some(Arc::new(AnthropicMessagesClient::gateway(url, auth))),
                    None,
                )
            } else {
                let base =
                    std::env::var("SEBAS_AGENT_PROVIDER_BASE_URL")
                        .unwrap_or_else(|_| "https://api.anthropic.com".into());
                match std::env::var("SEBAS_AGENT_PROVIDER_API_KEY") {
                    Ok(key) if !key.is_empty() => (
                        Some(Arc::new(AnthropicMessagesClient::direct_provider(base, key))),
                        None,
                    ),
                    _ => (
                        None,
                        Some(
                            "native backend needs SEBAS_AGENT_PROVIDER_API_KEY \
                             (or SEBAS_AGENT_GATEWAY_URL)"
                                .into(),
                        ),
                    ),
                }
            };

        let model = std::env::var("SEBAS_AGENT_MODEL").unwrap_or_else(|_| "claude-sonnet-4-5".into());
        // 无凭据时不 panic：以死客户端占位构造。`spawn` 先检查
        // `unavailable_cause` 并拒绝（诚实降级），占位客户端永远不被调用；
        // 直接调到它 = 既有门卫失效，terminal 错误立刻暴露。
        let client = client.unwrap_or_else(|| Arc::new(DeadLlmClient) as Arc<dyn LlmClient>);
        let available_models: Vec<String> = std::env::var("SEBAS_AGENT_MODELS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![model.clone()]);
        let manager = SessionManager::new(
            client,
            // 沙箱档位（design N2 配置面）：默认 Auto（Landlock 可用即用，
            // 否则防火墙回退）；`SEBAS_AGENT_BASH_SANDBOX=firewall` 强制回退档。
            ToolRegistry::with_sandbox(bash_timeout, agent_sandbox_mode()),
            SessionConfig {
                model: model.clone(),
                ..Default::default()
            },
        )
        .with_policy(Arc::new(PolicyEngine::new(PolicyConfig::default())))
        .with_approver(ApproverHub::new());
        (Arc::new(manager), cause, available_models, model)
    }

    /// Build the backend. Reads the agent LLM channel from the environment
    /// (design N9). Without credentials the backend reports honestly
    /// degraded: every spawn rejects with the cause.
    pub fn from_env(bash_timeout: Duration) -> Arc<Self> {
        let (manager, cause, available_models, default_model) =
            Self::build_native_manager(bash_timeout);
        Self::new(manager, cause, available_models, default_model)
    }

    /// Inject an already-configured manager (tests, or hosts that read the
    /// provider registry themselves).
    pub fn with_manager(manager: SessionManager) -> Arc<Self> {
        Self::new(
            Arc::new(manager),
            None,
            vec!["claude-sonnet-4-5".into()],
            "claude-sonnet-4-5".into(),
        )
    }

    /// Inject an already-configured manager behind an `Arc`（webui、通道
    /// server 与 feishu 桥共享同一个内核 manager）。`cause` = 装配时发现的
    /// 凭据缺失原因（None = 凭据齐全），由宿主从 `build_native_manager` 透传。
    pub fn with_manager_arc(
        manager: Arc<SessionManager>,
        cause: Option<String>,
        available_models: Vec<String>,
        default_model: String,
    ) -> Arc<Self> {
        Self::new(manager, cause, available_models, default_model)
    }

    /// 暴露内嵌的内核 manager（供 feishu 原生桥共享同一执行面）。
    pub fn native_manager(&self) -> Arc<SessionManager> {
        self.manager.clone()
    }

    fn new(
        manager: Arc<SessionManager>,
        unavailable_cause: Option<String>,
        available_models: Vec<String>,
        default_model: String,
    ) -> Arc<Self> {
        // The kernel needs an approver to surface gated calls. 生产路径
        // （`from_env`/`build_native_manager`）已挂 approver；`with_manager`
        // 注入的 manager 若缺，用它自己的 hub 补一个（此前缺了这个回填，
        // 双 hub 错位会让 webui 的 answer 到不了内核——sebas-22f 类问题）。
        let hub: Arc<dyn Approver> = match manager.approver() {
            Some(a) => a,
            None => ApproverHub::new(),
        };
        let (events, _) = broadcast::channel(256);
        let (notices, _) = broadcast::channel(64);
        Arc::new(Self {
            manager,
            hub,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            events,
            notices,
            unavailable_cause,
            available_models,
            default_model,
        })
    }

    fn encode_key(key: &ChannelKey) -> String {
        serde_json::to_string(key).expect("ChannelKey serialization")
    }

    async fn session_info(&self, encoded: &str) -> Option<SessionInfo> {
        let g = self.sessions.read().await;
        g.get(encoded).map(|s| s.info(&Self::decode_agent_key(encoded)))
    }

    fn decode_agent_key(encoded: &str) -> ChannelKey {
        serde_json::from_str(encoded).unwrap_or_else(|_| ChannelKey {
            channel: "feishu".into(),
            reference: encoded.to_string(),
        })
    }

    /// Drive one native session: kernel events → transcript + lifecycle
    /// events + review-card notices. Runs until the session task dies.
    async fn pump(
        mut rx: broadcast::Receiver<AgentEvent>,
        key: ChannelKey,
        encoded: String,
        sessions: Arc<RwLock<HashMap<String, NativeSession>>>,
        events: broadcast::Sender<SessionEvent>,
        notices: broadcast::Sender<PermissionNotice>,
    ) {
        use AgentEvent as AE;
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            let mut removed = false;
            // 锁内只做变更与 frame 计算；发送在锁外（发事件会同步唤醒订阅者）。
            let frame: Option<SessionEvent> = {
                let mut g = sessions.write().await;
                let Some(session) = g.get_mut(&encoded) else { break };
                match ev {
                    AE::TextDelta { delta, .. } => {
                        session.text_buf.push_str(&delta);
                        None
                    }
                    AE::ThinkingDelta { .. } | AE::ToolProgress { .. } | AE::ToolFinish { .. } => None,
                    AE::ToolStart { tool_name, args, .. } => {
                        let args_str = serde_json::to_string_pretty(&args).unwrap_or_default();
                        session
                            .push_markdown(format!("📖 **{tool_name}**\n```json\n{args_str}\n```"));
                        Some(SessionEvent::Updated { session: session.info(&key) })
                    }
                    AE::ToolEnd { tool_name, result, .. } => {
                        session.push_markdown(format!("✓ **{tool_name}**\n{result}"));
                        Some(SessionEvent::Updated { session: session.info(&key) })
                    }
                    AE::PermissionRequest { request_id, tool_name, args, reason, .. } => {
                        session.push_markdown(format!(
                            "⏳ **{tool_name}** awaits approval — {reason}"
                        ));
                        let _ = notices.send(PermissionNotice {
                            request_id,
                            session_id: encoded.clone(),
                            tool_name,
                            args,
                            reason,
                        });
                        Some(SessionEvent::Updated { session: session.info(&key) })
                    }
                    AE::ToolPolicy { tool_name, outcome, .. } => {
                        session.push_markdown(format!("🛡 **{tool_name}** policy: {outcome}"));
                        Some(SessionEvent::Updated { session: session.info(&key) })
                    }
                    AE::SessionSummary { turn_ms, model_calls, tool_calls, .. } => {
                        session.push_markdown(format!(
                            "🗒 turn summary — {model_calls} model calls, {tool_calls} tools, {turn_ms}ms"
                        ));
                        Some(SessionEvent::Updated { session: session.info(&key) })
                    }
                    AE::Error { message, terminal, .. } => {
                        session.push_markdown(format!("⚠ {message}"));
                        removed = terminal;
                        None
                    }
                    AE::Finished { .. } => {
                        session.flush_text();
                        Some(SessionEvent::Updated { session: session.info(&key) })
                    }
                }
            };
            match frame {
                Some(SessionEvent::Updated { .. }) | None => {
                    if let Some(frame) = frame {
                        let _ = events.send(frame);
                    }
                }
                _ => {}
            }
            if removed {
                sessions.write().await.remove(&encoded);
                let _ = events.send(SessionEvent::Removed {
                    channel: key.channel_str().to_string(),
                    key: key.reference.clone(),
                });
                break;
            }
        }
    }
}

#[async_trait::async_trait]
impl SessionBackend for NativeAgentBackend {
    async fn snapshot(&self) -> Vec<SessionInfo> {
        let g = self.sessions.read().await;
        let mut out: Vec<SessionInfo> =
            g.iter().map(|(encoded, s)| s.info(&Self::decode_agent_key(encoded))).collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.last_active_unix));
        out
    }

    async fn focused(&self) -> Option<ChannelKey> {
        // The native backend does not track focus; the acp side owns it.
        None
    }

    async fn set_focus(&self, _key: Option<ChannelKey>) {}

    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        if let Some(cause) = &self.unavailable_cause {
            return Err(SessionRejection::Unavailable { cause: cause.clone() });
        }
        let workdir: PathBuf = match project_dir.as_ref() {
            Some(dir) => {
                let p = PathBuf::from(dir);
                if !p.is_dir() {
                    return Err(SessionRejection::UnusableProjectDir);
                }
                p
            }
            None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };
        let handle = self.manager.create_session(workdir);
        // Native sessions live on the feishu channel with a bare
        // `agent-{8-hex}` reference (no thread part).
        let key = ChannelKey::new("feishu", format!("agent-{}", &handle.key[..8.min(handle.key.len())]));
        let encoded = Self::encode_key(&key);
        {
            let mut g = self.sessions.write().await;
            g.insert(
                encoded.clone(),
                NativeSession {
                    handle,
                    workdir: project_dir.clone(),
                    prompt: prompt.clone(),
                    transcript: Vec::new(),
                    text_buf: String::new(),
                    current_model_override: None,
                    available_models: self.available_models.clone(),
                    default_model: self.default_model.clone(),
                },
            );
        }
        let info = self.session_info(&encoded).await;
        if let Some(info) = info {
            let _ = self.events.send(SessionEvent::Created { session: info });
        }

        // Kernel pump：先订阅（broadcast 只转发订阅后的事件）再首 prompt。
        let rx = {
            let g = self.sessions.read().await;
            g.get(&encoded).expect("just inserted").handle.subscribe()
        };
        let sessions = self.sessions.clone();
        let events = self.events.clone();
        let notices = self.notices.clone();
        let pump_key = key.clone();
        let pump_encoded = encoded.clone();
        tokio::spawn(async move {
            Self::pump(rx, pump_key, pump_encoded, sessions, events, notices).await;
        });

        // First prompt drives the first turn.
        {
            let g = self.sessions.read().await;
            let h = &g.get(&encoded).expect("just inserted").handle;
            h.prompt(prompt).await;
        }
        Ok(key)
    }

    /// （wire-webui-sebas-agent-e2e）native 会话级模型 override。先写本地
    /// 当前模型字段，再下发内核 `set_model` 命令作用于后续 turn。`model_id`
    /// 不在 `available_models` 内仍接受（与 ACP 行为一致 —— 模型 ID 合法性
    /// 由内核 LLM 客户端实时校验）。
    async fn set_session_model(&self, key: ChannelKey, model_id: String) -> Result<(), SessionRejection> {
        let encoded = Self::encode_key(&key);
        let mut g = self.sessions.write().await;
        let Some(session) = g.get_mut(&encoded) else {
            return Err(SessionRejection::UnknownSession { key: encoded });
        };
        session.current_model_override = Some(model_id.clone());
        session.handle.set_model(model_id).await;
        Ok(())
    }

    async fn message(&self, key: ChannelKey, message: String) -> Result<(), SessionRejection> {
        let encoded = Self::encode_key(&key);
        let g = self.sessions.read().await;
        let Some(session) = g.get(&encoded) else {
            return Err(SessionRejection::UnknownSession { key: encoded });
        };
        session.handle.prompt(message).await;
        Ok(())
    }

    async fn close(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        let encoded = Self::encode_key(&key);
        let mut g = self.sessions.write().await;
        let Some(session) = g.remove(&encoded) else {
            return Err(SessionRejection::UnknownSession { key: encoded });
        };
        session.handle.cancel().await;
        Ok(())
    }

    async fn turns(&self, key: ChannelKey, from: u64) -> Result<Vec<TurnEntry>, SessionRejection> {
        let encoded = Self::encode_key(&key);
        let g = self.sessions.read().await;
        let Some(session) = g.get(&encoded) else {
            return Err(SessionRejection::UnknownSession { key: encoded });
        };
        Ok(session
            .transcript
            .iter()
            .filter(|e| e.position >= from)
            .cloned()
            .collect())
    }

    async fn reachability(&self) -> Reachability {
        match &self.unavailable_cause {
            Some(cause) => Reachability::Unreachable { cause: cause.clone() },
            None => Reachability::Reachable,
        }
    }

    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        Some(self.notices.subscribe())
    }

    async fn answer_permission(&self, request_id: &str, decision: PermissionDecision) -> bool {
        let answer = match decision {
            PermissionDecision::AllowOnce => ApprovalAnswer::AllowOnce,
            PermissionDecision::AllowSession => ApprovalAnswer::AllowSession,
            PermissionDecision::Deny => ApprovalAnswer::Deny,
            PermissionDecision::Escalate { reason } => ApprovalAnswer::Escalate { reason },
        };
        self.hub.answer(request_id, answer)
    }
}

/// The composite seam: one dashboard, two execution backends. Spawn routes on
/// the optional backend hint (`"native"` → the built-in kernel; anything else
/// → the Claude Code bridge); every other call routes on the key prefix
/// (`agent-*` chat ids belong to native sessions).
pub struct DualSessionBackend {
    pub acp: Arc<dyn SessionBackend>,
    pub native: Arc<NativeAgentBackend>,
    events: broadcast::Sender<SessionEvent>,
    /// Merged review-card notices from both children (acp + native), so a
    /// Claude/ACP permission request reaches the webui review card through the
    /// same channel as a native gated call.
    notices: broadcast::Sender<PermissionNotice>,
}

impl DualSessionBackend {
    pub fn new(acp: Arc<dyn SessionBackend>, native: Arc<NativeAgentBackend>) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        let (notices, _) = broadcast::channel(64);
        // Merge both children's lifecycle streams into one relay.
        {
            let tx = events.clone();
            let mut rx = acp.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let _ = tx.send(ev);
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
        }
        {
            let tx = events.clone();
            let mut rx = native.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let _ = tx.send(ev);
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
        }
        // Merge both children's permission-notice streams into one relay, so
        // review cards from either backend surface on the same feed.
        if let Some(mut rx) = acp.permission_requests() {
            let tx = notices.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(notice) => {
                            let _ = tx.send(notice);
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
        }
        if let Some(mut rx) = native.permission_requests() {
            let tx = notices.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(notice) => {
                            let _ = tx.send(notice);
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
        }
        Arc::new(Self {
            acp,
            native,
            events,
            notices,
        })
    }

    /// Native keys are feishu-channel `agent-*` references; everything else
    /// (including feishu chat keys and web keys) routes to the ACP bridge.
    /// Public: the core channel server uses the same predicate for its
    /// web-key existence pre-check (native sessions live outside the router
    /// map and would otherwise be wrongly rejected as unknown).
    pub fn is_native(key: &ChannelKey) -> bool {
        key.channel_str() == "feishu" && key.reference.starts_with("agent-")
    }

    fn route(&self, key: &ChannelKey) -> &dyn SessionBackend {
        if Self::is_native(key) {
            self.native.as_ref()
        } else {
            self.acp.as_ref()
        }
    }
}

#[async_trait::async_trait]
impl SessionBackend for DualSessionBackend {
    async fn snapshot(&self) -> Vec<SessionInfo> {
        let mut all = self.acp.snapshot().await;
        all.extend(self.native.snapshot().await);
        all.sort_by_key(|s| std::cmp::Reverse(s.last_active_unix));
        all
    }

    async fn focused(&self) -> Option<ChannelKey> {
        self.acp.focused().await
    }

    async fn set_focus(&self, key: Option<ChannelKey>) {
        match key {
            Some(k) if Self::is_native(&k) => self.native.set_focus(Some(k)).await,
            other => self.acp.set_focus(other).await,
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        self.acp.spawn(prompt, project_dir).await
    }

    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        backend: Option<&str>,
        _model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        match backend {
            Some("native") => self.native.spawn(prompt, project_dir).await,
            // `acp` / `acp:<slug>` (and any other non-native hint) route to
            // the ACP backend, which parses the slug and pins the kind. The
            // model id (add-acp-model-selection) is threaded into the spawn.
            _ => self.acp.spawn_with(prompt, project_dir, backend, _model).await,
        }
    }

    /// 0-turn placeholder: route to the backend that would own the session,
    /// so neither spawns an agent child for an empty prompt (P2).
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        backend: Option<String>,
        model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        match backend.as_deref() {
            Some("native") => self.native.spawn(String::new(), project_dir).await,
            _ => self.acp.create_placeholder(project_dir, backend, model).await,
        }
    }

    async fn message(&self, key: ChannelKey, message: String) -> Result<(), SessionRejection> {
        self.route(&key).message(key, message).await
    }

    async fn close(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        self.route(&key).close(key).await
    }

    async fn set_session_model(&self, key: ChannelKey, model_id: String) -> Result<(), SessionRejection> {
        // wire-webui-sebas-agent-e2e：按 key 分发。原生 key 路由到内核，
        // ACP key 走既有 InProcessBackend 的 Out::SendAcp SetModel。
        self.route(&key).set_session_model(key, model_id).await
    }

    async fn turns(&self, key: ChannelKey, from: u64) -> Result<Vec<TurnEntry>, SessionRejection> {
        self.route(&key).turns(key, from).await
    }

    async fn reachability(&self) -> Reachability {
        // 整体可达性 = session authority（core）是否可达，跟 acp 侧一致；
        // 某个执行体自身不可用（如 native 缺凭据）不是"core 不可达"，
        // 由 execution_bodies 逐体如实上报，不拉低整体门禁误伤 acp。
        self.acp.reachability().await
    }

    async fn execution_bodies(&self) -> Option<Vec<sebas_webui::session_backend::ExecutionBodyStatus>> {
        let acp = match self.acp.reachability().await {
            Reachability::Reachable => sebas_webui::session_backend::ExecutionBodyStatus {
                name: "acp".into(),
                ok: true,
                cause: None,
            },
            Reachability::Unreachable { cause } => {
                sebas_webui::session_backend::ExecutionBodyStatus {
                    name: "acp".into(),
                    ok: false,
                    cause: Some(cause),
                }
            }
        };
        let native = match self.native.reachability().await {
            Reachability::Reachable => sebas_webui::session_backend::ExecutionBodyStatus {
                name: "native".into(),
                ok: true,
                cause: None,
            },
            Reachability::Unreachable { cause } => {
                sebas_webui::session_backend::ExecutionBodyStatus {
                    name: "native".into(),
                    ok: false,
                    cause: Some(cause),
                }
            }
        };
        Some(vec![acp, native])
    }

    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        Some(self.notices.subscribe())
    }

    async fn answer_permission(&self, request_id: &str, decision: PermissionDecision) -> bool {
        if self.native.answer_permission(request_id, decision.clone()).await {
            return true;
        }
        self.acp.answer_permission(request_id, decision).await
    }
}

/// bash 沙箱档位解析（design N2 配置面）。
fn agent_sandbox_mode() -> SandboxMode {
    match std::env::var("SEBAS_AGENT_BASH_SANDBOX").as_deref() {
        Ok("firewall") => SandboxMode::Firewall,
        _ => SandboxMode::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_channels::ChannelKey;
    use sebas_acp::claude::session::{AcpCommand, AcpEvent, Decision};
    use sebas_agent::llm::fake::FakeLlmClient;
    use sebas_agent::policy::NetworkMode;
    use sebas_router::router::Out;
    use sebas_router::state::{Mapping, SessionMap};

    fn manager() -> SessionManager {
        // 脚本化：先破坏面 bash（→ Ask），再收尾文本。
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![(
                "t1",
                "bash",
                serde_json::json!({"command": "rm -rf build"}),
            )]),
            FakeLlmClient::say("gated call was approved"),
        ]);
        SessionManager::new(
            Arc::new(llm),
            ToolRegistry::with_sandbox(
                Duration::from_secs(10),
                sebas_agent::policy::SandboxMode::Firewall,
            ),
            SessionConfig::default(),
        )
        .with_policy(Arc::new(PolicyEngine::new(PolicyConfig {
            network: NetworkMode::Off,
            ..Default::default()
        })))
        // 生产路径（build_native_manager）已挂 approver；测试 manager 同样
        // 挂 hub，gated 调用才能呈现审查卡。
        .with_approver(sebas_agent::policy::ApproverHub::new())
    }

    #[tokio::test]
    async fn native_spawn_prompts_and_permission_round_trips() {
        let backend = NativeAgentBackend::with_manager(manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");

        // 审查卡流：拿到 request_id 后回填 allow-once。
        let mut notices = backend.permission_requests().expect("native has notices");
        let notice = tokio::time::timeout(Duration::from_secs(10), notices.recv())
            .await
            .expect("notice timeout")
            .expect("notice");
        assert_eq!(notice.tool_name, "bash");
        assert_eq!(
            notice.session_id,
            NativeAgentBackend::encode_key(&key),
            "session id is the encoded key"
        );
        assert!(
            backend
                .answer_permission(&notice.request_id, PermissionDecision::AllowOnce)
                .await,
            "answer must reach the pending request"
        );

        // turn 收尾后的 transcript：策略事件 + 完成文本可见。
        let deadline = Duration::from_secs(10);
        let _ = tokio::time::timeout(deadline, async {
            loop {
                let turns = backend.turns(key.clone(), 0).await.unwrap();
                let joined: String = turns.iter().map(|t| t.content.clone()).collect();
                if joined.contains("gated call was approved") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;

        // turns：transcript 里能看到审批与工具痕迹。
        let turns = backend.turns(key.clone(), 0).await.unwrap();
        let joined: String = turns.iter().map(|t| t.content.clone()).collect();
        assert!(joined.contains("bash"), "tool trace in transcript: {joined}");
        assert!(joined.contains("policy"), "policy event in transcript: {joined}");
        assert!(
            joined.contains("gated call was approved"),
            "completion text in transcript: {joined}"
        );

        // close 后 sessions 清空。
        assert!(backend.close(key).await.is_ok());
        assert!(backend.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn dual_routes_on_backend_hint_and_prefix() {
        let acp: Arc<dyn SessionBackend> =
            Arc::new(sebas_webui::session_backend::InProcessBackend::new(
                make_router().await,
            ));
        let dual = DualSessionBackend::new(acp, NativeAgentBackend::with_manager(manager()));
        // backend hint = native → key 前缀 agent-。
        let key = dual
            .spawn_with("go".into(), None, Some("native"), None)
            .await
            .expect("spawn native");
        assert!(DualSessionBackend::is_native(&key), "{:?}", key.reference);
        // 默认（无 hint）→ acp 路径：agent 前缀之外的 key。
        let acp_key = dual.spawn_with("hi".into(), None, None, None).await.expect("spawn acp");
        assert!(!DualSessionBackend::is_native(&acp_key));
    }

    /// 出站接收端必须保活：router 发送在通道关闭时会 panic。
    async fn make_router() -> sebas_router::RouterHandle {
        let (router, mut out_rx) = sebas_router::RouterHandle::new(sebas_router::SessionMap::new());
        tokio::spawn(async move {
            while out_rx.recv().await.is_some() {}
        });
        router
    }

    /// acp 会话权限经 dual 后端往返：acp 后端转出 PermissionNotice，dual 的
    /// `answer_permission` 先试 native（无匹配 → false）再回退到 acp（活路径），
    /// 最终经 `Out::SendAcp` 回路由出 `PermissionReply`。
    #[tokio::test]
    async fn acp_permission_round_trips_through_dual_backend() {
        let map = SessionMap::new();
        let key = ChannelKey::feishu("oc_dual", None);
        map.insert(key.clone(), Mapping::active("s1"))
            .await
            .unwrap();
        let (router, mut out_rx) = sebas_router::RouterHandle::new(map);

        let acp: Arc<dyn SessionBackend> = Arc::new(
            sebas_webui::session_backend::InProcessBackend::new(router.clone()),
        );
        let dual = DualSessionBackend::new(acp.clone(), NativeAgentBackend::with_manager(manager()));

        // 订阅 acp 后端审查卡流，再触发权限请求。
        let mut notices = acp.permission_requests().expect("acp has notices");
        router
            .dispatch_acp_event(AcpEvent::PermissionRequest {
                session_id: "s1".into(),
                request_id: "claude:toolu_dual".into(),
                tool_name: "Bash".into(),
                args: serde_json::json!({"cmd": "ls"}),
            })
            .await;

        let notice = tokio::time::timeout(Duration::from_secs(5), notices.recv())
            .await
            .expect("notice timeout")
            .expect("notice");
        assert_eq!(notice.request_id, "claude:toolu_dual");

        // dual.answer_permission：native 无匹配 → acp 回退命中（返回 true）。
        assert!(
            dual.answer_permission(&notice.request_id, PermissionDecision::AllowOnce)
                .await,
            "acp fallback must answer the pending request"
        );

        // 回路由：排掉权限卡（SendCard）后取到 PermissionReply。
        let reply = loop {
            let got = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
                .await
                .expect("permission reply not received in time")
                .expect("channel closed");
            if matches!(got, Out::SendAcp { .. }) {
                break got;
            }
        };
        match reply {
            Out::SendAcp {
                cmd:
                    AcpCommand::PermissionReply {
                        request_id,
                        decision,
                        ..
                    },
                ..
            } => {
                assert_eq!(request_id, "claude:toolu_dual");
                assert!(matches!(decision, Decision::AllowOnce));
            }
            other => panic!("expected SendAcp PermissionReply, got {other:?}"),
        }
    }
}
