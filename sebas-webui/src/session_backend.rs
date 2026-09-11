//! The session backend seam (openspec/changes/add-core-session-channel — task 2.1).
//!
//! Mirrors the `AdminAdapter` seam: the webui crate owns the trait, the sebas
//! binary crate supplies implementations (in-process over `DispatchHandle`, or
//! the core session channel socket client). The webui crate never depends on
//! the binary crate — that is the seam's whole point.
//!
//! Everything the session routes need flows through this trait: reads
//! (snapshot/turns/focus), mutations (spawn/message/close), the event
//! subscription for SSE, and the reachability report that drives honest
//! degradation rendering when the core is not connected.

use async_trait::async_trait;
use sebas_channels::key::ChannelKey;
use sebas_dispatch::{PendingSubmission, SessionEvent, SessionInfo, TurnEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
/// Whether the backend can currently reach the session authority (the core),
/// and if not, why — rendered verbatim so degradation is honest.
///
/// cover-core-channel-test-gaps A1.1（design D1）：不可达拆成三变体而非单一
/// `Unreachable`——枚举逼着每个消费点穷举三态，前端/`/api/summary` 据此输出
/// 机器可读的 `reachability.kind`（startup_failed | auth_rejected |
/// disconnected），banner 文案按 kind 区分，不靠 cause 文案字符串匹配。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reachability {
    /// The core is reachable; session controls are live.
    Reachable,
    /// The core never came up: the channel socket is absent (core never
    /// started or its startup attempt failed). cause keeps the fail-fast
    /// enriched full string (`core startup failed: <原因>` when the latch
    /// file has one).
    StartupFailed { cause: String },
    /// The socket is there but the channel handshake was rejected (secret
    /// mismatch). The client does not retry the same secret forever — it
    /// re-reads the secret source before the next attempt.
    AuthRejected { cause: String },
    /// The connection worked (or the socket existed) but the core is gone
    /// now: refused connect, post-handshake drop, or a timed-out request.
    Disconnected { cause: String },
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
    /// The targeted execution backend cannot serve the request even though
    /// the core is reachable (e.g. native without provider credentials), or
    /// the caller named a backend hint the core does not know.
    BackendUnavailable { backend: String, cause: String },
    /// （workbench-turn-queue 5.1，design D5）spawn 窗口 staging 队列已满：
    /// 提交未被接受、也不顶掉已暂存条目。携带上限，提交面据此可见拒绝。
    QueueFull { limit: usize },
    /// （workbench-turn-queue D7）pending submission 管理操作的类型化拒绝。
    PendingRejected { reason: PendingReason },
}

/// pending submission 管理拒绝的具体原因（workbench-turn-queue D7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingReason {
    /// id 不在待执行栈里，也从未开始过。
    Unknown,
    /// 该提交已经开轮（或已被激活合并）——绝不回滚在跑的回合。
    AlreadyStarted,
    /// 不能把普通提交移到优先（/btw）提交之前。
    PriorityConflict,
    /// 目标位置超出该处置组的范围。
    OutOfRange,
}

impl std::fmt::Display for PendingReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PendingReason::Unknown => write!(f, "待执行提交不存在"),
            PendingReason::AlreadyStarted => write!(f, "该提交已开始执行"),
            PendingReason::PriorityConflict => write!(f, "不能越过优先提交排序"),
            PendingReason::OutOfRange => write!(f, "目标位置越界"),
        }
    }
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
            SessionRejection::BackendUnavailable { backend, cause } => {
                write!(f, "执行体不可用: {backend} — {cause}")
            }
            SessionRejection::QueueFull { limit } => {
                write!(f, "待执行队列已满（上限 {limit}）：这条消息没有提交")
            }
            SessionRejection::PendingRejected { reason } => write!(f, "{reason}"),
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

