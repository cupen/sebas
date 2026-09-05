//! The session backend seam (openspec/changes/add-core-session-channel — task 2.1).
//!
//! Mirrors the `AdminAdapter` seam: the webui crate owns the trait, the sebas
//! binary crate supplies implementations (in-process over `RouterHandle`, or
//! the core session channel socket client). The webui crate never depends on
//! the binary crate — that is the seam's whole point.
//!
//! Everything the session routes need flows through this trait: reads
//! (snapshot/turns/focus), mutations (spawn/message/close), the event
//! subscription for SSE, and the reachability report that drives honest
//! degradation rendering when the core is not connected.

use async_trait::async_trait;
use sebas_channels::key::ChannelKey;
use sebas_router::{SessionEvent, SessionInfo, TurnEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
/// Whether the backend can currently reach the session authority (the core),
/// and if not, why — rendered verbatim so degradation is honest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reachability {
    /// The core is reachable; session controls are live.
    Reachable,
    /// The core cannot be reached; the board renders the cause and the
    /// composer stays disabled.
    Unreachable { cause: String },
}

/// Typed rejection for a session mutation (spec: rejections name the reason;
/// nothing is mutated on rejection).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum SessionRejection {
    /// No session exists for the given key.
    UnknownSession { key: String },
    /// The requested project directory is not a usable directory.
    /// Deliberately carries no path details — no existence disclosure.
    UnusableProjectDir,
    /// The core is at its session capacity.
    Capacity { limit: usize },
    /// The request could not be delivered to the session authority.
    Unavailable { cause: String },
}

impl std::fmt::Display for SessionRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionRejection::UnknownSession { key } => write!(f, "会话不存在: {key}"),
            SessionRejection::UnusableProjectDir => {
                write!(f, "项目目录不可用（不是目录或无法访问）")
            }
            SessionRejection::Capacity { limit } => write!(f, "会话数已达上限 {limit}"),
            SessionRejection::Unavailable { cause } => write!(f, "核心不可达: {cause}"),
        }
    }
}

/// One gated tool call awaiting an operator decision (webui review card).
/// `session_id` is the encoded session key; `request_id` equals the kernel's
/// `tool_use_id` and is what [`SessionBackend::answer_permission`] takes back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionNotice {
    pub request_id: String,
    /// Encoded session key (URL-safe, as used in routes).
    pub session_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub reason: String,
}

/// （wire-webui-sebas-agent-e2e）单个执行体（acp / native）的可用性：
/// composer 据此禁选并标注 cause。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBodyStatus {
    /// `"acp"` | `"native"`（未来可扩展）。
    pub name: String,
    pub ok: bool,
    /// 不可用时的原因（如实透传给 UI）。
    pub cause: Option<String>,
}

/// The operator's answer to a [`PermissionNotice`]. `escalate` = one-shot
/// elevated retry carrying the operator's stated reason (the session policy
/// itself never widens).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PermissionDecision {
    AllowOnce,
    AllowSession,
    Deny,
    Escalate { reason: String },
}

/// The seam every session-data source must satisfy.
#[async_trait]
pub trait SessionBackend: Send + Sync {
    /// Every known session, in the shape the session rows need.
    async fn snapshot(&self) -> Vec<SessionInfo>;

    /// The currently focused session, if any.
    async fn focused(&self) -> Option<ChannelKey>;

    /// Mark the focused session (idempotent; clearing with `None`).
    async fn set_focus(&self, key: Option<ChannelKey>);

    /// Subscribe to session events (created / updated / removed / resync).
    /// Bounded: a lagging consumer sees `broadcast::error::RecvError::Lagged`.
    fn subscribe(&self) -> broadcast::Receiver<SessionEvent>;

    /// Create a session, optionally rooted in a project directory. Returns
    /// the new session key. The placeholder is immediately visible in
    /// `snapshot` (Spawning) and via the event stream (Created).
    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<ChannelKey, SessionRejection>;

