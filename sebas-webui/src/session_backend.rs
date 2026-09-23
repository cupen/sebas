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
use sebas_dispatch::engine::CancelOutcome;
use sebas_dispatch::{
    PendingApproval, PendingSubmission, SessionEvent, SessionInfo, SessionIdentity, TurnEntry,
    TurnStreamEvent,
};
use serde::{Deserialize, Serialize};

// 会话后端的**wire 词表**已迁往 `sebas_domain::session`（add-domain-layer
// 3.2，design D3 原位再导出）：`SessionRejection` / `PendingReason` /
// `PermissionNotice` / `PermissionDecision` 的既有路径零改动。
// `SessionBackend` trait 与 `Reachability` 是本 crate 的缝，不迁。
pub use sebas_domain::session::{
    PermissionDecision, PermissionNotice, PendingReason, SessionRejection,
};
pub use sebas_domain::node::NodeView as NodeInfo;
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

/// （workbench-turn-queue 5.2）关闭会话的结果：`discarded_pending` = 随之
/// 丢弃的未执行待生效提交条数（close 响应携带，绝不静默丢队）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseReport {
    pub discarded_pending: usize,
}

// 一个执行节点的管理面视图：与 core 通道的 `NodeView` 已**合一**为
// `sebas_domain::node::NodeView`（add-domain-layer 4.1）——`local` 是共享
// 类型上的独立字段（只在 true 时上 wire），`NodeInfo` 路径经再导出保持
// 既有调用点零改动。

/// 节点侧路径判定结果（节点 `SessionOp::CheckPath { path }` 的应答形状：
/// `SessionResult::PathChecked { exists, is_dir, within_workspace }`）。
///
/// 路径可用性由**项目命名的那台机器**判定，主控不替远端做本地 `stat`。
/// `within_workspace`（add-workspace-root）是节点以它自己的 workspace root 做
/// 的 containment 判定；远端注册据此拒绝越界路径（缺省 `true` = 旧节点应答
/// 视为界内）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathCheck {
    pub exists: bool,
    pub is_dir: bool,
    #[serde(default = "default_true")]
    pub within_workspace: bool,
}