/// （workbench-turn-queue 5.2）关闭会话的结果：`discarded_pending` = 随之
/// 丢弃的未执行待生效提交条数（close 响应携带，绝不静默丢队）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseReport {
    pub discarded_pending: usize,
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

    /// （extract-im-service 2.1）ensure 语义的消息投递：未知 key 自动建会话、
    /// dormant 会话懒复活（IM 前端的聊天式投递）。缺省退化为普通 `message`
    /// ——未知 key 被执行体拒绝；具备建会话语义的执行体覆写本方法。
    async fn ensure_message(
        &self,
        key: ChannelKey,
        message: String,
    ) -> Result<(), SessionRejection> {
        self.message(key, message).await
    }

    /// （extract-im-service 2.2）取消会话在飞 turn；会话保留、可继续对话。
    /// 缺省拒绝——不支持取消的执行体如实上报，不伪装成功。
    async fn cancel(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        Err(SessionRejection::UnknownSession {
            key: serde_json::to_string(&key).unwrap_or_default(),
        })
    }

    /// Close a session (kills the live child when there is one). The report
    /// names how many pending submissions were discarded with it
    /// (workbench-turn-queue 5.2 — a close never drops a queue silently).
    async fn close(&self, key: ChannelKey) -> Result<CloseReport, SessionRejection>;

    /// （workbench-turn-queue D6/D7）会话的待生效提交全量视图（投递序）。
    /// 无队列的后端返回空列表——native 会话没有待执行栈。
    async fn pending(&self, _key: ChannelKey) -> Result<Vec<PendingSubmission>, SessionRejection> {
        Ok(Vec::new())
    }

    /// （workbench-turn-queue D7）按 id 移除一个未开始的提交，成功返回操作
    /// 后的全量 pending（客户端据此对账）。拒绝类型化：未知 id / 已开始 /
    /// 越过优先项 / 越界。缺省诚实不可用——不承载队列的后端没有管理面。
    async fn remove_pending(
        &self,
        _key: ChannelKey,
        _pending_id: u64,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        Err(SessionRejection::Unavailable {
            cause: "此后端不承载待执行队列".into(),
        })
    }

    /// （workbench-turn-queue D7）把一个未开始的提交重排到其处置组内的
    /// `to_index` 位置，成功返回操作后的全量 pending。
    async fn move_pending(
        &self,
        _key: ChannelKey,
        _pending_id: u64,
        _to_index: usize,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        Err(SessionRejection::Unavailable {
            cause: "此后端不承载待执行队列".into(),
        })
    }

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

    /// Create a session pinned to `agent`（workbench-agent-wire-fix D2：
    /// `[acp.agents.*]` 配置键名或 `"native"`；必填，无隐式默认）。`model`
    /// （add-acp-model-selection）是创建时请求的模型 id：会话建立后、首个
    /// prompt 前应用（失败报非致命错误、会话仍可对话）。默认实现忽略
    /// agent/model（单后端 seams 无选择面，落到自身唯一执行体）。
    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        _agent: &str,
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

    /// （add-fetch-models）providers 域抓取 op：按 provider 名解析 base url
    /// 与密钥，core 侧执行一次只读 GET 上游 `/models`，返回 id 列表。抓取不
    /// 改任何字段、不持久化；错误串 = `fetch_models: ` 前缀 + 净化原因。默认
    /// 实现诚实不可用——不承载 core providers 域的后端没有抓取能力。
    async fn fetch_provider_models(&self, _provider: &str) -> Result<Vec<String>, String> {
        Err("模型列表抓取不可用：core providers 域未由此后端承载".into())
    }

    /// Create a 0-turn placeholder session without spawning an agent child
    /// (P2 fix: an empty prompt must not be sent to the agent — opencode
    /// hangs on `session/prompt ""`). `agent` is the same agent id as
    /// [`SessionBackend::spawn_with`]（必填），and `model` the requested
    /// model id; both are remembered for the first message's spawn. The
    /// default falls back to `spawn("", …)` for backends without placeholder
    /// support (keeps the old callable surface honest).
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        _agent: &str,
        _model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        self.spawn(String::new(), project_dir).await
    }
}

// ─── In-process implementation (task 2.2) ──────────────────────────────────

/// The agent id IS the kind（workbench-agent-wire-fix D2）：wire 直接携带
/// `[acp.agents.*]` 配置键名——无 driver 前缀、无命名空间。`"native"` 到
/// 不了这里（复合后端先行路由）。
fn agent_kind_of(agent: &str) -> String {
    agent.to_string()
}