    /// Send a message to an existing session. Unknown keys are rejected.
    async fn message(&self, key: ChannelKey, message: String) -> Result<(), SessionRejection>;

    /// Close a session (kills the live child when there is one).
    async fn close(&self, key: ChannelKey) -> Result<(), SessionRejection>;

    /// 中程切换会话模型（add-acp-model-selection）：把所选的 model id 送给
    /// 会话驱动（ACP `session/set_config_option{configId:"model"}`）。成功 =
    /// 命令已进入会话通道（wire 层接受与否经事件流反馈：`ModelChanged` =
    /// 成功，非 terminal `Error` = 模型被 agent 拒绝）。失败 =
    /// 会话未知/不可达（改会话不再被跟踪）。默认实现：没有模型交互
    /// （原生内核等）的后端返回不可达。
    async fn set_session_model(
        &self,
        _key: ChannelKey,
        _model_id: String,
    ) -> Result<(), SessionRejection> {
        Err(SessionRejection::Unavailable {
            cause: "此后端不支持会话级模型切换".into(),
        })
    }

    /// The session's rendered transcript at or after `from` (monotonic
    /// positions — a second call at the returned last position yields only
    /// newer entries).
    async fn turns(&self, key: ChannelKey, from: u64) -> Result<Vec<TurnEntry>, SessionRejection>;

    /// Whether the session authority is reachable right now, and if not, why.
    async fn reachability(&self) -> Reachability;

    /// （wire-webui-sebas-agent-e2e）各执行体的逐体可用性，供 composer 把
    /// 不可用的执行体禁选 + 标注 cause（spec：不可用执行体不因整体门禁
    /// 误伤其他执行体）。`None` = 此后端不区分执行体（summary 省略该段，
    /// 前端降级为只看整体 reachability）。
    async fn execution_bodies(&self) -> Option<Vec<ExecutionBodyStatus>> {
        None
    }

    /// Live stream of gated tool calls awaiting a decision (the review-card
    /// feed). `None` = this backend has no permission interaction (its
    /// sessions never gate, or gating is surfaced elsewhere).
    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        None
    }

    /// Deliver an operator decision for `request_id`. Returns `false` when
    /// no pending request carries that id (already answered, timed out, or
    /// unknown — callers may retry briefly).
    async fn answer_permission(&self, _request_id: &str, _decision: PermissionDecision) -> bool {
        false
    }

    /// Create a session, optionally pinning the execution backend. The
    /// default ignores the hint (single-backend seams); composite seams
    /// route on it. `model`（add-acp-model-selection）是创建时请求的模型 id：
    /// 会话建立后、首个 prompt 前应用（失败报非致命错误、会话仍可对话）。
    /// 默认实现忽略 model（单后端 seams 无模型选择面）。
    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        _backend: Option<&str>,
        _model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        self.spawn(prompt, project_dir).await
    }

    // ─── State store methods (add-state-store) ───────────────────────────────

    /// Load a snapshot of the core state store's domain.
    /// Returns `None` when the domain is unknown or the store is unreachable.
    async fn state_snapshot(&self, domain: &str) -> Option<serde_json::Value> {
        let _ = domain;
        None
    }

    /// Mutate a domain of the core state store.
    /// Returns `Ok(())` on success, or an error string.
    async fn state_mutate(&self, domain: &str, payload: serde_json::Value) -> Result<(), String> {
        let _ = (domain, payload);
        Err("state store 不可用".into())
    }

    /// Create a 0-turn placeholder session without spawning an agent child
    /// (P2 fix: an empty prompt must not be sent to the agent — opencode
    /// hangs on `session/prompt ""`). `backend` is the same execution-backend
    /// hint as [`SessionBackend::spawn_with`] (`"acp:<slug>"` etc.), and
    /// `model` the requested model id; both are remembered for the first
    /// message's spawn. The default falls back to `spawn("", …)` for backends
    /// without placeholder support (keeps the old callable surface honest).
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        _backend: Option<String>,
        _model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        self.spawn(String::new(), project_dir).await
    }
}