/// `PathCheck::within_workspace` 的 serde 缺省：缺字段 = 界内（兼容旧应答）。
fn default_true() -> bool {
    true
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

    /// 聚焦即拉起（workbench-live-conversation-flow 3.1）：无 prompt 拉起
    /// 会话子进程（占位 fresh / Dormant resume）。幂等：已活/在途 →
    /// `Ok(false)`。默认实现 = 无子进程概念的后端如实回报无事可做。
    async fn activate(&self, _key: ChannelKey) -> Result<bool, SessionRejection> {
        Ok(false)
    }

    /// Subscribe to live turn-content events（workbench-live-conversation-flow
    /// 1.1）：transcript 每批追加一条事件，携带 `(channel, key)` 与按落库序
    /// 排列的条目。Lagged 是建议性的（内容可从快照恢复），消费端不得因此
    /// 断链。默认实现返回一个立即关闭的接收端——不承载流的后端据此让消费
    /// 方进入「无流」旁路（消费端对 Closed 的处置是停用该支路，不是报错）。
    fn subscribe_turn_events(&self) -> broadcast::Receiver<TurnStreamEvent> {
        let (_tx, rx) = broadcast::channel(1);
        rx
    }

    /// （add-core-reachability-ws-push D1）核心可达性**翻转**的独立广播通道
    /// （`subscribe_turn_events` 先例）：只有状态真变才发布（D2，发布点在
    /// channel 后端的 `set_status()` 收口），可达性本身是全量状态——Lagged
    /// 丢帧无一致性代价，下一帧或客户端重连后的 get 收敛。
    ///
    /// 默认实现返回一个立即关闭的接收端：不承载翻转源的后端（in-process
    /// 与 core 同进程同生共死、fake 后端、native/dual 复合后端）据此让
    /// 消费方停用该支路（对 Closed 的处置是停用，不是报错）。
    fn reachability_updates(&self) -> broadcast::Receiver<Reachability> {
        let (_tx, rx) = broadcast::channel(1);
        rx
    }

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

    /// 已注册执行节点的可用性（add-remote-execution-node 8.x）。真源是 core 的
    /// 节点注册表（`NodeLinkOp::ListNodes`），**不另造一份**。
    ///
    /// 默认诚实不可用：不承载注册表的后端不能假装知道远端节点状态。前端据此
    /// 显示「节点状态不可得」，而不是把「看不见」说成「没有节点」。
    async fn nodes(&self) -> Result<Vec<NodeInfo>, String> {
        Err("此后端不承载节点注册表（节点可用性不可得）".into())
    }

    /// 请**节点自己**判定一个路径是否可用（`SessionOp::CheckPath { path }`
    /// → `SessionResult::PathChecked { exists, is_dir, within_workspace }`）。
    /// 远端项目注册前必须走这里：主控在这台机器上无从判断那台机器上的路径。
    ///
    /// 默认诚实不可用——绝不回退成本地 `stat`（那会把「主控上恰好同名」误当成
    /// 「节点上存在」）。
    async fn check_node_path(&self, _node_id: &str, _path: &str) -> Result<PathCheck, String> {
        Err("此后端不能向节点发起路径校验".into())
    }

    /// Live stream of gated tool calls awaiting a decision (the review-card
    /// feed). `None` = this backend has no permission interaction (its
    /// sessions never gate, or gating is surfaced elsewhere).
    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        None
    }

    /// 待批请求读模型（fix-webui-approval-restore-and-session-identity 1.2）：
    /// 按会话枚举当前泊车审批（request_id / 工具 / 参数），与推送通道独立
    /// ——WebUI 打开/刷新会话时拉取它重建审批面。未知会话 → typed rejection
    /// （路由转 404）；无泊车 = 空表。默认诚实不可用：不承载泊车登记的后端
    /// 不假装知道。
    async fn pending_approvals(
        &self,
        _key: ChannelKey,
    ) -> Result<Vec<PendingApproval>, SessionRejection> {
        Err(SessionRejection::Unavailable {
            cause: "此后端不承载泊车审批读模型".into(),
        })
    }

    /// 设置/清空会话 label（fix-webui-approval-restore-and-session-identity
    /// 5.1，design D6）。`None` = 清空。未知会话 → typed rejection。
    async fn set_session_label(
        &self,
        _key: ChannelKey,
        _label: Option<String>,
    ) -> Result<(), SessionRejection> {
        Err(SessionRejection::Unavailable {
            cause: "此后端不支持会话命名".into(),
        })
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
    /// prompt 前应用（失败报非致命错误、会话仍可对话）。
    ///
    /// `node`（add-remote-execution-node 8.1）是项目所属执行节点：`Some(id)`
    /// （本机项目为 `Some("local")`）请核心把会话放到该节点；`None` = 无项目
    /// 会话（飞书来源），由核心落到配置的默认执行节点。默认实现忽略节点参数
    /// 落到本地执行体——单后端 seams 没有远端放置面，如实降级由各实现负责。
    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        _agent: &str,
        _model: Option<String>,
        _mode: Option<String>,
        _node: Option<String>,
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
    /// model id; both are remembered for the first message's spawn. `node`
    /// the same execution-node dimension as `spawn_with`. The default falls
    /// back to `spawn("", …)` for backends without placeholder support
    /// (keeps the old callable surface honest).
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        _agent: &str,
        _model: Option<String>,
        _mode: Option<String>,
        _node: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        self.spawn(String::new(), project_dir).await
    }

    /// （add-agent-mode-selection）中程切换会话权限模式（控制面词汇
    /// `ask`/`edit`/`allow`/`auto`）。成功 = 命令已进入会话通道；执行体
    /// 接受与否经事件流反馈（`ModeChanged` = 成功，非终态 `Error` = 拒绝、
    /// mode 不变）——与 [`SessionBackend::set_session_model`] 同一契约。
    /// 默认实现诚实不可用。
    async fn set_session_mode(
        &self,
        _key: ChannelKey,
        _mode: String,
    ) -> Result<(), SessionRejection> {
        Err(SessionRejection::Unavailable {
            cause: "此后端不支持会话级模式切换".into(),
        })
    }

    /// 从归档条目重建会话（fix-webui-qa-defects 2.2，design D1）：以原 key
    /// 重建 Dormant 映射（`session_id` / `project_dir` 沿用）并把归档转写
    /// 回放进 turn 存储。成功后快照/detail 立即可见该会话；失败时调用方
    /// 必须保留归档条目（「消费归档 ⇄ 重建会话」同事务语义——绝不删了
    /// 归档却没重建会话）。`label` / `prompt_preview`（round4 3.1）= 归档
    /// 时刻的命名来源，随恢复迁回映射。默认诚实不可用。
    async fn restore_session(
        &self,
        _key: ChannelKey,
        _session_id: Option<String>,
        _project_dir: Option<String>,
        _transcript: Vec<TurnEntry>,
        _identity: SessionIdentity,
        _label: Option<String>,
        _prompt_preview: Option<String>,
    ) -> Result<(), SessionRejection> {
        Err(SessionRejection::Unavailable {
            cause: "此后端不支持从归档重建会话".into(),
        })
    }
}

// ─── In-process implementation (task 2.2) ──────────────────────────────────

/// The agent id IS the kind（workbench-agent-wire-fix D2）：wire 直接携带
/// `[acp.agents.*]` 配置键名——无 driver 前缀、无命名空间。`"native"` 到
/// 不了这里（复合后端先行路由）。
fn agent_kind_of(agent: &str) -> String {
    agent.to_string()
}