/// 代码内置 preset 表的 JSON 形状（make-core-own-provider-data 1.3；与 core
/// channel 的 presets 域 / router `/admin/presets` 同一 wire：name + 三槽位 +
/// api_key_env + models）。preset 数据跟随代码，只读、无存储副本。models 是
/// 条目列表（id + 能力标记；redesign-provider-models-settings 1.2）。
fn preset_table_value() -> serde_json::Value {
    let out: Vec<serde_json::Value> = sebas_router::config::presets()
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "base_url_anthropic": p.base_url_anthropic,
                "base_url_openai_chat": p.base_url_openai_chat,
                "base_url_openai_responses": p.base_url_openai_responses,
                "api_key_env": p.api_key_env,
                "models": p.models.iter().map(sebas_router::config::PresetModel::to_entry).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({ "presets": out })
}

/// In-process backend over the router. Used by `sebas run --webui`, where the
/// webui lives in the same process as the session authority.
pub struct InProcessBackend {
    router: sebas_dispatch::DispatchHandle,
    /// Review-card notices relayed from the router's ACP permission broadcast.
    notices: broadcast::Sender<PermissionNotice>,
    /// `request_id` → routing `session_id`, recorded when a PermissionRequest
    /// is relayed, so `answer_permission` can route the reply back to the
    /// owning session without the caller knowing the session id.
    request_sessions: Arc<RwLock<HashMap<String, String>>>,
}