// ─── In-process implementation (task 2.2) ──────────────────────────────────

/// Parse a webui backend hint into the requested ACP agent kind slug.
/// `"acp:<slug>"` → `Some(slug)`; a bare `"acp"` (or anything else, including
/// `"native"` which the composite seam handles before it reaches here) →
/// `None`, meaning the configured default kind.
fn parse_acp_kind(backend: &str) -> Option<String> {
    backend
        .strip_prefix("acp:")
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// projects 域 mutation 分发（与 core channel 服务端同款）：payload 用
/// `op` 字段区分子操作——add / remove / save。
async fn project_mutation(
    engine: &(dyn sebas_router::state_store::StateStoreEngine + Send + Sync),
    payload: &serde_json::Value,
) -> Result<(), String> {
    let op = payload
        .get("op")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("save");
    match op {
        "add" => {
            let path = payload
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "add: 缺少 path 字段".to_string())?;
            let name = payload
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "add: 缺少 name 字段".to_string())?;
            let added_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            engine.add_project(path, name, added_at).await
        }
        "remove" => {
            let path = payload
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "remove: 缺少 path 字段".to_string())?;
            match engine.remove_project(path).await {
                Ok(true) => Ok(()),
                Ok(false) => Err(format!("remove: project '{path}' 不存在")),
                Err(e) => Err(e),
            }
        }
        "save" => {
            let projects = payload
                .get("projects")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            engine.save_projects(projects).await
        }
        other => Err(format!("projects: 未知 op '{other}'")),
    }
}

/// In-process backend over the router. Used by `sebas run --webui`, where the
/// webui lives in the same process as the session authority.
pub struct InProcessBackend {
    router: sebas_router::RouterHandle,
    /// Review-card notices relayed from the router's ACP permission broadcast.
    notices: broadcast::Sender<PermissionNotice>,
    /// `request_id` → routing `session_id`, recorded when a PermissionRequest
    /// is relayed, so `answer_permission` can route the reply back to the
    /// owning session without the caller knowing the session id.
    request_sessions: Arc<RwLock<HashMap<String, String>>>,
}

impl InProcessBackend {
    pub fn new(router: sebas_router::RouterHandle) -> Self {
        let (notices, _) = broadcast::channel(64);
        let request_sessions = Arc::new(RwLock::new(HashMap::new()));

        // Relay the router's independent ACP permission broadcast (design D6)
        // into the `PermissionNotice` review-card feed. `session_id` is the
        // URL-safe encoded ChannelKey — the same shape the WebUI routes and the
        // review-card filter key off.
        {
            let router = router.clone();
            let notices = notices.clone();
            let request_sessions = request_sessions.clone();
            // 同步订阅（在 spawn 之前）确保广播在第一条 PermissionRequest
            // 到达时已有接收端，避免 tokio broadcast "无接收者" 丢事件。
            let mut rx = router.subscribe_acp_permission_requests();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(sebas_acp::AcpEvent::PermissionRequest {
                            session_id,
                            request_id,
                            tool_name,
                            args,
                        }) => {
                            let key = router.map.lookup_key_by_session(&session_id).await;
                            let encoded = key
                                .map(|k| crate::routes::encode_session_key(&k))
                                .unwrap_or_else(|| session_id.clone());
                            request_sessions
                                .write()
                                .await
                                .insert(request_id.clone(), session_id);
                            let _ = notices.send(PermissionNotice {
                                request_id,
                                session_id: encoded,
                                tool_name,
                                args,
                                reason: String::new(),
                            });
                        }
                        // 广播只承载 PermissionRequest；其余变体到不了这里。
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        Self {
            router,
            notices,
            request_sessions,
        }
    }
}

/// `PermissionDecision` → ACP `Decision`（design D6/R5）。ACP 侧没有 escalate
/// 等价，`Escalate` 降级为 `AllowOnce`（reason 丢弃，记为已知取舍）。
fn map_permission_decision(d: PermissionDecision) -> sebas_acp::Decision {
    match d {
        PermissionDecision::AllowOnce => sebas_acp::Decision::AllowOnce,
        PermissionDecision::AllowSession => sebas_acp::Decision::AllowSession,
        PermissionDecision::Deny => sebas_acp::Decision::Deny,
        PermissionDecision::Escalate { reason } => {
            tracing::warn!(%reason, "ACP 无 escalate 等价；降级为 AllowOnce");
            sebas_acp::Decision::AllowOnce
        }
    }
}

#[async_trait]
impl SessionBackend for InProcessBackend {
    async fn snapshot(&self) -> Vec<SessionInfo> {
        self.router.session_info_snapshot().await
    }