/// `Some(节点)` 且不是本机 → 远端节点 id；`None` / `"local"` / 空白 → `None`
/// （本机放置，行为不变）。8.1：`"local"` 必须与本机同义，否则本机项目会被
/// 当成远端项目而被拒。
fn remote_node_of(node: Option<&str>) -> Option<&str> {
    node.map(str::trim)
        .filter(|n| !n.is_empty() && *n != crate::projects::LOCAL_NODE_ID)
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

    fn subscribe_turn_events(&self) -> broadcast::Receiver<TurnStreamEvent> {
        self.router.subscribe_turn_events()
    }

    async fn activate(&self, key: ChannelKey) -> Result<bool, SessionRejection> {
        self.router
            .web_activate_session(key)
            .await
            .map_err(|e| SessionRejection::Unavailable {
                cause: format!("activate failed: {e}"),
            })
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
        Ok(self
            .router
            .web_spawn(prompt, project_dir, None, None, None)
            .await)
    }

    /// Spawn through the router with the agent kind pinned（D2：agent id 即
    /// kind）and the requested model id threaded to the spawn out（D3：建会话
    /// 后、首 prompt 前应用）。
    ///
    /// `node`（8.1）：进程内后端调的是 `DispatchHandle::web_spawn`，它没有
    /// 节点维度（远端放置走 `CoreChannelRequest::Spawn { node }`）。远端节点
    /// 在这里**如实拒绝**，绝不静默落到本机——静默本地执行会把「我以为跑在
    /// 那台机器上」变成一条看不出来的谎。
    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        agent: &str,
        model: Option<String>,
        mode: Option<String>,
        node: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        if let Some(remote) = remote_node_of(node.as_deref()) {
            return Err(SessionRejection::Unavailable {
                cause: format!("进程内后端不承载远端会话放置（节点 {remote}）"),
            });
        }
        let kind = agent_kind_of(agent);
        Ok(self
            .router
            .web_spawn(prompt, project_dir, Some(kind), model, mode)
            .await)
    }

    /// 0-turn placeholder: create the session row without spawning an agent
    /// child (P2 fix). The requested kind/model are remembered on the mapping
    /// so the first message spawns the right agent. 远端节点同上如实拒绝。
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        agent: &str,
        model: Option<String>,
        mode: Option<String>,
        node: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        if let Some(remote) = remote_node_of(node.as_deref()) {
            return Err(SessionRejection::Unavailable {
                cause: format!("远端会话不能建 0-turn 占位（节点 {remote}）：请直接发送第一条输入"),
            });
        }
        let kind = agent_kind_of(agent);
        Ok(self
            .router
            .web_create_placeholder(project_dir, Some(kind), model, mode)
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

    /// （add-agent-mode-selection）解析路由 session_id 后经 `Out::SendAcp`
    /// 送达 `SetMode`——claude 驱动在运行时经 SDK `set_permission_mode`
    /// 切换；接受与否经事件流反馈（`ModeChanged` / 非终态 `Error`）。
    async fn set_session_mode(
        &self,
        key: ChannelKey,
        mode: String,
    ) -> Result<(), SessionRejection> {
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
        // 期望值先记在映射上（快照立即反映操作者意图）；effective 由
        // `ModeChanged` 事件落定（engine 的 apply_event 处理）。
        // （3.2，D5b）desired 非空：切换必须给出控制面词之一。
        self.router.map.set_desired_mode(&key, mode.clone()).await;
        self.router
            .emit(sebas_dispatch::Out::SendAcp {
                session_id: sid.clone(),
                cmd: sebas_acp::AcpCommand::SetMode {
                    session_id: sid,
                    mode,
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

    /// （extract-im-service 2.2）取消在飞 turn。workbench-interaction-polish
    /// 1.1：引擎三态判定——在飞派发中断，空闲/未知转 typed 拒绝（不再对
    /// 「无事可取消」回 Ok）。
    async fn cancel(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        match self.router.web_cancel_session(&key).await {
            CancelOutcome::Dispatched => Ok(()),
            CancelOutcome::Idle => Err(SessionRejection::Idle {
                key: serde_json::to_string(&key).unwrap_or_default(),
            }),
            CancelOutcome::Unknown => Err(SessionRejection::UnknownSession {
                key: serde_json::to_string(&key).unwrap_or_default(),
            }),
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

    /// 归档恢复（fix-webui-qa-defects 2.2）：直达引擎的 `web_restore_session`
    /// ——Dormant 重建 + 转写回放 + 命名来源迁移（round4 3.1）。引擎拒绝按
    /// 类型映射为 wire 拒绝。
    async fn restore_session(
        &self,
        key: ChannelKey,
        session_id: Option<String>,
        project_dir: Option<String>,
        transcript: Vec<TurnEntry>,
        identity: SessionIdentity,
        label: Option<String>,
        prompt_preview: Option<String>,
    ) -> Result<(), SessionRejection> {
        self.router
            .web_restore_session(
                key,
                session_id,
                project_dir,
                transcript,
                identity,
                label,
                prompt_preview,
            )
            .await
            .map_err(|e| match e {
                sebas_dispatch::error::DispatchError::Capacity(limit) => {
                    SessionRejection::Capacity { limit }
                }
                other => SessionRejection::Unavailable {
                    cause: format!("restore failed: {other}"),
                },
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

    /// （1.2）读模型直达引擎：泊车登记（工具/参数）按会话枚举。未知会话 →
    /// UnknownSession（路由转 404）；占位（无 session_id）如实返回空表。
    async fn pending_approvals(
        &self,
        key: ChannelKey,
    ) -> Result<Vec<PendingApproval>, SessionRejection> {
        self.router
            .pending_permission_requests(&key)
            .await
            .ok_or(SessionRejection::UnknownSession {
                key: key.reference.clone(),
            })
    }

    /// （5.1）label 设置直达引擎映射；未知 key → UnknownSession。
    async fn set_session_label(
        &self,
        key: ChannelKey,
        label: Option<String>,
    ) -> Result<(), SessionRejection> {
        self.router
            .web_set_session_label(key.clone(), label)
            .await
            .map_err(|_| SessionRejection::UnknownSession {
                key: key.reference.clone(),
            })
    }

    async fn answer_permission(&self, request_id: &str, decision: PermissionDecision) -> bool {
        let session_id = self.request_sessions.read().await.get(request_id).cloned();
        let Some(session_id) = session_id else {
            return false;
        };
        // 原生会话（make-feishu-optional-webui-primary）：权限请求来自桥 →
        // 决定回填到原生内核（ApproverHub）。先试 native，失败再回退 acp。
        // 原生泊车同样登记在引擎泊车表（publish_native_permission），其批复
        // 成功在此解除登记——读模型/fail-closed 对两条执行体同一口径。
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
            self.router.resolve_permission_request(request_id).await;
            return true;
        }
        // acp 会话：走既有 Out::SendAcp PermissionReply。（1.4 / 2.3）
        // fail-closed：只有**当前泊车中**的 request_id 可批复——已批复 /
        // 已随 cancel 释放 / 从未泊车的 id 一律拒绝（返回 false → 路由 404），
        // 绝不复活任何状态。
        if self
            .router
            .permission_parked_session(request_id)
            .await
            .is_none()
        {
            return false;
        }
        let decision = map_permission_decision(decision);
        self.router
            .emit(sebas_dispatch::Out::SendAcp {
                session_id: session_id.clone(),
                cmd: sebas_acp::AcpCommand::PermissionReply {
                    session_id: session_id.clone(),
                    request_id: request_id.to_string(),
                    decision: decision.clone(),
                },
            })
            .await;
        // permission-mode-auto-gate：「Allow session」（「本会话不再询问」）
        // 语义重定义为 放行当前请求 + 会话 mode 切 auto——与 dispatch 飞书
        // 点击路径、飞书卡面同一组合（复用 [`Self::set_session_mode`]：
        // desired_mode 落映射 + `Out::SendAcp(SetMode)`，与 webui 中程切换
        // 同源）。放行已先行回出、不回退；SetMode 发不出（会话映射竞态
        // 消失）只是失去 mode 切换，不影响已回的 allow。成败经事件流回执
        // （`ModeChanged`=成功 / 带「模式未变」标记的非终态 `Error`=失败，
        // engine 的 apply_event 据实翻卡上报，im/前端失败呈现已就绪）。
        // （2.3）批复出站后摘除路由登记——同 id 迟到的第二次批复不再命中
        // request_sessions（且泊车登记已由 emit 单点解除），双保险 fail-closed。
        self.request_sessions.write().await.remove(request_id);
        if matches!(decision, sebas_acp::Decision::AllowSession)
            && let Some(key) = self.router.map.lookup_key_by_session(&session_id).await
        {
            let _ = self
                .set_session_mode(key, sebas_dispatch::engine::AUTO_MODE.to_string())
                .await;
        }
        true
    }
}

// ─── Fake backend for tests (task 2.3) ─────────────────────────────────────

/// migrate-project-registry 6.2：测试用项目条目（规范记录的 wire 形状）。
///
/// 注册表已无文件后端（5.1 删除了文件回退），route 层测试构造「升级前遗留的
/// 越界项目」「远端项目」时不能再往 `projects.json` 里塞——改由这里的条目
/// 直接落进 FakeBackend 的 projects 域。
fn project_entry_json(node_id: &str, path: &str) -> serde_json::Value {
    let name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    serde_json::json!({
        "id": crate::projects::project_id_for_on(node_id, path),
        "path": path,
        "name": name,
        "branch_at": 0,
        "added_at": 0,
        "sort_order": 0,
        "node_id": node_id,
    })
}

/// migrate-project-registry 6.2：projects 域的**内存 mutation 语义**，与
/// `sebas_dispatch::state_store::project_mutation` 对齐（add 按 path 去重、
/// remove 按 path、save/reorder 全量替换）。
///
/// route 层测试要的是「POST 之后列表真的变了」；旧的 no-op 桩在文件后端删除
/// 后会让「注册 → 列表」往返断言失去意义。
fn apply_project_mutation(
    domains: &mut HashMap<String, Option<serde_json::Value>>,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let op = payload
        .get("op")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let slot = domains
        .entry("projects".to_string())
        .or_insert_with(|| Some(serde_json::json!({ "projects": [] })));
    if slot.is_none() {
        *slot = Some(serde_json::json!({ "projects": [] }));
    }
    let list = slot
        .as_mut()
        .and_then(|v| v.get_mut("projects"))
        .and_then(|p| p.as_array_mut())
        .expect("projects array");
    match op {
        "add" => {
            let path = payload
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "add: 缺少 path 字段".to_string())?;
            if list
                .iter()
                .any(|p| p.get("path").and_then(serde_json::Value::as_str) == Some(path))
            {
                return Err(format!("项目已注册: {path}"));
            }
            let node_id = payload
                .get("node_id")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(crate::projects::LOCAL_NODE_ID);
            let name = payload
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let mut entry = project_entry_json(node_id, path);
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("name".into(), serde_json::json!(name));
                obj.insert("sort_order".into(), serde_json::json!(list.len() as i64));
            }
            list.push(entry);
            Ok(())
        }
        "remove" => {
            let path = payload
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "remove: 缺少 path 字段".to_string())?;
            let before = list.len();
            list.retain(|p| p.get("path").and_then(serde_json::Value::as_str) != Some(path));
            if list.len() == before {
                Err(format!("项目不存在: {path}"))
            } else {
                Ok(())
            }
        }
        "save" | "reorder" => {
            *list = payload
                .get("projects")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            Ok(())
        }
        other => Err(format!("未知的 projects 子操作: {other}")),
    }
}

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
    /// add-remote-execution-node 8.x：可注入的节点注册表视图。`None` = 该后端
    /// 不承载节点注册表（`nodes()` 如实回 Err，前端呈现「状态不可得」）。
    nodes: std::sync::Mutex<Option<Vec<NodeInfo>>>,
    /// add-remote-execution-node 8.1：按 `(节点, 路径)` 注入的节点侧判定结果。
    /// 未注入的组合走默认（无法校验）。
    path_checks: std::sync::Mutex<HashMap<(String, String), Result<PathCheck, String>>>,
    /// add-remote-execution-node 8.1（route 层测试用）：最近一次
    /// `spawn_with` / `create_placeholder` 收到的 `node`。`None` = 还没调用过。
    last_spawn_node: std::sync::Mutex<Option<Option<String>>>,
    /// fix-webui-qa-defects 2.2（route 层测试用）：记录 `restore_session`
    /// 调用 `(key, session_id, project_dir, 条目数, 身份, label, prompt_preview)`。
    restores: std::sync::Mutex<
        Vec<(
            ChannelKey,
            Option<String>,
            Option<String>,
            usize,
            SessionIdentity,
            Option<String>,
            Option<String>,
        )>,
    >,
    /// fix-webui-approval-restore-and-session-identity 1.2（route 层测试用）：
    /// 按编码会话键注入的待批审批读模型；未注入的键 = 未知会话拒绝。
    approvals: std::sync::Mutex<HashMap<String, Vec<PendingApproval>>>,
    /// fix-webui-approval-restore-and-session-identity 5.1（route 层测试用）：
    /// 最近一次 `set_session_label` 的入参。
    last_label: std::sync::Mutex<Option<(String, Option<String>)>>,
    /// fix-webui-qa-defects-round5 1.2（route 层测试用）：pending 管理面
    /// （remove/move）的类型化拒绝注入。`None` = 缺省诚实不可用（Unavailable）。
    pending_op_rejection: std::sync::Mutex<Option<PendingReason>>,
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
            nodes: std::sync::Mutex::new(None),
            path_checks: std::sync::Mutex::new(HashMap::new()),
            last_spawn_node: std::sync::Mutex::new(None),
            restores: std::sync::Mutex::new(Vec::new()),
            approvals: std::sync::Mutex::new(HashMap::new()),
            last_label: std::sync::Mutex::new(None),
            pending_op_rejection: std::sync::Mutex::new(None),
        }
    }

    /// fix-webui-approval-restore-and-session-identity 1.2（route 层测试用）：
    /// 注入某会话（编码键）的待批审批读模型。
    pub fn set_pending_approvals(&self, encoded_key: &str, approvals: Vec<PendingApproval>) {
        self.approvals
            .lock()
            .expect("approvals lock")
            .insert(encoded_key.to_string(), approvals);
    }

    /// fix-webui-approval-restore-and-session-identity 5.1（route 层测试用）：
    /// 最近一次 `set_session_label` 收到的 `(编码键, label)`。
    pub fn last_label(&self) -> Option<(String, Option<String>)> {
        self.last_label
            .lock()
            .expect("last label lock")
            .clone()
    }

    /// fix-webui-qa-defects-round5 1.2（route 层测试用）：注入 pending 管理
    /// 面（remove/move）的类型化拒绝；`None` 恢复缺省诚实不可用。
    pub fn set_pending_op_rejection(&self, reason: Option<PendingReason>) {
        *self
            .pending_op_rejection
            .lock()
            .expect("pending op rejection lock") = reason;
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
            // webui 侧自建条目（本地回显等）无结构化标题。
            title: None,
            failure_class: None,
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

    /// migrate-project-registry 6.2（route 层测试用）：把 projects 域置为
    /// 「已接线且可用」的空注册表——POST/GET 走真实状态源，而不是降级路径。
    ///
    /// 注册表已无文件后端（5.1 删除了文件回退），route 层测试需要一个可增删的
    /// 内存状态源来覆盖「注册 → 列表」往返。
    pub fn enable_projects_store(&self) {
        self.set_state_domain("projects", Some(serde_json::json!({ "projects": [] })));
        self.set_state_mutate_ok(true);
    }

    /// migrate-project-registry 6.2（route 层测试用）：绕过 API 的路径执法，
    /// 直接落一条注册表条目——用于构造「升级前遗留」的越界本机项目与远端
    /// 项目。返回该条目 id。
    pub fn seed_project(&self, node_id: &str, path: &str) -> String {
        let entry = project_entry_json(node_id, path);
        let id = entry
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut domains = self.state_domains.lock().expect("state domain lock");
        let slot = domains
            .entry("projects".to_string())
            .or_insert_with(|| Some(serde_json::json!({ "projects": [] })));
        if slot.is_none() {
            *slot = Some(serde_json::json!({ "projects": [] }));
        }
        slot.as_mut()
            .and_then(|v| v.get_mut("projects"))
            .and_then(|p| p.as_array_mut())
            .expect("projects array")
            .push(entry);
        id
    }

    /// add-fetch-models：注入某个 provider 的抓取结果（route 层测试用）。
    pub fn set_fetch_models_result(&self, provider: &str, result: Result<Vec<String>, String>) {
        self.fetch_models_results
            .lock()
            .expect("fetch models results lock")
            .insert(provider.to_string(), result);
    }

    /// add-remote-execution-node 8.x：注入节点注册表视图（route 层测试用）。
    /// `Some(vec![])` = 注册表可达但一个节点都没注册（这是两回事，不能混）。
    pub fn set_nodes(&self, nodes: Option<Vec<NodeInfo>>) {
        *self.nodes.lock().expect("nodes lock") = nodes;
    }

    /// add-remote-execution-node 8.1：注入节点侧路径判定结果（route 层测试用）。
    pub fn set_path_check(&self, node_id: &str, path: &str, result: Result<PathCheck, String>) {
        self.path_checks
            .lock()
            .expect("path checks lock")
            .insert((node_id.to_string(), path.to_string()), result);
    }

    /// add-remote-execution-node 8.1：最近一次 spawn/placeholder 收到的 node
    /// （route 层测试断言「项目的 node_id 确实传到了 trait 缝」）。
    pub fn last_spawn_node(&self) -> Option<String> {
        self.last_spawn_node
            .lock()
            .expect("last spawn node lock")
            .clone()
            .flatten()
    }

    /// fix-webui-qa-defects 2.2：已记录的 restore 调用（route 层测试断言
    /// 「重建请求确实传到了缝上」）。round4 3.1：随行携带命名来源。
    pub async fn restores(
        &self,
    ) -> Vec<(
        ChannelKey,
        Option<String>,
        Option<String>,
        usize,
        SessionIdentity,
        Option<String>,
        Option<String>,
    )> {
        self.restores.lock().expect("restores lock").clone()
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
            remote: None,
            desired_mode: sebas_dispatch::engine::ask_mode(),
            effective_mode: None,
            // rail-declutter-unread：fake 会话不产 transcript，段数为 0。
            msg_count: 0,
            // session-slash-commands：fake 会话无命令面（空表不上 wire）。
            available_commands: Vec::new(),
            // fix-pending-queue-liveness 2.3：fake 会话（spawning 占位）恒
            // 占用（spawn 窗口）。
            turn_engaged: true,
            spawn_failure_reason: None,
            parked_approvals: 0,
            label: None,
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

    // fix-webui-qa-defects-round5 1.2：注入的类型化拒绝（路由映射断言用）；
    // 未注入 = 与 trait 缺省同形的诚实不可用。
    async fn remove_pending(
        &self,
        _key: ChannelKey,
        _pending_id: u64,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        match self
            .pending_op_rejection
            .lock()
            .expect("pending op rejection lock")
            .clone()
        {
            Some(reason) => Err(SessionRejection::PendingRejected { reason }),
            None => Err(SessionRejection::Unavailable {
                cause: "此后端不承载待执行队列".into(),
            }),
        }
    }

    async fn move_pending(
        &self,
        _key: ChannelKey,
        _pending_id: u64,
        _to_index: usize,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        match self
            .pending_op_rejection
            .lock()
            .expect("pending op rejection lock")
            .clone()
        {
            Some(reason) => Err(SessionRejection::PendingRejected { reason }),
            None => Err(SessionRejection::Unavailable {
                cause: "此后端不承载待执行队列".into(),
            }),
        }
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

    /// fix-webui-qa-defects 2.2：reachable = 记录调用并成功；unreachable =
    /// 如实拒绝（route 层据此断言「失败时归档不动」）。
    async fn restore_session(
        &self,
        key: ChannelKey,
        session_id: Option<String>,
        project_dir: Option<String>,
        transcript: Vec<TurnEntry>,
        identity: SessionIdentity,
        label: Option<String>,
        prompt_preview: Option<String>,
    ) -> Result<(), SessionRejection> {
        if !self.reachable.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(SessionRejection::Unavailable {
                cause: "核心不可达（fake）".into(),
            });
        }
        let n = transcript.len();
        self.restores
            .lock()
            .expect("restores lock")
            .push((
                key,
                session_id,
                project_dir,
                n,
                identity,
                label,
                prompt_preview,
            ));
        Ok(())
    }

    /// fix-webui-approval-restore-and-session-identity 1.2：注入的读模型；
    /// 未注入的键如实拒绝（UnknownSession → 路由 404）。
    async fn pending_approvals(
        &self,
        key: ChannelKey,
    ) -> Result<Vec<PendingApproval>, SessionRejection> {
        let encoded = crate::routes::encode_session_key(&key);
        match self.approvals.lock().expect("approvals lock").get(&encoded) {
            Some(list) => Ok(list.clone()),
            None => Err(SessionRejection::UnknownSession {
                key: encoded,
            }),
        }
    }

    /// fix-webui-approval-restore-and-session-identity 5.1：就地改会话行的
    /// label（快照即真相）；未知会话拒绝。记录最近一次入参供断言。
    async fn set_session_label(
        &self,
        key: ChannelKey,
        label: Option<String>,
    ) -> Result<(), SessionRejection> {
        let encoded = crate::routes::encode_session_key(&key);
        *self.last_label.lock().expect("last label lock") = Some((encoded.clone(), label.clone()));
        let mut g = self.inner.write().await;
        match g
            .sessions
            .iter_mut()
            .find(|s| s.channel == key.channel.as_str() && s.key == key.reference)
        {
            Some(s) => {
                s.label = label;
                Ok(())
            }
            None => Err(SessionRejection::UnknownSession { key: encoded }),
        }
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
        if !self
            .state_mutate_ok
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err("state store 不可用".into());
        }
        // migrate-project-registry 6.2：projects 域是真实的内存状态源（其余
        // 域维持 no-op——那些测试只断言调用成败，不断言快照变化）。
        if domain == "projects" {
            let mut domains = self.state_domains.lock().expect("state domain lock");
            return apply_project_mutation(&mut domains, &payload);
        }
        Ok(())
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

    /// add-remote-execution-node 8.1：记录 node 便于 route 层断言「项目节点确实
    /// 传到了缝上」，其余行为与 `spawn` 一致。
    async fn spawn_with(
        &self,
        prompt: String,
        project_dir: Option<String>,
        _agent: &str,
        _model: Option<String>,
        _mode: Option<String>,
        node: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        *self.last_spawn_node.lock().expect("last spawn node lock") = Some(node.clone());
        self.spawn(prompt, project_dir).await
    }

    /// 远端 0-turn 占位如实拒绝（与 core 的远端路由同口径）：远端会话必须由
    /// 一条真实的首条输入建立，占位行没有可对应的远端执行事实。
    async fn create_placeholder(
        &self,
        project_dir: Option<String>,
        _agent: &str,
        _model: Option<String>,
        _mode: Option<String>,
        node: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        *self.last_spawn_node.lock().expect("last spawn node lock") = Some(node.clone());
        if let Some(remote) = remote_node_of(node.as_deref()) {
            return Err(SessionRejection::Unavailable {
                cause: format!("远端会话不能建 0-turn 占位（节点 {remote}）：请直接发送第一条输入"),
            });
        }
        self.spawn(String::new(), project_dir).await
    }

    /// add-remote-execution-node 8.x：注入的注册表视图；`None` = 未注入 →
    /// 走 trait 默认（诚实不可用）。
    async fn nodes(&self) -> Result<Vec<NodeInfo>, String> {
        match self.nodes.lock().expect("nodes lock").clone() {
            Some(nodes) => Ok(nodes),
            None => Err("此后端不承载节点注册表（节点可用性不可得）".into()),
        }
    }

    /// add-remote-execution-node 8.1：注入的节点侧判定；未注入如实拒绝。
    async fn check_node_path(&self, node_id: &str, path: &str) -> Result<PathCheck, String> {
        self.path_checks
            .lock()
            .expect("path checks lock")
            .get(&(node_id.to_string(), path.to_string()))
            .cloned()
            .unwrap_or_else(|| Err("此后端不能向节点发起路径校验".into()))
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
                remote: None,
                desired_mode: sebas_dispatch::engine::ask_mode(),
                effective_mode: None,
                msg_count: 0,
                available_commands: Vec::new(),
                turn_engaged: false,
                spawn_failure_reason: None,
                parked_approvals: 0,
                label: None,
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

    // （add-core-reachability-ws-push 1.1）无翻转源的后端给立即关闭的接收端
    // ——消费方（ws 支路）据此停用该腿，不是报错。
    #[tokio::test]
    async fn reachability_updates_default_is_immediately_closed() {
        let backend = FakeBackend::new();
        let mut rx = backend.reachability_updates();
        assert!(matches!(
            rx.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ));

        // in-process 同款（与 core 同进程同生共死，无独立翻转源）。
        let map = sebas_dispatch::SessionMap::new();
        let (router, _rx) = sebas_dispatch::DispatchHandle::new(map);
        let in_process = InProcessBackend::new(router);
        let mut rx = in_process.reachability_updates();
        assert!(matches!(
            rx.recv().await,
            Err(broadcast::error::RecvError::Closed)
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

    // fix-webui-qa-defects-round5 1.3：Unavailable 的 Display 不再自称
    // 「核心不可达」——该说法只保留给 core.reachability 真实可达性信号
    // （前端 fatal 横幅）；语义不可用（如不承载队列）如实呈现 cause。
    #[test]
    fn unavailable_display_names_the_cause_without_claiming_core_unreachable() {
        let r = SessionRejection::Unavailable {
            cause: "此后端不承载待执行队列".into(),
        };
        let text = r.to_string();
        assert_eq!(text, "操作不可用: 此后端不承载待执行队列");
        assert!(!text.contains("核心不可达"), "{text}");
    }

    // fix-webui-qa-defects-round5 1.2 收口：PendingReason 四类 Display
    // 是 wire 上 404 / 409 / 400 文案的最终真源（api.rs 路由层只做
    // HTTP 状态码映射，文本走这里的 Display），下游 e2e（pending_management
    // _passes_typed_rejections_through_composite_409 / _reaches_the_host_
    // backend_in_embedded_shape）按字面匹配 cause 子串——文案漂移即 break
    // 契约。单测钉死四类：未知 id / 已开始 / 越优先 / 越界。
    #[test]
    fn pending_reason_display_text_is_the_wire_truth() {
        let cases = [
            (PendingReason::Unknown, "待执行提交不存在"),
            (PendingReason::AlreadyStarted, "该提交已开始执行"),
            (PendingReason::PriorityConflict, "不能越过优先提交排序"),
            (PendingReason::OutOfRange, "目标位置越界"),
        ];
        for (reason, expected) in cases {
            let text = reason.to_string();
            assert_eq!(text, expected, "{reason:?}");
            // 四类文案均不得冒充可达性失败（round5 1.3 文案诚实性）。
            assert!(
                !text.contains("核心不可达") && !text.contains("操作不可用"),
                "{reason:?} must not impersonate reachability: {text}"
            );
        }
    }

    // ── permission-mode-auto-gate：answer_permission 的 AllowSession 组合 ──

    async fn recv_out(
        rx: &mut tokio::sync::mpsc::Receiver<sebas_dispatch::Out>,
    ) -> sebas_dispatch::Out {
        tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
            .await
            .expect("Out within 200ms")
            .expect("outbound channel open")
    }

    #[tokio::test]
    async fn answer_permission_allow_session_also_switches_mode_to_auto() {
        let map = sebas_dispatch::SessionMap::new();
        let key = sebas_channels::ChannelKey::feishu("oc_mode", None);
        map.insert(key.clone(), sebas_dispatch::Mapping::active("s-mode"))
            .await
            .unwrap();
        let (router, mut out_rx) = sebas_dispatch::DispatchHandle::new(map);
        let backend = InProcessBackend::new(router.clone());
        // 生产由 relay 在 PermissionRequest 广播时登记；测试直接注入
        // request_id → routing session_id（同文件私有字段）。批复前请求必须
        // 处于泊车中（fail-closed 前提）——与 dispatch_acp_event 泊车登记一致。
        backend
            .request_sessions
            .write()
            .await
            .insert("req-auto".into(), "s-mode".into());
        router
            .stall_registry()
            .note_permission_parked("s-mode", "req-auto", "Bash", serde_json::json!({}))
            .await;

        assert!(
            backend
                .answer_permission("req-auto", PermissionDecision::AllowSession)
                .await
        );
        // 批复成功后泊车解除（读模型随之清空——1.1 契约的批复侧）。
        assert_eq!(
            router
                .pending_permission_requests(&key)
                .await
                .unwrap()
                .len(),
            0
        );
        // 迟到的第二次批复被拒（1.4/2.3：不复活任何状态）。
        assert!(
            !backend
                .answer_permission("req-auto", PermissionDecision::Deny)
                .await,
            "a decided request must not be answerable again"
        );

        // Out 顺序钉死语义：① 放行（首要语义先行）② SetMode{auto}
        // （与 webui 中程切换/dispatch 点击路径同源的组合件）。
        match recv_out(&mut out_rx).await {
            sebas_dispatch::Out::SendAcp {
                session_id,
                cmd: sebas_acp::AcpCommand::PermissionReply { decision, .. },
            } => {
                assert_eq!(session_id, "s-mode");
                assert!(matches!(decision, sebas_acp::Decision::AllowSession));
            }
            other => panic!("expected PermissionReply first, got {other:?}"),
        }
        match recv_out(&mut out_rx).await {
            sebas_dispatch::Out::SendAcp {
                session_id,
                cmd:
                    sebas_acp::AcpCommand::SetMode {
                        session_id: sid,
                        mode,
                    },
            } => {
                assert_eq!(session_id, "s-mode");
                assert_eq!(sid, "s-mode");
                assert_eq!(mode, "auto");
            }
            other => panic!("expected SetMode after reply, got {other:?}"),
        }
        // desired_mode 落位（成败的最终回执走 engine 的事件流处理）。
        let desired = router.map.get(&key).await.map(|m| m.desired_mode);
        assert_eq!(desired.as_deref(), Some("auto"));
    }

    #[tokio::test]
    async fn answer_permission_allow_once_and_deny_do_not_switch_mode() {
        let map = sebas_dispatch::SessionMap::new();
        let key = sebas_channels::ChannelKey::feishu("oc_mode", None);
        map.insert(key.clone(), sebas_dispatch::Mapping::active("s-mode"))
            .await
            .unwrap();
        let (router, mut out_rx) = sebas_dispatch::DispatchHandle::new(map);
        let backend = InProcessBackend::new(router.clone());
        backend
            .request_sessions
            .write()
            .await
            .insert("req-1".into(), "s-mode".into());
        backend
            .request_sessions
            .write()
            .await
            .insert("req-2".into(), "s-mode".into());
        // 两条请求均在泊车中（批复的 fail-closed 前提）。
        router
            .stall_registry()
            .note_permission_parked("s-mode", "req-1", "Bash", serde_json::json!({}))
            .await;
        router
            .stall_registry()
            .note_permission_parked("s-mode", "req-2", "Read", serde_json::json!({}))
            .await;

        for (req, expect) in [
            ("req-1", sebas_acp::Decision::AllowOnce),
            ("req-2", sebas_acp::Decision::Deny),
        ] {
            let d = match expect {
                sebas_acp::Decision::AllowOnce => PermissionDecision::AllowOnce,
                _ => PermissionDecision::Deny,
            };
            assert!(backend.answer_permission(req, d).await);
            match recv_out(&mut out_rx).await {
                sebas_dispatch::Out::SendAcp {
                    cmd: sebas_acp::AcpCommand::PermissionReply { decision, .. },
                    ..
                } => {
                    assert_eq!(
                        std::mem::discriminant(&decision),
                        std::mem::discriminant(&expect)
                    );
                }
                other => panic!("expected PermissionReply for {req}, got {other:?}"),
            }
        }
        // 只有 reply、没有 SetMode；desired_mode 不被触碰。
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), out_rx.recv())
                .await
                .is_err(),
            "AllowOnce/Deny 不得触发 SetMode"
        );
        // （3.2，D5b）desired 非空：未被触碰 = 仍是构造时的缺省词 ask。
        let desired = router.map.get(&key).await.map(|m| m.desired_mode);
        assert_eq!(
            desired.as_deref(),
            Some(sebas_dispatch::engine::ASK_MODE),
            "AllowOnce/Deny 不得改写 desired_mode"
        );
    }
}