impl InProcessBackend {
    pub fn new(router: sebas_dispatch::DispatchHandle) -> Self {
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

/// dispatch 层的类型化拒绝 → wire 上的 `SessionRejection`（workbench-turn-queue
/// D7）。会话不存在与 id 未知同形（404 PendingRejected::Unknown）。
fn pending_op_rejection(e: sebas_dispatch::PendingOpError) -> SessionRejection {
    let reason = match e {
        sebas_dispatch::PendingOpError::Unknown => crate::session_backend::PendingReason::Unknown,
        sebas_dispatch::PendingOpError::AlreadyStarted => {
            crate::session_backend::PendingReason::AlreadyStarted
        }
        sebas_dispatch::PendingOpError::PriorityConflict => {
            crate::session_backend::PendingReason::PriorityConflict
        }
        sebas_dispatch::PendingOpError::OutOfRange => {
            crate::session_backend::PendingReason::OutOfRange
        }
    };
    SessionRejection::PendingRejected { reason }
}

/// `PermissionDecision` → ACP `Decision`（design D6/R5）。ACP 侧没有 escalate
/// 等价，`Escalate` 降级为 `AllowOnce`（reason 丢弃，记为已知取舍）。
fn map_permission_decision(d: PermissionDecision) -> sebas_acp::Decision {
    match d {
        PermissionDecision::AllowOnce => sebas_acp::Decision::AllowOnce,
        PermissionDecision::AllowSession => sebas_acp::Decision::AllowSession,
        PermissionDecision::Deny => sebas_acp::Decision::Deny,
        PermissionDecision::Escalate { reason } => {
            tracing::warn!(%reason, "ACP has no escalate equivalent, falling back to AllowOnce");
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
        // fail-fast-on-startup-errors（webui spec delta）：spawn 失败不再
        // "surface as a Removed event later"——dispatch 立即把错误事件推进
        // transcript、会话标记 spawn-failed 并发布 Updated；Removed 只作为
        // 后续显式关闭等状态变更的次要信号。
        Ok(self.router.web_spawn(prompt, project_dir, None, None).await)
    }

    /// Spawn through the router with the agent kind pinned（D2：agent id 即
    /// kind）and the requested model id threaded to the spawn out（D3：建会
    /// 话后、首 prompt 前应用）。
    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        agent: &str,
        model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        let kind = agent_kind_of(agent);
        Ok(self
            .router
            .web_spawn(prompt, project_dir, Some(kind), model)
            .await)
    }

    /// 0-turn placeholder: create the session row without spawning an agent
    /// child (P2 fix). The requested kind/model are remembered on the mapping
    /// so the first message spawns the right agent.
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        agent: &str,
        model: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        let kind = agent_kind_of(agent);
        Ok(self
            .router
            .web_create_placeholder(project_dir, Some(kind), model)
            .await)
    }

    async fn set_session_model(
        &self,
        key: ChannelKey,
        model_id: String,
    ) -> Result<(), SessionRejection> {
        // 解析路由 session_id（web 会话的 chat_id 是 web-* 键，不是 ACP
        // routing id），再经 Out::SendAcp 送达 SetModel。
        let Some(sid) = self
            .router
            .map
            .get(&key)
            .await
            .and_then(|m| m.session_id().map(str::to_owned))
        else {
            return Err(SessionRejection::UnknownSession {
                key: key.reference.clone(),
            });
        };
        self.router
            .emit(sebas_dispatch::Out::SendAcp {
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
        // channel server, which pre-checks existence. The staging-queue
        // overflow (workbench-turn-queue 5.1) surfaces as a typed QueueFull.
        self.router
            .web_send_message(key, message)
            .await
            .map_err(|cap_full| SessionRejection::QueueFull {
                limit: cap_full.cap,
            })
    }

    async fn close(&self, key: ChannelKey) -> Result<CloseReport, SessionRejection> {
        match self.router.web_close_session(key).await {
            sebas_dispatch::engine::CloseOutcome::Closed { discarded_pending } => {
                Ok(CloseReport { discarded_pending })
            }
            sebas_dispatch::engine::CloseOutcome::NotFound => {
                Err(SessionRejection::UnknownSession { key: String::new() })
            }
        }
    }

    async fn pending(&self, key: ChannelKey) -> Result<Vec<PendingSubmission>, SessionRejection> {
        Ok(self.router.session_pending(&key).await)
    }

    async fn remove_pending(
        &self,
        key: ChannelKey,
        pending_id: u64,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        self.router
            .remove_pending(&key, pending_id)
            .await
            .map_err(pending_op_rejection)
    }

    async fn move_pending(
        &self,
        key: ChannelKey,
        pending_id: u64,
        to_index: usize,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        self.router
            .move_pending(&key, pending_id, to_index)
            .await
            .map_err(pending_op_rejection)
    }

    /// （extract-im-service 2.2）取消在飞 turn：映射存在即发 `AcpCommand::Cancel`。
    async fn cancel(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        if self.router.web_cancel_session(&key).await {
            Ok(())
        } else {
            Err(SessionRejection::UnknownSession {
                key: serde_json::to_string(&key).unwrap_or_default(),
            })
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
        // preset 表是只读代码数据，不依赖状态库（make-core-own-provider-data
        // 1.3）：无论引擎是否初始化都可读。
        if domain == "presets" {
            return Some(preset_table_value());
        }
        // In-process backend: use the engine when available.
        let engine = sebas_dispatch::state_store::engine()?;
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
        // 与 core channel 服务端共用同一组分发实现（make-core-own-provider-data
        // 1.1/1.2/3.1：settings 域含 defaults ops；providers / aliases 域是
        // provider 数据唯一写通道的两个入口，形状逐字相同）。
        let engine = sebas_dispatch::state_store::engine()
            .ok_or_else(|| "state store 未初始化".to_string())?;
        match domain {
            "settings" => sebas_dispatch::state_store::settings_mutation(engine, &payload).await,
            "providers" => sebas_dispatch::state_store::providers_mutation(engine, &payload).await,
            "aliases" => sebas_dispatch::state_store::aliases_mutation(engine, &payload).await,
            "projects" => sebas_dispatch::state_store::project_mutation(engine, &payload).await,
            other => Err(format!("unknown domain: {other}")),
        }
    }

    async fn fetch_provider_models(&self, provider: &str) -> Result<Vec<String>, String> {
        // add-fetch-models：内嵌形态与 core channel 服务端共用同一实现（单一
        // 实现避免两侧漂移，同 state_mutate 的做法）。
        let engine = sebas_dispatch::state_store::engine()
            .ok_or_else(|| "fetch_models: state store 未初始化".to_string())?;
        sebas_dispatch::state_store::providers_fetch_models(engine, provider).await
    }

    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        Some(self.notices.subscribe())
    }

    async fn answer_permission(&self, request_id: &str, decision: PermissionDecision) -> bool {
        let session_id = self.request_sessions.read().await.get(request_id).cloned();
        let Some(session_id) = session_id else {
            return false;
        };
        // 原生会话（make-feishu-optional-webui-primary）：权限请求来自桥 →
        // 决定回填到原生内核（ApproverHub）。先试 native，失败再回退 acp。
        let native = match decision.clone() {
            PermissionDecision::AllowOnce => {
                sebas_dispatch::native_bridge::NativeApprovalDecision::AllowOnce
            }
            PermissionDecision::AllowSession => {
                sebas_dispatch::native_bridge::NativeApprovalDecision::AllowSession
            }
            PermissionDecision::Deny => sebas_dispatch::native_bridge::NativeApprovalDecision::Deny,
            PermissionDecision::Escalate { reason } => {
                sebas_dispatch::native_bridge::NativeApprovalDecision::Escalate { reason }
            }
        };
        if self
            .router
            .answer_native_permission(request_id, native)
            .await
        {
            return true;
        }
        // acp 会话：走既有 Out::SendAcp PermissionReply。
        let decision = map_permission_decision(decision);
        self.router
            .emit(sebas_dispatch::Out::SendAcp {
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
    /// fix-webui-detached-status：可设置的 state 域快照（route 层测试用）。
    /// 域缺省 = state_snapshot 对该域返回 None（真源不可达）。
    state_domains: std::sync::Mutex<HashMap<String, Option<serde_json::Value>>>,
    /// （wire-webui-sebas-agent-e2e 3.1）可注入的逐执行体可用性（route 层
    /// 测试用）。`None` = 后端不区分执行体（summary 透传 null）。
    execution_bodies: std::sync::Mutex<Option<Vec<ExecutionBodyStatus>>>,
    /// harden-core-channel-deployment 4.2（端点测试用）：`state_mutate` 是否
    /// 成功。默认 false（真源不可达 → 降级路径）；置 true 模拟状态库可用。
    state_mutate_ok: std::sync::atomic::AtomicBool,
    /// add-fetch-models（route 层测试用）：按 provider 名注入的抓取结果。
    /// 未注入的名字走 trait 默认（诚实不可用）。
    fetch_models_results: std::sync::Mutex<HashMap<String, Result<Vec<String>, String>>>,
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
            state_domains: std::sync::Mutex::new(HashMap::new()),
            execution_bodies: std::sync::Mutex::new(None),
            state_mutate_ok: std::sync::atomic::AtomicBool::new(false),
            fetch_models_results: std::sync::Mutex::new(HashMap::new()),
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

    /// fix-webui-detached-status：注入某个 state 域的快照（None = 该域
    /// 不可用），供 route 层测试 settings 的 provider 真源行为。
    pub fn set_state_domain(&self, domain: &str, payload: Option<serde_json::Value>) {
        self.state_domains
            .lock()
            .expect("state domain lock")
            .insert(domain.to_string(), payload);
    }

    /// Flip reachability; `cause` is reported while unreachable.
    pub fn set_reachable(&self, reachable: bool, cause: &str) {
        self.reachable
            .store(reachable, std::sync::atomic::Ordering::SeqCst);
        *self.unreachable_cause.lock().unwrap() = Some(cause.to_string());
    }

    /// harden-core-channel-deployment 4.2：翻转 `state_mutate` 的成败（端点
    /// 测试降级标记用）。默认 false = 真源不可达。
    pub fn set_state_mutate_ok(&self, ok: bool) {
        self.state_mutate_ok
            .store(ok, std::sync::atomic::Ordering::SeqCst);
    }

    /// add-fetch-models：注入某个 provider 的抓取结果（route 层测试用）。
    pub fn set_fetch_models_result(&self, provider: &str, result: Result<Vec<String>, String>) {
        self.fetch_models_results
            .lock()
            .expect("fetch models results lock")
            .insert(provider.to_string(), result);
    }

    /// （wire-webui-sebas-agent-e2e 3.1）注入逐执行体可用性，summary 原样
    /// 透传；`None` = 后端不区分执行体。
    pub fn set_execution_bodies(&self, bodies: Option<Vec<ExecutionBodyStatus>>) {
        *self.execution_bodies.lock().expect("execution bodies lock") = bodies;
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
            usage: None,
            backend: None,
            pending: Vec::new(),
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

    async fn close(&self, key: ChannelKey) -> Result<CloseReport, SessionRejection> {
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
        // Fake 会话不建模待执行栈的丢弃计数（route 层测试只关心 close 语义）。
        Ok(CloseReport::default())
    }

    async fn pending(&self, key: ChannelKey) -> Result<Vec<PendingSubmission>, SessionRejection> {
        let g = self.inner.read().await;
        Ok(g.sessions
            .iter()
            .find(|s| s.channel == key.channel.as_str() && s.key == key.reference)
            .map(|s| s.pending.clone())
            .unwrap_or_default())
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

    async fn state_snapshot(&self, domain: &str) -> Option<serde_json::Value> {
        self.state_domains
            .lock()
            .expect("state domain lock")
            .get(domain)
            .cloned()
            .flatten()
    }

    async fn state_mutate(&self, domain: &str, payload: serde_json::Value) -> Result<(), String> {
        let _ = (domain, payload);
        if self
            .state_mutate_ok
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(());
        }
        Err("state store 不可用".into())
    }

    async fn fetch_provider_models(&self, provider: &str) -> Result<Vec<String>, String> {
        self.fetch_models_results
            .lock()
            .expect("fetch models results lock")
            .get(provider)
            .cloned()
            .unwrap_or_else(|| Err("fetch_models: core providers 域未由此后端承载".into()))
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
            // The fake models the generic runtime-down shape (A1.1: the
            // startup-failed / auth-rejected discrimination lives in the
            // channel backend; route tests only need an unreachable state).
            Reachability::Disconnected { cause }
        }
    }

    async fn execution_bodies(&self) -> Option<Vec<ExecutionBodyStatus>> {
        self.execution_bodies
            .lock()
            .expect("execution bodies lock")
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2.2 验收：in-process 满足 trait 且永远 Reachable。
    #[tokio::test]
    async fn in_process_backend_satisfies_trait_and_is_reachable() {
        let map = sebas_dispatch::SessionMap::new();
        let (router, _rx) = sebas_dispatch::DispatchHandle::new(map);
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
                usage: None,
                backend: None,
                pending: Vec::new(),
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
        assert!(matches!(
            events.try_recv(),
            Ok(SessionEvent::Removed { .. })
        ));
        assert!(backend.snapshot().await.is_empty());

        // unreachable mode reports the cause through every mutating path.
        backend.set_reachable(false, "socket absent");
        assert_eq!(
            backend.reachability().await,
            Reachability::Disconnected {
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

    // fix-webui-detached-status 1.1：新变体的 serde 往返与 Display，
    // 既有变体的 wire 形状不因新增而变化。
    #[test]
    fn backend_unavailable_rejection_serializes_round_trip() {
        let r = SessionRejection::BackendUnavailable {
            backend: "native".into(),
            cause: "needs SEBAS_AGENT_PROVIDER_API_KEY".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(
            json,
            r#"{"code":"backend_unavailable","backend":"native","cause":"needs SEBAS_AGENT_PROVIDER_API_KEY"}"#
        );
        let back: SessionRejection = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        assert_eq!(
            r.to_string(),
            "执行体不可用: native — needs SEBAS_AGENT_PROVIDER_API_KEY"
        );
    }

    #[test]
    fn legacy_rejection_wire_shapes_unchanged() {
        // 旧报文仍可解码；code 名保持 snake_case 稳定。
        for (json, expect) in [
            (
                r#"{"code":"unknown_session","key":"web-legacy-k"}"#,
                "unknown",
            ),
            (r#"{"code":"unusable_project_dir"}"#, "dir"),
            (r#"{"code":"capacity","limit":3}"#, "cap"),
            (r#"{"code":"unavailable","cause":"socket gone"}"#, "unavail"),
        ] {
            let r: SessionRejection = serde_json::from_str(json).unwrap();
            match expect {
                "unknown" => assert!(matches!(r, SessionRejection::UnknownSession { .. })),
                "dir" => assert_eq!(r, SessionRejection::UnusableProjectDir),
                "cap" => assert!(matches!(r, SessionRejection::Capacity { limit: 3 })),
                _ => assert!(matches!(r, SessionRejection::Unavailable { .. })),
            }
        }
    }
}