    async fn focused(&self) -> Option<ChannelKey> {
        self.router.active_session_snapshot().await
    }

    async fn set_focus(&self, key: Option<ChannelKey>) {
        self.router.web_set_active(key).await;
    }

    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.router.subscribe_session_events()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        // web_spawn never fails structurally: the placeholder is inserted and
        // the spawn failure surfaces as a Removed event later.
        Ok(self.router.web_spawn(prompt, project_dir, None, None).await)
    }

    /// Parse a backend hint (`"native"` handled by the composite seam;
    /// `"acp"` / `"acp:<slug>"` arrive here) into the requested agent kind,
    /// then spawn through the router with that kind pinned and the requested
    /// model id threaded to the spawn out（D3：建会话后、首 prompt 前应用）。
    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        backend: Option<&str>,
        model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        let kind = backend.and_then(parse_acp_kind);
        Ok(self.router.web_spawn(prompt, project_dir, kind, model).await)
    }

    /// 0-turn placeholder: create the session row without spawning an agent
    /// child (P2 fix). The requested kind/model are remembered on the mapping
    /// so the first message spawns the right agent.
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        backend: Option<String>,
        model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        let kind = backend.as_deref().and_then(parse_acp_kind);
        Ok(self
            .router
            .web_create_placeholder(project_dir, kind, model)
            .await)
    }

    async fn set_session_model(&self, key: ChannelKey, model_id: String) -> Result<(), SessionRejection> {
        // 解析路由 session_id（web 会话的 chat_id 是 web-* 键，不是 ACP
        // routing id），再经 Out::SendAcp 送达 SetModel。
        let Some(sid) = self.router.map.get(&key).await.and_then(|m| m.session_id().map(str::to_owned))
        else {
            return Err(SessionRejection::UnknownSession {
                key: key.reference.clone(),
            });
        };
        self.router
            .emit(sebas_router::Out::SendAcp {
                session_id: sid.clone(),
                cmd: sebas_acp::AcpCommand::SetModel {
                    session_id: sid,
                    model_id,
                },
            })
            .await;
        Ok(())
    }

    async fn message(&self, key: ChannelKey, message: String) -> Result<(), SessionRejection> {
        // Route semantics preserved: an unknown key spawns a new session (the
        // feishu inbound path behaves the same). Typed rejections apply to the
        // channel server, which pre-checks existence.
        self.router.web_send_message(key, message).await;
        Ok(())
    }

    async fn close(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        match self.router.web_close_session(key).await {
            sebas_router::router::CloseOutcome::Closed => Ok(()),
            sebas_router::router::CloseOutcome::NotFound => {
                Err(SessionRejection::UnknownSession {
                    key: String::new(),
                })
            }
        }
    }

    async fn turns(&self, key: ChannelKey, from: u64) -> Result<Vec<TurnEntry>, SessionRejection> {
        self.router
            .session_turns(&key, from)
            .await
            .ok_or(SessionRejection::UnknownSession {
                key: key.reference.clone(),
            })
    }

    async fn reachability(&self) -> Reachability {
        // Same process as the authority: always reachable.
        Reachability::Reachable
    }

    async fn state_snapshot(&self, domain: &str) -> Option<serde_json::Value> {
        // In-process backend: use the engine when available.
        let engine = sebas_router::state_store::engine()?;
        match domain {
            "settings" => engine.load_settings().await.ok().flatten(),
            "providers" => {
                let state = engine.load_persisted_state().await;
                Some(serde_json::to_value(&state).ok()?)
            }
            "projects" => {
                // Load projects from the DB via the engine.
                let projects = engine.load_projects().await.ok()?;
                Some(serde_json::json!({ "projects": projects }))
            }
            _ => None,
        }
    }

    async fn state_mutate(&self, domain: &str, payload: serde_json::Value) -> Result<(), String> {
        let engine = sebas_router::state_store::engine()
            .ok_or_else(|| "state store 未初始化".to_string())?;
        match domain {
            "settings" => {
                let value = payload.get("value").cloned().unwrap_or(payload);
                engine.save_settings(value).await
            }
            "projects" => {
                // 与 core channel 服务端同款 op 分发（add/remove/save）。
                project_mutation(engine.as_ref(), &payload).await
            }
            other => Err(format!("unknown domain: {other}")),
        }
    }

    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        Some(self.notices.subscribe())
    }

    async fn answer_permission(&self, request_id: &str, decision: PermissionDecision) -> bool {
        let session_id = self
            .request_sessions
            .read()
            .await
            .get(request_id)
            .cloned();
        let Some(session_id) = session_id else {
            return false;
        };
        // 原生会话（make-feishu-optional-webui-primary）：权限请求来自桥 →
        // 决定回填到原生内核（ApproverHub）。先试 native，失败再回退 acp。
        let native = match decision.clone() {
            PermissionDecision::AllowOnce => {
                sebas_router::native_bridge::NativeApprovalDecision::AllowOnce
            }
            PermissionDecision::AllowSession => {
                sebas_router::native_bridge::NativeApprovalDecision::AllowSession
            }
            PermissionDecision::Deny => sebas_router::native_bridge::NativeApprovalDecision::Deny,
            PermissionDecision::Escalate { reason } => {
                sebas_router::native_bridge::NativeApprovalDecision::Escalate { reason }
            }
        };
        if self.router.answer_native_permission(request_id, native).await {
            return true;
        }
        // acp 会话：走既有 Out::SendAcp PermissionReply。
        let decision = map_permission_decision(decision);
        self.router
            .emit(sebas_router::Out::SendAcp {
                session_id: session_id.clone(),
                cmd: sebas_acp::AcpCommand::PermissionReply {
                    session_id,
                    request_id: request_id.to_string(),
                    decision,
                },
            })
            .await;
        true
    }
}

// ─── Fake backend for tests (task 2.3) ─────────────────────────────────────

/// Fake backend for tests: settable session set, in-memory transcript,
/// and an "unreachable" mode. No child process, no socket.
pub struct FakeBackend {
    inner: tokio::sync::RwLock<FakeState>,
    events: broadcast::Sender<SessionEvent>,
    reachable: std::sync::atomic::AtomicBool,
    unreachable_cause: std::sync::Mutex<Option<String>>,
    /// The next spawn index — used to mint distinct fake keys.
    next_spawn: std::sync::atomic::AtomicU64,
}

#[derive(Default)]
struct FakeState {
    sessions: Vec<SessionInfo>,
    focused: Option<ChannelKey>,
    transcripts: HashMap<String, Vec<TurnEntry>>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBackend {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            inner: tokio::sync::RwLock::new(FakeState::default()),
            events,
            reachable: std::sync::atomic::AtomicBool::new(true),
            unreachable_cause: std::sync::Mutex::new(None),
            next_spawn: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Seed/replace the visible session set.
    pub async fn set_sessions(&self, sessions: Vec<SessionInfo>) {
        self.inner.write().await.sessions = sessions;
    }

    /// Append one transcript entry for `session_id` (position auto-assigned).
    pub async fn push_turn(&self, session_id: &str, kind: &str, content: &str) {
        self.push_turn_typed(session_id, kind, "markdown", content)
            .await;
    }

    /// `push_turn` with an explicit `element_type` ("markdown" | "thinking").
    pub async fn push_turn_typed(
        &self,
        session_id: &str,
        kind: &str,
        element_type: &str,
        content: &str,
    ) {
        let mut g = self.inner.write().await;
        let log = g.transcripts.entry(session_id.to_string()).or_default();
        let position = log.len() as u64;
        log.push(TurnEntry {
            position,
            kind: kind.to_string(),
            element_type: element_type.to_string(),
            content: content.to_string(),
            created_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        });
    }

    /// Flip reachability; `cause` is reported while unreachable.
    pub fn set_reachable(&self, reachable: bool, cause: &str) {
        self.reachable
            .store(reachable, std::sync::atomic::Ordering::SeqCst);
        *self.unreachable_cause.lock().unwrap() = Some(cause.to_string());
    }

    /// Emit an event as if the authority had published it.
    pub fn emit(&self, ev: SessionEvent) {
        let _ = self.events.send(ev);
    }

    fn key_str(key: &ChannelKey) -> String {
        serde_json::to_string(key).unwrap_or_default()
    }
}

#[async_trait]
impl SessionBackend for FakeBackend {
    async fn snapshot(&self) -> Vec<SessionInfo> {
        self.inner.read().await.sessions.clone()
    }

    async fn focused(&self) -> Option<ChannelKey> {
        self.inner.read().await.focused.clone()
    }

    async fn set_focus(&self, key: Option<ChannelKey>) {
        self.inner.write().await.focused = key;
    }

    fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        if !self.reachable.load(std::sync::atomic::Ordering::SeqCst) {
            let cause = self
                .unreachable_cause
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default();
            return Err(SessionRejection::Unavailable { cause });
        }
        let n = self
            .next_spawn
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let key = ChannelKey::new("web", format!("web-fake-{n}"));
        let session = SessionInfo {
            channel: key.channel.as_str().to_string(),
            key: key.reference.clone(),
            session_id: None,
            status: "spawning".into(),
            phase: None,
            user_prompt: None,
            last_active_unix: 0,
            project_dir,
            current_model: None,
            available_models: None,
            agent_kind: None,
        };
        let ev = SessionEvent::Created { session };
        if let SessionEvent::Created { session } = &ev {
            self.inner.write().await.sessions.push(session.clone());
        }
        self.emit(ev);
        let _ = prompt; // the fake does not model prompt-driven topic derivation
        Ok(key)
    }

    async fn message(&self, key: ChannelKey, _message: String) -> Result<(), SessionRejection> {
        if !self.reachable.load(std::sync::atomic::Ordering::SeqCst) {
            let cause = self
                .unreachable_cause
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default();
            return Err(SessionRejection::Unavailable { cause });
        }
        let exists = {
            let g = self.inner.read().await;
            g.sessions
                .iter()
                .any(|s| s.channel == key.channel.as_str() && s.key == key.reference)
        };
        if exists {
            Ok(())
        } else {
            Err(SessionRejection::UnknownSession {
                key: Self::key_str(&key),
            })
        }
    }

    async fn close(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        if !self.reachable.load(std::sync::atomic::Ordering::SeqCst) {
            let cause = self
                .unreachable_cause
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default();
            return Err(SessionRejection::Unavailable { cause });
        }
        let mut g = self.inner.write().await;
        let before = g.sessions.len();
        g.sessions
            .retain(|s| !(s.channel == key.channel.as_str() && s.key == key.reference));
        if g.sessions.len() == before {
            return Err(SessionRejection::UnknownSession {
                key: Self::key_str(&key),
            });
        }
        if g.focused.as_ref() == Some(&key) {
            g.focused = None;
        }
        drop(g);
        self.emit(SessionEvent::Removed {
            channel: key.channel.as_str().to_string(),
            key: key.reference,
        });
        Ok(())
    }

    async fn turns(&self, key: ChannelKey, from: u64) -> Result<Vec<TurnEntry>, SessionRejection> {
        let g = self.inner.read().await;
        let Some(sid) = g
            .sessions
            .iter()
            .find(|s| s.channel == key.channel.as_str() && s.key == key.reference)
            .and_then(|s| s.session_id.clone())
        else {
            return Err(SessionRejection::UnknownSession {
                key: Self::key_str(&key),
            });
        };
        Ok(g.transcripts
            .get(&sid)
            .map(|log| log.iter().filter(|e| e.position >= from).cloned().collect())
            .unwrap_or_default())
    }

    async fn reachability(&self) -> Reachability {
        if self.reachable.load(std::sync::atomic::Ordering::SeqCst) {
            Reachability::Reachable
        } else {
            let cause = self
                .unreachable_cause
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "核心不可达".into());
            Reachability::Unreachable { cause }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2.2 验收：in-process 满足 trait 且永远 Reachable。
    #[tokio::test]
    async fn in_process_backend_satisfies_trait_and_is_reachable() {
        let map = sebas_router::SessionMap::new();
        let (router, _rx) = sebas_router::RouterHandle::new(map);
        let backend = InProcessBackend::new(router);
        assert_eq!(backend.reachability().await, Reachability::Reachable);
        assert!(backend.snapshot().await.is_empty());
        assert!(backend.focused().await.is_none());
    }

    // 2.3 验收：fake 能驱动每个 trait 方法（无子进程 / socket）。
    #[tokio::test]
    async fn fake_backend_drives_every_trait_method() {
        let backend = FakeBackend::new();
        let mut events = backend.subscribe();

        // spawn → visible + Created event.
        let key = backend.spawn("hi".into(), None).await.unwrap();
        assert_eq!(backend.snapshot().await.len(), 1);
        assert!(matches!(
            events.try_recv(),
            Ok(SessionEvent::Created { .. })
        ));

        // message/close on the key work; unknown keys are rejected.
        assert!(backend.message(key.clone(), "yo".into()).await.is_ok());
        let bogus = ChannelKey::new("web", "nope");
        assert_eq!(
            backend.message(bogus.clone(), "yo".into()).await,
            Err(SessionRejection::UnknownSession {
                key: FakeBackend::key_str(&bogus)
            })
        );

        // focus round-trip.
        backend.set_focus(Some(key.clone())).await;
        assert_eq!(backend.focused().await, Some(key.clone()));

        // turns: unknown → rejection; pushed entries filter by position.
        assert!(backend.turns(key.clone(), 0).await.is_err());
        // 给它一个 session_id 再推 transcript。
        backend
            .set_sessions(vec![SessionInfo {
                channel: key.channel.as_str().to_string(),
                key: key.reference.clone(),
                session_id: Some("s9".into()),
                status: "active".into(),
                phase: None,
                user_prompt: None,
                last_active_unix: 0,
                project_dir: None,
                current_model: None,
                available_models: None,
                agent_kind: None,
            }])
            .await;
        backend.push_turn("s9", "prompt", "p1").await;
        backend.push_turn("s9", "content", "c1").await;
        let tail = backend.turns(key.clone(), 1).await.unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].position, 1);
        assert_eq!(tail[0].content, "c1");

        // close works and emits Removed.
        assert!(backend.close(key.clone()).await.is_ok());
        assert!(matches!(events.try_recv(), Ok(SessionEvent::Removed { .. })));
        assert!(backend.snapshot().await.is_empty());

        // unreachable mode reports the cause through every mutating path.
        backend.set_reachable(false, "socket absent");
        assert_eq!(
            backend.reachability().await,
            Reachability::Unreachable {
                cause: "socket absent".into()
            }
        );
        assert!(matches!(
            backend.spawn("x".into(), None).await,
            Err(SessionRejection::Unavailable { .. })
        ));
        assert!(matches!(
            backend.message(key, "x".into()).await,
            Err(SessionRejection::Unavailable { .. })
        ));
    }
}
