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
use sebas_agent::policy::SandboxMode;
use sebas_agent::policy::{Approver, ApproverHub, PolicyConfig, PolicyEngine};
use sebas_agent::session::{AgentEvent, SessionConfig, SessionHandle, SessionManager};
use sebas_agent::tools::ToolRegistry;
use sebas_channels::ChannelKey;
use sebas_channels::card::AppUsage;
use sebas_domain::session::CardPhase;
use sebas_dispatch::{
    PendingApproval, PendingSubmission, SessionEvent, SessionIdentity, SessionInfo, TurnEntry,
    TurnStreamEvent, count_chat_messages,
};
use sebas_webui::session_backend::{
    CloseReport, PermissionDecision, PermissionNotice, Reachability, SessionBackend,
    SessionRejection,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, broadcast};

/// One live native session: kernel handle + its rendered transcript.
struct NativeSession {
    handle: SessionHandle,
    workdir: Option<String>,
    prompt: String,
    /// Rendered transcript entries (turn-content retrieval source).
    /// fix-webui-streaming-liveness 2.1（D2）：文本 delta 逐条落账——与 IM 桥
    /// （`native_dispatch_bridge`）和 ACP 引擎同粒度，不再攒到回合/工具边界
    /// 整块 flush。日志（本 vec）仍是唯一事实。
    transcript: Vec<TurnEntry>,
    /// rail-declutter-unread D1/D2：可见回复段数改为**派生**口径——
    /// `count_chat_messages`（相邻连续 markdown 条目合并为一段，ACP 面同
    /// 款），逐 delta 落账后不再按 flush 次数累计。
    /// （wire-webui-sebas-agent-e2e）会话级模型 override。`None` = 走内核默认；
    /// 设置后下一次 turn 起用，新值即时生效。
    current_model_override: Option<String>,
    /// 装配期的可用模型清单（来自 `SEBAS_AGENT_MODELS`）。`info()` 透出。
    available_models: Vec<String>,
    /// 装配期的默认模型 id（`SEBAS_AGENT_MODEL`），与内核 SessionConfig 共用。
    default_model: String,
    /// （workbench-interaction-polish 1.1）宿主侧在飞近似：prompt 提交置位、
    /// 终态事件（Finished / Error）复位。内核侧串行队列——排队中的下一条
    /// prompt 开轮时宿主没有事件可依，此标志可能在「排队连跑」窗口里落后
    /// 真实一拍；cancel 的 Idle 拒绝按它判定（内核对空闲 cancel 本就无效果，
    /// 会话无损，只是拒绝文案可能偏保守）。
    in_flight: bool,
    /// （extend-test-model-scenarios 3.4）本轮是否已落过**可见输出**条目
    /// （正文/thinking/工具/错误）。空闲态收到 prompt 开新一轮时复位；
    /// 回合收尾（`AE::SessionSummary`）时仍为假 = 零输出回合，按域层文案
    /// 追加一条 `notice` 合成提示，回合在时间线上不再不可见地消失。
    ///
    /// 与 ACP 面的差异：原生转录没有 `prompt` 条目（操作者提交不走内核
    /// 事件），因此判据不是「最后一条 prompt 之后」而是宿主侧的开轮标志。
    /// 排队连跑（busy 时再提交）沿用在飞近似的同一取舍：标志只在空闲开轮
    /// 时复位，排队轮次的零输出不单独补提示。
    turn_visible_output: bool,
    /// （fix-webui-qa-round8 2.2；2.2 review 补修）宿主**持有的**排队提交栈：
    /// busy 时的提交只入本队列、不直送内核（直送会让 remove/move 撤不掉已
    /// 进内核串行队列的提交）。队列由 pump 驱动推进：回合终态（Finished /
    /// Error，含取消）+ 该回合的 summary 落账（零输出判定完结）后，弹出
    /// 队头置 in_flight 并投递内核——与 ACP 面的队列推进语义对齐。空闲
    /// 提交立即开轮不入队（它在飞，不在栈上）；`close` / 终态拆除随会话
    /// 清空。remove/move 因此对队列内容有完整否决权。
    pending_queue: Vec<PendingSubmission>,
    /// （fix-webui-qa-round11 1.2，design D1）卡相位真值：回合开始 →
    /// `OnIt`、正常结束 → `Done`、失败 → `CrossMark`。与 ACP 卡态同一
    /// 词汇、同一元数据通道（`SessionInfo.phase`）——`SessionStatus::derive`
    /// 继续单点派生，native 会话不再因「输入恒空」永远落在 Queued。
    /// `None` = 尚无任何回合（占位创建），诚实呈 Queued。
    card_phase: Option<CardPhase>,
    /// （fix-webui-qa-round11 1.2）首条用户消息预览（行命名锚定值，与 ACP
    /// `first_prompt_preview` 同语义）：spawn 首条 prompt 或占位创建后的
    /// 第一条 `message()` 落定，后续消息绝不移动它。
    first_prompt_preview: Option<String>,
    /// （add-local-usage-statistics 3.1）会话累计 usage（快照芯片的数据源，
    /// 与 ACP 卡态 `usage` 同一呈现面）：逐回合 `SessionSummary` 累计（跨
    /// 回合求和、`None` 保 None）。`None` = 从未有 token 上报（门控语义：
    /// 不冒充全零，快照 `usage` 保持缺席）。
    session_usage: Option<sebas_domain::usage::TurnTokenUsage>,
}

impl NativeSession {
    /// Append one content entry (position assigned monotonically from the
    /// transcript length) and return the landed entry for the live turn
    /// event. The clone keeps the transcript authoritative; the returned
    /// entry rides the broadcast.
    /// `title` / `tool_use_id` 仅工具条目携带（fold-tool-calls-into-process-
    /// tree 3.2）：一等 tool 条目 = `element_type = "tool"` + 结构化标题 +
    /// 上游 call id，其余条目两参恒 `None`。
    fn push_entry(
        &mut self,
        element_type: &str,
        content: String,
        title: Option<String>,
        tool_use_id: Option<String>,
    ) -> TurnEntry {
        let entry = TurnEntry {
            position: self.transcript.len() as u64,
            kind: sebas_domain::session::TurnKind::Content,
            element_type: sebas_domain::session::TurnElementType::from_wire(element_type),
            content,
            created_at_unix: chrono::Utc::now().timestamp().max(0) as u64,
            title,
            tool_use_id,
            failure_class: None,
        };
        self.transcript.push(entry.clone());
        entry
    }

    /// （fix-webui-qa-round8 2.1）操作者提交的 prompt 条目（与 ACP `seed_card`
    /// 等价）：内核事件里没有「用户说了什么」——宿主在投递内核前把它落进
    /// 转录（`kind = "prompt"`），native 转录不再「只见回复不见提问」。
    fn push_prompt_entry(&mut self, content: String) -> TurnEntry {
        let entry = TurnEntry {
            position: self.transcript.len() as u64,
            kind: sebas_domain::session::TurnKind::Prompt,
            element_type: sebas_domain::session::TurnElementType::Markdown,
            content,
            created_at_unix: chrono::Utc::now().timestamp().max(0) as u64,
            title: None,
            tool_use_id: None,
            failure_class: None,
        };
        self.transcript.push(entry.clone());
        entry
    }

    /// 单条转录条目的 turn 流广播（pump `land` 与宿主侧投递共用：日志是
    /// 唯一事实，广播只是增量补充——Lagged 由消费端按快照收敛）。
    fn broadcast_entry(
        turn_events: &broadcast::Sender<TurnStreamEvent>,
        key: &ChannelKey,
        entry: TurnEntry,
    ) {
        let _ = turn_events.send(TurnStreamEvent {
            channel: key.channel_str().to_string(),
            key: key.reference.clone(),
            entries: vec![entry],
        });
    }

    /// （fix-webui-qa-round8 2.2）影子队列视图：position 重算为投递序下标
    /// （与 ACP 面同语义——视图即投影，不持久内部序号）。
    fn pending_view(&self) -> Vec<PendingSubmission> {
        self.pending_queue
            .iter()
            .enumerate()
            .map(|(i, p)| PendingSubmission {
                id: p.id,
                text: p.text.clone(),
                position: i,
                disposition: p.disposition.clone(),
                priority: p.priority,
            })
            .collect()
    }

    /// 可见回复段数（派生口径，与 ACP 面一致）：连续 markdown 合并一段、
    /// error 逐条计、prompt/thinking/tool 不计。
    fn msg_count(&self) -> u64 {
        count_chat_messages(&self.transcript)
    }

    fn info(&self, key: &ChannelKey, parked_approvals: u32) -> SessionInfo {
        // Native keys are feishu-channel `agent-{8-hex}` references with no
        // thread part; feed the flattened SessionInfo directly off the key.
        SessionInfo {
            channel: key.channel_str().to_string(),
            key: key.reference.clone(),
            session_id: Some(self.handle.key.clone()),
            status: sebas_domain::session::SessionPhase::Active,
            // （fix-webui-qa-round11 1.2，design D1）卡相位随回合真值回填
            // （OnIt/Done/CrossMark）——`SessionStatus::derive` 的输入不再是
            // 恒空，native 会话与 ACP 同一派生单点推进 Working/Done/Failed；
            // None（占位创建、尚无回合）诚实落 Queued。
            phase: self.card_phase.clone(),
            user_prompt: Some(self.prompt.clone()),
            last_active_unix: chrono::Utc::now().timestamp(),
            project_dir: self.workdir.clone(),
            // wire-webui-sebas-agent-e2e: 原生内核可用模型来自装配期的环境
            // 变量清单；当前模型取会话级 override，缺省为内核默认。
            current_model: self
                .current_model_override
                .clone()
                .or(Some(self.default_model.clone())),
            available_models: Some(self.available_models.clone()),
            // 原生内核不属于任何 ACP kind（add-composer-agent-binding）。
            agent_kind: None,
            // 原生内核 usage（add-local-usage-statistics 3.1）：快照 `usage`
            // 从恒 None 改为如实携带——逐回合 summary 累计的中立计数折算成
            // 芯片形状（`AppUsage`）。从未上报的会话保持 None（门控语义）。
            usage: self.session_usage.as_ref().map(|u| AppUsage {
                model: u.model.clone(),
                total_input: u.input_tokens.unwrap_or(0),
                total_output: u.output_tokens.unwrap_or(0),
            }),
            // wire-webui-sebas-agent-e2e D4：native 会话在快照/事件中自带执行体标。
            backend: Some("native".into()),
            // （fix-webui-qa-round8 2.2）native 会话的待执行栈 = 宿主影子队列
            // （提交即记、终态帧对账推进）。pending-stack 据此在 native 会话
            // 上与 ACP 同样可渲染；空队列不上 wire 语义不变。
            pending: self.pending_view(),
            // 原生内核会话跑在主控本机，没有节点维度。
            remote: None,
            // （add-agent-mode-selection）native 内核不承载 mode：不声称生效。
            desired_mode: sebas_dispatch::engine::ask_mode(),
            effective_mode: None,
            // rail-declutter-unread D1：可见回复段数——派生口径（2.1 随逐
            // delta 落账切换），transcript 是唯一事实。
            msg_count: self.msg_count(),
            // （session-slash-commands 2.3）native 内核无命令发现——
            // AgentEvent 词汇不动，命令表恒空（composer 不渲染面板、
            // `/` 前缀按普通文本放行的诚实退化）。
            available_commands: Vec::new(),
            // fix-pending-queue-liveness 2.3 + fix-webui-qa-round11 1.2：回合
            // 占用事实 = 宿主在飞近似 ∨ 泊车审批在等（与 ACP 的 WORKING ∨
            // 泊车同表）。composer 的停止钮据此对 native in-flight 回合出现。
            turn_engaged: self.in_flight || parked_approvals > 0,
            spawn_failure_reason: None,
            parked_approvals,
            label: None,
            // （fix-webui-qa-round11 1.2）首条消息预览随行下发——行命名链的
            // 锚定数据源，native 会话首条消息后不再「未命名会话」。
            first_prompt_preview: self
                .first_prompt_preview
                .clone()
                .filter(|p| !p.is_empty()),
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
    /// 实时回合内容（fix-webui-streaming-liveness 2.1，D2）：native 面的
    /// turn 事件流——transcript 每落一条就广播一次，webui WS 以 `turn.append`
    /// 转播。Lagged 只丢增量（快照收敛），消费端不断链。
    turn_events: broadcast::Sender<TurnStreamEvent>,
    /// Gated-call feed (review cards).
    notices: broadcast::Sender<PermissionNotice>,
    /// 泊车审批读模型（native 侧）：encoded key → request_id → 审批行。pump
    /// 在 `PermissionRequest` 事件登记，回合终态（Finished/Error）、close 与
    /// 决策投递成功时清除；`pending_approvals` 据此枚举。webui 打开/刷新会话
    /// 经它重建审批面，不再依赖一次性 WS 推送（与 ACP 读模型同契约）——
    /// fake-provider 秒回的回合里推送会先于页面挂载到达，没有这层兜底，
    /// native 审批卡在竞态下永久丢失。
    pending_approvals:
        Arc<RwLock<HashMap<String, HashMap<String, PendingApproval>>>>,
    /// Why the native backend is unavailable (missing LLM credentials), if so.
    unavailable_cause: Option<String>,
    /// （wire-webui-sebas-agent-e2e）原生内核可供选择的模型 id 列表。来自
    /// `SEBAS_AGENT_MODELS`（逗号分隔），缺省仅含 `SEBAS_AGENT_MODEL`。
    /// WebUI composer 的模型下拉数据源。
    available_models: Vec<String>,
    /// （wire-webui-sebas-agent-e2e）默认模型 id（`SEBAS_AGENT_MODEL`），
    /// 在 native 会话尚未设置任何覆盖前对所有 turn 生效。
    default_model: String,
    /// （fix-webui-qa-round8 2.2）影子队列条目的稳定 id 源（后端级单调，跨
    /// 会话唯一——remove/move 按 `(key, id)` 寻址）。
    next_pending_id: std::sync::atomic::AtomicU64,
    /// （add-local-usage-statistics 3.2）native 直连回合的本地落账 sink：
    /// pump 在 `SessionSummary` 帧结算一行交它。`None` = 本装配经 router
    /// （`SEBAS_AGENT_ROUTER_URL` 已注入，router 已记）→ 不本地记（design
    /// D1 双算规避的写入侧分叉）。装配晚于构造（run.rs 顺序），走 `RwLock`
    /// 注入；sink 是廉价 Clone（通道发送端 + 查询句柄），按值存。
    local_usage: std::sync::RwLock<Option<crate::usage_local::LocalUsageSink>>,
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
             (set SEBAS_AGENT_PROVIDER_API_KEY or SEBAS_AGENT_ROUTER_URL)",
        ))
    }
}

impl NativeAgentBackend {
    /// 从环境装配原生内核 manager（design N9）。优先直连
    /// `SEBAS_AGENT_PROVIDER_BASE_URL` + `SEBAS_AGENT_PROVIDER_API_KEY`
    /// （默认端点 `https://api.anthropic.com`），或
    /// `SEBAS_AGENT_ROUTER_URL`（+ 可选 `SEBAS_AGENT_ROUTER_AUTH`）走 router。
    ///
    /// 无凭据时返回 `(manager, Some(cause), …)`——manager 可建但每个 spawn
    /// 会拒绝并报 cause；可用模型与默认模型从 `SEBAS_AGENT_MODELS` /
    /// `SEBAS_AGENT_MODEL` 推导，与管理器共享同一装配面（wire-webui-sebas-agent-e2e D5）。
    pub fn build_native_manager(
        bash_timeout: Duration,
    ) -> (Arc<SessionManager>, Option<String>, Vec<String>, String) {
        let (client, cause): (Option<Arc<dyn LlmClient>>, Option<String>) = {
            let router_url = std::env::var("SEBAS_AGENT_ROUTER_URL").ok();
            if let Some(url) = router_url {
                let auth = std::env::var("SEBAS_AGENT_ROUTER_AUTH")
                    .unwrap_or_else(|_| "sk-gw-local-dev".into());
                (
                    Some(Arc::new(AnthropicMessagesClient::router(url, auth))),
                    None,
                )
            } else {
                let base = std::env::var("SEBAS_AGENT_PROVIDER_BASE_URL")
                    .unwrap_or_else(|_| "https://api.anthropic.com".into());
                match std::env::var("SEBAS_AGENT_PROVIDER_API_KEY") {
                    Ok(key) if !key.is_empty() => (
                        Some(Arc::new(AnthropicMessagesClient::direct_provider(
                            base, key,
                        ))),
                        None,
                    ),
                    _ => (
                        None,
                        Some(
                            "native backend needs SEBAS_AGENT_PROVIDER_API_KEY \
                             (or SEBAS_AGENT_ROUTER_URL)"
                                .into(),
                        ),
                    ),
                }
            }
        };

        let model =
            std::env::var("SEBAS_AGENT_MODEL").unwrap_or_else(|_| "claude-sonnet-4-5".into());
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
        let (turn_events, _) = broadcast::channel(256);
        let (notices, _) = broadcast::channel(64);
        Arc::new(Self {
            manager,
            hub,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            events,
            turn_events,
            notices,
            pending_approvals: Arc::new(RwLock::new(HashMap::new())),
            unavailable_cause,
            available_models,
            default_model,
            next_pending_id: std::sync::atomic::AtomicU64::new(1),
            local_usage: std::sync::RwLock::new(None),
        })
    }

    /// （add-local-usage-statistics 3.2）装配点注入 native 直连回合的本地
    /// 落账 sink（run.rs 按 `SEBAS_AGENT_ROUTER_URL` 门控后传入；`None` =
    /// 经 router 的装配，不本地记）。幂等覆盖（重复 set 以最后一次为准）。
    pub fn set_local_usage(&self, sink: Option<crate::usage_local::LocalUsageSink>) {
        *self
            .local_usage
            .write()
            .unwrap_or_else(|e| e.into_inner()) = sink;
    }

    /// 会话键编码走 `sebas-channels` 的唯一实现（canonical URL-safe
    /// `channel\0reference` percent 形）。此前这里是 ChannelKey 的 JSON
    /// 字符串形——`permission.requested` 帧把它原样透传（api.rs 不再加工），
    /// 与 webui 路由/turn 流的编码形不一致，review-card 的精确匹配永远丢弃
    /// native 审批卡；ACP 通路（InProcessBackend 中继）经 `encode_session_key`
    /// 产出编码形，两条通路必须在 wire 上同形（webui-ws-rpc 契约）。
    fn encode_key(key: &ChannelKey) -> String {
        sebas_channels::key::encode_session_key(key)
    }

    async fn session_info(&self, encoded: &str) -> Option<SessionInfo> {
        // 泊车数先取（pending_approvals 与 sessions 无同时持锁——读守卫在
        // 语句末即释放，锁序 sessions → pending_approvals 不变）。
        let parked = self.parked_count(encoded).await;
        let g = self.sessions.read().await;
        g.get(encoded)
            .map(|s| s.info(&Self::decode_agent_key(encoded), parked))
    }

    /// （fix-webui-qa-round11 1.2）某会话的泊车审批数：`info()` 的
    /// `parked_approvals` / `turn_engaged` 泊车维度输入（waiting 投影与
    /// composer 停止钮同源）。
    async fn parked_count(&self, encoded: &str) -> u32 {
        self.pending_approvals
            .read()
            .await
            .get(encoded)
            .map(|m| m.len() as u32)
            .unwrap_or(0)
    }

    /// encode_key 的逆：严格解码（无 NUL / 非法转义 → None）。退路保持旧
    /// 语义（整串当作 feishu reference）——只服务防御性输入，正常键全部
    /// 出自 [`Self::encode_key`]，必可解码。
    fn decode_agent_key(encoded: &str) -> ChannelKey {
        sebas_channels::key::decode_session_key(encoded)
            .unwrap_or_else(|| ChannelKey::new("feishu", encoded))
    }

    /// Drive one native session: kernel events → transcript + lifecycle
    /// events + review-card notices. Runs until the session task dies.
    ///
    /// fix-webui-streaming-liveness 2.1（D2）：transcript 的每一次落账都同步
    /// 广播一条 `TurnStreamEvent`（逐 delta、逐工具痕迹），webui 据此实时
    /// 呈现——transcript 与 turn 流由同一段代码产出，粒度天然一致。broadcast
    /// `send` 是同步的，锁内发送不会阻塞 pump。
    #[allow(clippy::too_many_arguments)]
    async fn pump(
        mut rx: broadcast::Receiver<AgentEvent>,
        key: ChannelKey,
        encoded: String,
        sessions: Arc<RwLock<HashMap<String, NativeSession>>>,
        events: broadcast::Sender<SessionEvent>,
        turn_events: broadcast::Sender<TurnStreamEvent>,
        notices: broadcast::Sender<PermissionNotice>,
        pending_approvals: Arc<RwLock<HashMap<String, HashMap<String, PendingApproval>>>>,
        // （add-local-usage-statistics 3.2）native 直连回合的本地落账 sink；
        // `None` = 经 router 的装配（router 已记，不本地记）。
        local_usage: Option<crate::usage_local::LocalUsageSink>,
    ) {
        use AgentEvent as AE;
        // transcript 落账 + turn 事件广播的一体化出口（锁内调用）。
        fn land(
            session: &mut NativeSession,
            turn_events: &broadcast::Sender<TurnStreamEvent>,
            key: &ChannelKey,
            element_type: &str,
            content: String,
        ) {
            land_entry(session, turn_events, key, element_type, content, None, None);
        }
        /// fold-tool-calls-into-process-tree 3.2：一等 tool 条目出口——
        /// `element_type = "tool"` + 结构化标题（复用 `sebas_dispatch`
        /// ::tool_entry_title，键序表不复制）+ 上游 call id。
        #[allow(clippy::too_many_arguments)]
        fn land_tool(
            session: &mut NativeSession,
            turn_events: &broadcast::Sender<TurnStreamEvent>,
            key: &ChannelKey,
            content: String,
            title: String,
            tool_use_id: Option<String>,
        ) {
            land_entry(
                session,
                turn_events,
                key,
                "tool",
                content,
                Some(title),
                tool_use_id,
            );
        }
        fn land_entry(
            session: &mut NativeSession,
            turn_events: &broadcast::Sender<TurnStreamEvent>,
            key: &ChannelKey,
            element_type: &str,
            content: String,
            title: Option<String>,
            tool_use_id: Option<String>,
        ) {
            // extend-test-model-scenarios 3.4：可见输出记账（判据与
            // `sebas-dispatch` 引擎面 `turn_has_visible_output` 同表：
            // markdown/thinking/tool/error 且内容非空；notice 本身不算）。
            // tool 与 markdown 同在可见输出表——本 change 把工具痕迹从
            // markdown 升为 tool 条目，记账语义不变（task 3.2）。
            if matches!(element_type, "markdown" | "thinking" | "tool" | "error")
                && !content.is_empty()
            {
                session.turn_visible_output = true;
            }
            let entry = session.push_entry(element_type, content, title, tool_use_id);
            let _ = turn_events.send(TurnStreamEvent {
                channel: key.channel_str().to_string(),
                key: key.reference.clone(),
                entries: vec![entry],
            });
        }
        // （fix-webui-qa-round8 2.2 review 补修）回合终态后由 pump 出队队头
        // 并投递内核：宿主持有队列（busy 提交不直送内核，remove/move 才有
        // 完整否决权）。终态（Finished/Error）与 summary 是**两个事件**——
        // 推进标记与待投递文本必须跨迭代存活：终态置位，下一帧（summary，
        // 零输出判定随之完结）出队队头。出队时点在 summary 落账之后，先复
        // 位判据会把上一轮的可见输出记账清掉。`dispatch` 在锁外投递（prompt
        // 发送在内核 cmd 通道上等待，绝不持锁跨 await）。
        let mut advance_queue = false;
        let mut dispatch: Option<String> = None;
        // （add-local-usage-statistics 3.2）回合终态观察：内核收尾序列是
        // 「终态（Finished/Error）先行、SessionSummary 随后」——落账行的
        // status/error 从这里取（summary 帧自身不携带终态）。消费即清。
        let mut last_terminal: Option<(u16, Option<String>)> = None;
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
                let Some(session) = g.get_mut(&encoded) else {
                    break;
                };
                // （fix-webui-qa-round11 1.2）泊车审批数是 `info()` 的输入之一
                // （waiting 投影 + turn_engaged 的泊车维度）。sessions →
                // pending_approvals 与 PermissionRequest 分支同序，无反向持锁。
                let parked = pending_approvals
                    .read()
                    .await
                    .get(&encoded)
                    .map(|m| m.len() as u32)
                    .unwrap_or(0);
                match ev {
                    AE::TextDelta { delta, .. } => {
                        // D2：逐 delta 落账 + 实时 turn 事件（不再只写
                        // text_buf 攒整块）。段计数走派生口径，不逐 delta 发
                        // Updated——会话状态刷新骑工具/回合边界，正文走
                        // turn.append。
                        land(session, &turn_events, &key, "markdown", delta);
                        None
                    }
                    AE::ThinkingDelta { delta, .. } => {
                        // extend-test-model-scenarios 3.3：thinking 逐段进转录
                        // （`element_type = "thinking"`），前端折叠呈现、与正文
                        // 可区分；`msg_count` 不计思考段。此前 thinking 在原生
                        // 面被直接丢弃，test/thinking 场景无落点。
                        land(session, &turn_events, &key, "thinking", delta);
                        None
                    }
                    AE::ToolProgress { .. } | AE::ToolFinish { .. } => None,
                    AE::ToolStart {
                        tool_name,
                        args,
                        tool_use_id,
                        ..
                    } => {
                        // fold-tool-calls-into-process-tree 3.2：调用条目升为
                        // 一等 tool 条目（📖 参数段 + 结构化标题 + call id），
                        // 不再以 markdown 正文呈现。
                        let args_str = serde_json::to_string_pretty(&args).unwrap_or_default();
                        land_tool(
                            session,
                            &turn_events,
                            &key,
                            format!("📖 **{tool_name}**\n```json\n{args_str}\n```"),
                            sebas_dispatch::tool_entry_title(false, &tool_name, Some(&args)),
                            tool_use_id,
                        );
                        Some(SessionEvent::Updated {
                            session: session.info(&key, parked),
                        })
                    }
                    AE::ToolEnd {
                        tool_name,
                        result,
                        tool_use_id,
                        ..
                    } => {
                        // fold-tool-calls-into-process-tree 3.2：结果条目同为
                        // 一等 tool 条目；ToolEnd wire 无 args → 标题退化
                        // `✓ {tool}`（与 ACP 面同一口径），id 与配对调用相等。
                        land_tool(
                            session,
                            &turn_events,
                            &key,
                            format!("✓ **{tool_name}**\n{result}"),
                            sebas_dispatch::tool_entry_title(true, &tool_name, None),
                            tool_use_id,
                        );
                        Some(SessionEvent::Updated {
                            session: session.info(&key, parked),
                        })
                    }
                    AE::PermissionRequest {
                        request_id,
                        tool_name,
                        args,
                        reason,
                        ..
                    } => {
                        land(
                            session,
                            &turn_events,
                            &key,
                            "markdown",
                            format!("⏳ **{tool_name}** awaits approval — {reason}"),
                        );
                        // 泊车登记（读模型半边）：决策/回合终态负责清除。
                        // 本帧的泊车数在登记后重算（info 帧如实带 1，
                        // waiting/turn_engaged 投影不再等下一次翻转）。
                        pending_approvals
                            .write()
                            .await
                            .entry(encoded.clone())
                            .or_default()
                            .insert(
                                request_id.clone(),
                                PendingApproval {
                                    request_id: request_id.clone(),
                                    tool_name: tool_name.clone(),
                                    args: args.clone(),
                                },
                            );
                        let parked = pending_approvals
                            .read()
                            .await
                            .get(&encoded)
                            .map(|m| m.len() as u32)
                            .unwrap_or(parked);
                        let _ = notices.send(PermissionNotice {
                            request_id,
                            session_id: encoded.clone(),
                            tool_name,
                            args,
                            reason,
                        });
                        Some(SessionEvent::Updated {
                            session: session.info(&key, parked),
                        })
                    }
                    AE::ToolPolicy {
                        tool_name, outcome, ..
                    } => {
                        land(
                            session,
                            &turn_events,
                            &key,
                            "markdown",
                            format!("🛡 **{tool_name}** policy: {outcome}"),
                        );
                        Some(SessionEvent::Updated {
                            session: session.info(&key, parked),
                        })
                    }
                    AE::SessionSummary {
                        turn_ms,
                        model_calls,
                        tool_calls,
                        usage,
                        ..
                    } => {
                        // extend-test-model-scenarios 3.4：回合正常收尾但本轮零
                        // 可见输出 → 追加域层同文案的 notice 合成提示（与
                        // ACP/IM 引擎面 `append_zero_output_notice_if_empty`
                        // 同语义：真实回合才补，notice 不重复补）。判据在落
                        // summary 之前取，summary 自己不算可见输出。
                        if !session.turn_visible_output {
                            land(
                                session,
                                &turn_events,
                                &key,
                                "notice",
                                sebas_domain::session::ZERO_OUTPUT_NOTICE.to_string(),
                            );
                        }
                        // （add-local-usage-statistics 3.1）快照 usage 的累计：
                        // 逐回合 summary 求和（None 保 None）——芯片在下一帧
                        // Updated 起如实携带；从未上报的会话保持缺席。
                        if let Some(u) = &usage {
                            match session.session_usage.as_mut() {
                                Some(acc) => acc.accumulate(u),
                                None => session.session_usage = Some(u.clone()),
                            }
                        }
                        // （add-local-usage-statistics 3.2/D5）直连回合落账一行：
                        // usage 如实透传（None = 只计请求数），终态取上面观察
                        // 到的收尾；sink 未装配（经 router）= 不本地记。
                        if let Some(sink) = &local_usage {
                            let (status, error) = last_terminal.take().unwrap_or((
                                crate::usage_local::TURN_STATUS_FINISHED,
                                None,
                            ));
                            sink.record(usage.clone().unwrap_or_default().into_turn_record(
                                crate::usage_local::PROTOCOL_NATIVE,
                                status,
                                turn_ms,
                                error,
                            ));
                        }
                        land(
                            session,
                            &turn_events,
                            &key,
                            "markdown",
                            format!(
                                "🗒 turn summary — {model_calls} model calls, {tool_calls} tools, {turn_ms}ms"
                            ),
                        );
                        // （2.2 review 补修）summary 落账 = 上一轮的零输出判定
                        // 完结：此刻出队队头、置 in_flight 并交由锁外投递内核。
                        // 队列空（最后一轮）则 in_flight 保持 false。
                        // （fix-webui-qa-round11 1.2）出队即下一回合开轮：卡
                        // 相位回 OnIt（Working 真值随 Updated 帧下发）。
                        if advance_queue
                            && let Some(head) = if session.pending_queue.is_empty() {
                                None
                            } else {
                                Some(session.pending_queue.remove(0))
                            }
                        {
                            session.in_flight = true;
                            session.turn_visible_output = false;
                            session.card_phase = Some(CardPhase::OnIt);
                            dispatch = Some(head.text);
                        }
                        Some(SessionEvent::Updated {
                            session: session.info(&key, parked),
                        })
                    }
                    AE::Error {
                        message, terminal, ..
                    } => {
                        // ⚠ 错误行是操作员可见的 agent 产出——进 transcript 并
                        // 走 turn 流；terminal 错误随后拆除映射。
                        // （fix-webui-qa-round8 7.3）操作者主动取消（内核的
                        // 非 terminal "turn cancelled"）是**中性取消**不是失败：
                        // 落域层统一文案的中性 notice（与 ACP 引擎面同一份
                        // 常量），不再以 ⚠ 错误形态呈现。
                        if message == "turn cancelled" {
                            land(
                                session,
                                &turn_events,
                                &key,
                                "notice",
                                sebas_domain::session::TURN_CANCELLED_NOTICE.to_string(),
                            );
                        } else {
                            land(
                                session,
                                &turn_events,
                                &key,
                                "markdown",
                                format!("⚠ {message}"),
                            );
                        }
                        // workbench-interaction-polish 1.1：turn 终态（含取消
                        // 的非 terminal "turn cancelled"）复位在飞标志。
                        // （fix-webui-qa-round11 1.2）失败回合的卡相位真值 =
                        // CrossMark（SessionStatus::derive 派生 Failed）。取消
                        // 是中性操作，同样终回合——相位落 CrossMark 与 ACP 的
                        // 取消终态一致（可再开新一轮，开轮即回 OnIt）。
                        session.in_flight = false;
                        session.card_phase = Some(CardPhase::CrossMark);
                        // （add-local-usage-statistics 3.2）回合终态观察：非
                        // terminal 的「turn cancelled」= 操作者中性取消（499），
                        // 其余 Error（含 terminal 崩坏）= 失败（500）。
                        last_terminal = Some((
                            if !terminal && message == "turn cancelled" {
                                crate::usage_local::TURN_STATUS_CANCELLED
                            } else {
                                crate::usage_local::TURN_STATUS_FAILED
                            },
                            Some(message.clone()),
                        ));
                        // 回合已终：内核对悬空审批 fail-closed，泊车登记随之清除。
                        pending_approvals.write().await.remove(&encoded);
                        // （2.2 review 补修）回合已终：排队队头等 summary 落账
                        // 后由本 pump 出队投递（advance_queue）；terminal 错误
                        // 随会话拆除，队列整体清空、不出队。
                        if !terminal {
                            advance_queue = true;
                        }
                        removed = terminal;
                        None
                    }
                    AE::Finished { .. } => {
                        // 正文已逐 delta 落账（2.1），收尾无积压可 flush；
                        // Updated 仍照发——状态/段计数随快照刷新。
                        session.in_flight = false;
                        // （add-local-usage-statistics 3.2）回合终态观察：
                        // Finished = 完成（200）。
                        last_terminal = Some((crate::usage_local::TURN_STATUS_FINISHED, None));
                        // （fix-webui-qa-round11 1.2）正常结束的卡相位真值 =
                        // Done（rail/历史/计数随 derive 单点推进 done，native
                        // 会话不再永远 Queued）。
                        session.card_phase = Some(CardPhase::Done);
                        // 回合已终：悬空审批不再待决（fail-closed），清登记。
                        pending_approvals.write().await.remove(&encoded);
                        // （2.2 review 补修）回合已终：排队队头等 summary 落账
                        // 后由本 pump 出队投递（advance_queue）。
                        advance_queue = true;
                        Some(SessionEvent::Updated {
                            session: session.info(&key, parked),
                        })
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
            if dispatch.is_some() {
                // 锁外投递下一轮（队列推进；prompt 发送在内核 cmd 通道上等待）。
                // take() 复位标记——下一轮终态重新置位。
                let text = dispatch.take().expect("checked above");
                let g = sessions.read().await;
                if let Some(session) = g.get(&encoded) {
                    session.handle.prompt(text).await;
                }
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

    /// （fix-webui-qa-round12 4.2，D4）静默应用会话级模型 override：与
    /// [`SessionBackend::set_session_model`] 同一状态迁移（override 写入 +
    /// 内核下发），但**不落** `model_change` 回执条目——会话创建表单的模型
    /// 选定不是「切换」（spec：switch receipts are operator-driven）。返回
    /// None = 无留痕价值（创建时选定 = 内核缺省，from == to，无切换事实）。
    async fn apply_model_override(
        &self,
        key: &ChannelKey,
        model_id: &str,
    ) -> Result<Option<TurnEntry>, SessionRejection> {
        let encoded = Self::encode_key(key);
        let mut g = self.sessions.write().await;
        let Some(session) = g.get_mut(&encoded) else {
            return Err(SessionRejection::UnknownSession {
                key: encoded,
            });
        };
        let from = session
            .current_model_override
            .clone()
            .or_else(|| Some(session.default_model.clone()));
        session.current_model_override = Some(model_id.to_string());
        session.handle.set_model(model_id.to_string()).await;
        if from.as_deref() == Some(model_id) {
            return Ok(None);
        }
        let entry = TurnEntry::model_change(
            session.transcript.len() as u64,
            serde_json::json!({ "from": from, "to": model_id }),
        );
        session.transcript.push(entry.clone());
        Ok(Some(entry))
    }
}

#[async_trait::async_trait]
impl SessionBackend for NativeAgentBackend {
    async fn snapshot(&self) -> Vec<SessionInfo> {
        // 泊车数快照先行（读守卫随语句结束释放，不与 sessions 锁重叠持有）。
        let parked: std::collections::HashMap<String, u32> = self
            .pending_approvals
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.len() as u32))
            .collect();
        let g = self.sessions.read().await;
        let mut out: Vec<SessionInfo> = g
            .iter()
            .map(|(encoded, s)| {
                s.info(
                    &Self::decode_agent_key(encoded),
                    parked.get(encoded).copied().unwrap_or(0),
                )
            })
            .collect();
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

    /// 实时回合内容（fix-webui-streaming-liveness 2.1，D2）：native 面自己
    /// 承载 turn 流（pump 逐落账广播），不再依赖 ACP 桥。
    fn subscribe_turn_events(&self) -> broadcast::Receiver<TurnStreamEvent> {
        self.turn_events.subscribe()
    }

    async fn spawn(
        &self,
        prompt: String,
        project_dir: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        if let Some(cause) = &self.unavailable_cause {
            // fix-webui-detached-status：执行体侧拒绝不再冒充"核心不可达"。
            return Err(SessionRejection::BackendUnavailable {
                backend: "native".into(),
                cause: cause.clone(),
            });
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
        let key = ChannelKey::new(
            "feishu",
            format!("agent-{}", &handle.key[..8.min(handle.key.len())]),
        );
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
                    current_model_override: None,
                    available_models: self.available_models.clone(),
                    default_model: self.default_model.clone(),
                    // 首条 prompt 即开轮（串行队列空）；占位创建（prompt 空）
                    // 不开轮——首条真实消息经 message() 再开（review 修正：
                    // 空种子回合会在转录顶部留一条空 prompt 条目与空回显）。
                    in_flight: !prompt.trim().is_empty(),
                    // 开轮：可见输出计数从头开始（extend-test-model-scenarios
                    // 3.4，零输出回合补 notice 的判据）。
                    turn_visible_output: false,
                    // （2.2）首条提交立即开轮，不入影子队列。
                    pending_queue: Vec::new(),
                    // （fix-webui-qa-round11 1.2）带 prompt 的 spawn 即开轮：
                    // 卡相位 OnIt、首条消息即命名锚——rail 呈 Working 而非
                    // Queued；占位创建两者皆 None/空（诚实 Queued + 未命名）。
                    card_phase: (!prompt.trim().is_empty()).then_some(CardPhase::OnIt),
                    first_prompt_preview: (!prompt.trim().is_empty()).then_some(prompt.clone()),
                    // （add-local-usage-statistics 3.1）快照 usage 累计从零起步。
                    session_usage: None,
                },
            );
            // （fix-webui-qa-round8 2.1）首条 prompt 与 ACP `seed_card` 等价：
            // 投递内核前落 prompt 条目（操作者的第一条消息在转录里可见）。
            // 占位创建（空 prompt）不落条目、不开轮。
            if !prompt.trim().is_empty() {
                let session = g.get_mut(&encoded).expect("just inserted");
                let entry = session.push_prompt_entry(prompt.clone());
                NativeSession::broadcast_entry(&self.turn_events, &key, entry);
            }
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
        let turn_events = self.turn_events.clone();
        let notices = self.notices.clone();
        let pending = self.pending_approvals.clone();
        let pump_key = key.clone();
        let pump_encoded = encoded.clone();
        // （add-local-usage-statistics 3.2）落账 sink 在 pump 孵化时快照：
        // 装配点先 set 再 serve，会话 spawn 必然晚于注入。
        let pump_local_usage = self
            .local_usage
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        tokio::spawn(async move {
            Self::pump(
                rx, pump_key, pump_encoded, sessions, events, turn_events, notices, pending,
                pump_local_usage,
            )
            .await;
        });

        // First prompt drives the first turn（占位创建无 prompt：不开轮）。
        if !prompt.trim().is_empty() {
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
    ///
    /// （fix-webui-qa-round8 5.2 → fix-webui-qa-round12 4.2，D4）操作者显式
    /// 切换：落一条 `model_change` 系统条目（含新旧模型名）——回执只由
    /// 操作者切换产生；会话创建表单的模型选定走
    /// [`NativeAgentBackend::apply_model_override`]（静默，不产生「切换」
    /// 回执）。
    async fn set_session_model(
        &self,
        key: ChannelKey,
        model_id: String,
    ) -> Result<(), SessionRejection> {
        if let Some(entry) = self.apply_model_override(&key, &model_id).await? {
            NativeSession::broadcast_entry(&self.turn_events, &key, entry);
        }
        Ok(())
    }

    /// （fix-webui-qa-round8 2.1/2.2；2.2 review 补修）操作者消息投递：
    /// - 提交先落 prompt 条目（与 ACP seed_card 等价，转录不再只见回复）；
    /// - 空闲提交立即开轮（直投内核）；busy 提交**只入宿主影子队列、绝不
    ///   直送内核**——直送会让 remove/move 撤不掉已进内核串行队列的提交
    ///   （「已移除」的提交仍执行，spec 违约）。队头的实际投递归 pump 的
    ///   终态帧（回合结束 → 出队队头 → prompt），与 ACP 面的队列推进语义
    ///   对齐。
    async fn message(&self, key: ChannelKey, message: String) -> Result<(), SessionRejection> {
        let encoded = Self::encode_key(&key);
        // 泊车数先取（读守卫随语句释放；锁序 sessions → pending_approvals）。
        let parked = self.parked_count(&encoded).await;
        let (entry, info_frame, dispatch) = {
            // 写锁：in_flight 置位与会话查找同临界区（prompt 只借用 handle）。
            let mut g = self.sessions.write().await;
            let Some(session) = g.get_mut(&encoded) else {
                return Err(SessionRejection::UnknownSession { key: encoded });
            };
            // （fix-webui-qa-round11 1.2）首条消息落命名锚（后续消息绝不
            // 移动它），提交即回卡相位 OnIt——rail 立即呈 Working。
            if session
                .first_prompt_preview
                .as_deref()
                .unwrap_or("")
                .is_empty()
            {
                session.first_prompt_preview = Some(message.clone());
            }
            session.card_phase = Some(CardPhase::OnIt);
            // workbench-interaction-polish 1.1：空闲时这条 prompt 立即开轮。
            let mut dispatch: Option<String> = None;
            if !session.in_flight {
                // 空闲开轮 → 新一轮的零输出判据复位（extend-test-model-scenarios
                // 3.4）。
                session.turn_visible_output = false;
                dispatch = Some(message.clone());
            } else {
                // （fix-webui-qa-round8 2.2）busy 提交 = 宿主暂存（id 由后端
                // 单调源分发；disposition=Turn 与 ACP 面同词汇）。此刻不碰
                // 内核——remove/move 因此对队列内容有完整否决权。
                let id = self
                    .next_pending_id
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                session.pending_queue.push(PendingSubmission {
                    id,
                    text: message.clone(),
                    position: session.pending_queue.len(),
                    disposition: sebas_dispatch::PendingDisposition::Turn,
                    priority: false,
                });
            }
            session.in_flight = true;
            let entry = session.push_prompt_entry(message.clone());
            // 两条分支都发：排队帧带 pending 全量，空闲直投帧带开轮 working 真值
            // ——后续回合必须重新武装 turn_engaged，否则前端终点通知与 rail
            // working 翻转只在首回合成立（fix-webui-qa-round11 3c review）。
            let info_frame = Some(SessionEvent::Updated {
                session: session.info(&key, parked),
            });
            (entry, info_frame, dispatch)
        };
        NativeSession::broadcast_entry(&self.turn_events, &key, entry);
        if let Some(frame) = info_frame {
            let _ = self.events.send(frame);
        }
        if let Some(text) = dispatch {
            // 空闲直投（锁外：prompt 发送在内核 cmd 通道上等待）。
            let g = self.sessions.read().await;
            if let Some(session) = g.get(&encoded) {
                session.handle.prompt(text).await;
            }
        }
        Ok(())
    }

    /// （workbench-interaction-polish 1.1）native 会话的取消：未知 key 照旧
    /// typed 拒绝；已知会话按宿主在飞近似判定——空闲拒绝（不伪造成功），
    /// 在飞则下发内核既有 cancel（interrupt；空闲时内核本就无效果）。
    async fn cancel(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        let encoded = Self::encode_key(&key);
        let g = self.sessions.read().await;
        let Some(session) = g.get(&encoded) else {
            return Err(SessionRejection::UnknownSession { key: encoded });
        };
        if !session.in_flight {
            return Err(SessionRejection::Idle { key: encoded });
        }
        session.handle.cancel().await;
        Ok(())
    }

    async fn close(&self, key: ChannelKey) -> Result<CloseReport, SessionRejection> {
        let encoded = Self::encode_key(&key);
        let mut g = self.sessions.write().await;
        let Some(session) = g.remove(&encoded) else {
            return Err(SessionRejection::UnknownSession { key: encoded });
        };
        session.handle.cancel().await;
        // 会话拆除：泊车审批随会话消失（内核 fail-closed，不再待决）。
        self.pending_approvals.write().await.remove(&encoded);
        // （fix-webui-qa-round8 2.2）影子队列随会话终结清空（映射整体移除，
        // 这里语义自洽；计数不可得——close 的丢弃语义由 ACP 面承载，native
        // 影子队列如实返回 0）。
        Ok(CloseReport::default())
    }

    /// （fix-webui-qa-round8 2.2）native 待执行栈的读面 = 影子队列。未知会话
    /// typed 拒绝，与 ACP 面同契约。
    async fn pending(&self, key: ChannelKey) -> Result<Vec<PendingSubmission>, SessionRejection> {
        let encoded = Self::encode_key(&key);
        let g = self.sessions.read().await;
        let Some(session) = g.get(&encoded) else {
            return Err(SessionRejection::UnknownSession { key: encoded });
        };
        Ok(session.pending_view())
    }

    /// （fix-webui-qa-round8 2.2）按 id 移除一个未开始的排队提交，成功返回
    /// 操作后的全量影子队列。已知会话 + 未知 id = 类型化 Unknown（与 ACP
    /// 面同词表）；影子队列里的条目都未开始执行（执行中的不在栈上）。
    async fn remove_pending(
        &self,
        key: ChannelKey,
        pending_id: u64,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        let encoded = Self::encode_key(&key);
        let parked = self.parked_count(&encoded).await;
        let frame = {
            let mut g = self.sessions.write().await;
            let Some(session) = g.get_mut(&encoded) else {
                return Err(SessionRejection::UnknownSession { key: encoded });
            };
            let Some(pos) = session.pending_queue.iter().position(|p| p.id == pending_id) else {
                return Err(SessionRejection::PendingRejected {
                    reason: sebas_domain::session::PendingReason::Unknown,
                });
            };
            session.pending_queue.remove(pos);
            let _ = self.events.send(SessionEvent::Updated {
                session: session.info(&key, parked),
            });
            session.pending_view()
        };
        Ok(frame)
    }

    /// （fix-webui-qa-round8 2.2）把一个未开始的提交重排到影子队列的
    /// `to_index` 位置，成功返回操作后的全量影子队列（与 ACP 面同契约）。
    async fn move_pending(
        &self,
        key: ChannelKey,
        pending_id: u64,
        to_index: usize,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        let encoded = Self::encode_key(&key);
        let parked = self.parked_count(&encoded).await;
        let frame = {
            let mut g = self.sessions.write().await;
            let Some(session) = g.get_mut(&encoded) else {
                return Err(SessionRejection::UnknownSession { key: encoded });
            };
            let Some(from) = session.pending_queue.iter().position(|p| p.id == pending_id) else {
                return Err(SessionRejection::PendingRejected {
                    reason: sebas_domain::session::PendingReason::Unknown,
                });
            };
            if to_index >= session.pending_queue.len() {
                return Err(SessionRejection::PendingRejected {
                    reason: sebas_domain::session::PendingReason::OutOfRange,
                });
            }
            let moved = session.pending_queue.remove(from);
            session.pending_queue.insert(to_index, moved);
            let _ = self.events.send(SessionEvent::Updated {
                session: session.info(&key, parked),
            });
            session.pending_view()
        };
        Ok(frame)
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
            // A1.1: this seam only knows "the agent is unavailable right now"
            // — the generic runtime-down shape of the three-way enum.
            Some(cause) => Reachability::Disconnected {
                cause: cause.clone(),
            },
            None => Reachability::Reachable,
        }
    }

    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        Some(self.notices.subscribe())
    }

    /// 泊车审批读模型（native 侧）：登记表按会话枚举。未知会话 → typed
    /// rejection（路由转 404）；已知会话无泊车 = 空表。webui 打开/刷新会话
    /// 据此重建审批面——推送先于页面挂载到达的竞态由这一半边兜住（与 ACP
    /// 读模型同契约，fix-webui-approval-restore-and-session-identity 1.2）。
    async fn pending_approvals(
        &self,
        key: ChannelKey,
    ) -> Result<Vec<PendingApproval>, SessionRejection> {
        let encoded = Self::encode_key(&key);
        {
            let sessions = self.sessions.read().await;
            if !sessions.contains_key(&encoded) {
                return Err(SessionRejection::UnknownSession { key: encoded });
            }
        }
        let g = self.pending_approvals.read().await;
        Ok(g.get(&encoded)
            .map(|approvals| approvals.values().cloned().collect())
            .unwrap_or_default())
    }

    async fn answer_permission(&self, request_id: &str, decision: PermissionDecision) -> bool {
        // 决定词汇已合一（type-session-vocabularies 3.2）：这里不再有手写桥，
        // `PermissionDecision` 与 `ApprovalAnswer` 是**同一个**类型，直投即可。
        // 未知取值**不得**静默解决泊车审批（spec `agent-driver`）→ 如实拒绝投递，
        // 待决请求保持悬空。
        if !decision.is_answerable() {
            eprintln!(
                "agent_backend: 审批 {request_id} 收到未知决定 {:?}，已拒绝投递（审批保持悬空）",
                decision.as_str()
            );
            return false;
        }
        let delivered = self.hub.answer(request_id, decision);
        if delivered {
            // 决定已投递：从泊车登记移除（读模型不再枚举该请求；迟到推送 /
            // 陈旧读模型行由前端墓碑与 404 expired 语义兜住）。
            let mut g = self.pending_approvals.write().await;
            for approvals in g.values_mut() {
                approvals.remove(request_id);
            }
        }
        delivered
    }
}

/// The composite seam: one dashboard, two execution backends. Spawn routes on
/// the optional backend hint (`"native"` → the built-in kernel; anything else
/// → the Claude Code bridge); every other call routes on the key prefix
/// (`agent-*` chat ids belong to native sessions).
/// 节点参数里"真的在远端"的那个值（`None`/空/`local` = 本机）。
///
/// 与 `sebas_webui::session_backend` 的同名私有函数语义一致：本机标识是
/// `projects::LOCAL_NODE_ID`，两边必须同一个词。
fn remote_node_of(node: Option<&str>) -> Option<&str> {
    node.map(str::trim)
        .filter(|n| !n.is_empty() && *n != sebas_webui::projects::LOCAL_NODE_ID)
}

pub struct DualSessionBackend {
    pub acp: Arc<dyn SessionBackend>,
    pub native: Arc<NativeAgentBackend>,
    events: broadcast::Sender<SessionEvent>,
    /// 实时回合内容的合流出口（fix-webui-streaming-liveness 2.1，D2）：
    /// acp 桥与 native pump 两路 turn 流在此合成一条，webui WS 只订阅本后端。
    turn_events: broadcast::Sender<TurnStreamEvent>,
    /// Merged review-card notices from both children (acp + native), so a
    /// Claude/ACP permission request reaches the webui review card through the
    /// same channel as a native gated call.
    notices: broadcast::Sender<PermissionNotice>,
    /// （add-usage-statistics 2.2）config `[router] listen`：usage 聚合反代的
    /// loopback 目标（内嵌 webui 的 backend 就在本进程 = core，直接取数，与
    /// 通道服务端的 usage 分支共用 `router_admin` 同一实现）。`None` =
    /// router 未随部署启用（usage 请求如实回不可达）。
    usage_listen: Option<String>,
    /// （add-local-usage-statistics 2.1）本地用量库查询句柄：`source=local|all`
    /// 聚合在本进程直接执行（core = 本进程），与通道服务端共用
    /// `usage_local::usage_timeseries_outcome` 同一编排单点。`None` = 账本未
    /// 装配（测试装配）——local/all 如实回全零窗口。
    local_usage: Option<sebas_db::writer::StateHandle>,
}

impl DualSessionBackend {
    /// wire-webui-sebas-agent-e2e D4：给 acp 侧转发的事件打执行体标
    /// （native 侧的 info() 自带 `native` 标，无需在此处理）。
    fn stamp_acp_backend(ev: SessionEvent) -> SessionEvent {
        match ev {
            SessionEvent::Created { mut session } => {
                session.backend = Some("acp".into());
                SessionEvent::Created { session }
            }
            SessionEvent::Updated { mut session } => {
                session.backend = Some("acp".into());
                SessionEvent::Updated { session }
            }
            other => other,
        }
    }

    pub fn new(acp: Arc<dyn SessionBackend>, native: Arc<NativeAgentBackend>) -> Arc<Self> {
        Self::with_usage_sources(acp, native, None, None)
    }

    /// （add-usage-statistics 2.2 / add-local-usage-statistics 4.1）带 usage
    /// 双源取数面的构造：run.rs 装配内嵌 webui 时传入（core = 本进程，聚合
    /// 直接在本进程执行）。`local_usage` = 本地账本查询句柄；`usage_listen` =
    /// config `[router] listen`（`None` = router 未随部署启用）。
    pub fn with_usage_sources(
        acp: Arc<dyn SessionBackend>,
        native: Arc<NativeAgentBackend>,
        local_usage: Option<sebas_db::writer::StateHandle>,
        usage_listen: Option<String>,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        let (turn_events, _) = broadcast::channel(256);
        let (notices, _) = broadcast::channel(64);
        // Merge both children's lifecycle streams into one relay.
        {
            let tx = events.clone();
            let mut rx = acp.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let _ = tx.send(Self::stamp_acp_backend(ev));
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
        // fix-webui-streaming-liveness 2.1（D2）：合流两路 turn 流——acp 桥
        // （内嵌形态即 engine 的 transcript 广播）与 native pump。Lagged 只丢
        // 增量（快照收敛），不断链。
        {
            let tx = turn_events.clone();
            let mut rx = acp.subscribe_turn_events();
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
            let tx = turn_events.clone();
            let mut rx = native.subscribe_turn_events();
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
            turn_events,
            notices,
            usage_listen,
            local_usage,
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

    /// workbench-agent-wire-fix D2：wire 只认 agent id——`[acp.agents.*]`
    /// 配置键名（非空、不含命名空间分隔符）或保留值 `"native"`。旧词汇
    /// （`acp`/`acp:<slug>`/缺省）一律 typed rejection；id 是否已配置由
    /// ACP 侧的 agent kinds 把关，这里只看词汇形式。
    fn validate_agent_id(agent: &str) -> Result<(), SessionRejection> {
        let legacy = agent == "acp" || agent.starts_with("acp:");
        let valid = !legacy && (agent == "native" || (!agent.is_empty() && !agent.contains(':')));
        if valid {
            Ok(())
        } else {
            Err(SessionRejection::BackendUnavailable {
                backend: agent.to_string(),
                cause: "agent 必须是配置的 agent id 或 \"native\"（旧 backend 词汇已退役）".into(),
            })
        }
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
        // D4：acp 侧条目在快照出口统一打标（native 侧的 info() 自带 native 标）。
        for s in all.iter_mut() {
            if s.backend.is_none() {
                s.backend = Some("acp".into());
            }
        }
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

    // 聚焦即拉起（workbench-live-conversation-flow 3.1）：native 会话无
    // 子进程生命周期（内核即进程），activate 恒为无操作。
    async fn activate(&self, key: ChannelKey) -> Result<bool, SessionRejection> {
        if Self::is_native(&key) {
            return Ok(false);
        }
        self.acp.activate(key).await
    }

    // 回合内容流（fix-webui-streaming-liveness 2.1，D2）：acp 桥与 native
    // pump 两路在此合流（`new()` 里的中继任务），订阅端拿到单一出口。
    fn subscribe_turn_events(&self) -> broadcast::Receiver<sebas_dispatch::TurnStreamEvent> {
        self.turn_events.subscribe()
    }

    // make-core-own-provider-data 3.1：状态库域不属于任何一个执行体（provider
    // /aliases/settings/projects/presets 是 core 持有的共享数据）。复合后端
    // 必须转发到承载状态库的一侧（acp 桥，内嵌形态即 InProcessBackend→
    // engine），否则内嵌 webui 的 provider 管理面会误报不可达。native 侧
    // 不持有状态库。
    async fn state_snapshot(&self, domain: &str) -> Option<serde_json::Value> {
        self.acp.state_snapshot(domain).await
    }

    async fn state_mutate(&self, domain: &str, payload: serde_json::Value) -> Result<(), String> {
        self.acp.state_mutate(domain, payload).await
    }

    // add-fetch-models：抓取 op 与状态库域同归属（core 持有的共享数据面），
    // 复合后端转发到承载状态库的一侧。
    async fn fetch_provider_models(&self, provider: &str) -> Result<Vec<String>, String> {
        self.acp.fetch_provider_models(provider).await
    }

    /// （add-usage-statistics 2.2 / add-local-usage-statistics 4.1）usage 聚合
    /// 三口径：本复合后端跑在 core 进程内，与通道服务端的 usage 分支共用
    /// `usage_local::usage_timeseries_outcome` 同一编排单点（单一实现防两侧
    /// 漂移）。`source=router` 保持既有反代语义（不可达 → 结构化 cause）；
    /// `local` 纯本地聚合；`all` 本地聚合 + 尽力反代合并，router 不可达仍
    /// 200 且响应带 `router_cause`（`Ok` 半边，不是错误）。
    async fn usage_timeseries(
        &self,
        source: &str,
        granularity: &str,
        days: u32,
        tz_offset: i32,
    ) -> Result<serde_json::Value, sebas_webui::session_backend::UsageQueryError> {
        use sebas_webui::session_backend::UsageQueryError;
        let params = sebas_router::usage_query::parse_params(
            Some(granularity),
            Some(&days.to_string()),
            Some(&tz_offset.to_string()),
        )
        .map_err(|e| UsageQueryError::RouterError {
            status: 400,
            message: e.to_string(),
        })?;
        match crate::usage_local::usage_timeseries_outcome(
            self.local_usage.as_ref(),
            self.usage_listen.as_deref(),
            source,
            params,
            chrono::Utc::now(),
        )
        .await
        {
            crate::usage_local::UsageQueryOutcome::Ok(payload) => Ok(payload),
            crate::usage_local::UsageQueryOutcome::RouterError { status, body } => {
                Err(UsageQueryError::RouterError {
                    status,
                    message: body
                        .get("error")
                        .and_then(|e| e.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("router 应答状态 {status}")),
                })
            }
            crate::usage_local::UsageQueryOutcome::RouterUnreachable { cause } => {
                Err(UsageQueryError::RouterUnreachable { cause })
            }
        }
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
        agent: &str,
        model: Option<String>,
        mode: Option<String>,
        node: Option<String>,
    ) -> Result<ChannelKey, SessionRejection> {
        // 远端放置由 core 的节点链路负责（5.1）；这个复合后端只承载主控本机的
        // 两种执行体。远端节点到这里说明调用路径走错了——如实拒绝，不悄悄在本机
        // 建一个会话（那会让操作者以为任务跑在目标机器上）。
        if let Some(remote) = remote_node_of(node.as_deref()) {
            return Err(SessionRejection::Unavailable {
                cause: format!("进程内后端不承载远端会话放置（节点 {remote}）"),
            });
        }
        Self::validate_agent_id(agent)?;
        if agent == "native" {
            let key = self.native.spawn(prompt, project_dir).await?;
            // wire-webui-sebas-agent-e2e 4.2：创建时选定的模型对 native
            // 会话同样生效——走会话级 override 缝（作用于后续 turn 并
            // 反映在快照 current_model 上），不再被静默丢弃。会话刚由
            // 本调用建成，set 理论不会失败；万一失败也不否定已建成的
            // 会话。（fix-webui-qa-round12 4.2，D4）创建表单的模型选定走
            // 静默 override——不是「切换」，不落 model_change 回执。
            if let Some(m) = model {
                let _ = self.native.apply_model_override(&key, &m).await;
            }
            Ok(key)
        }
        // agent id 即 kind（D2）：ACP 侧按配置键名钉住 kind。模型 id
        // （add-acp-model-selection）随 spawn 透传。
        else {
            // 已经确认是本机会话，向下转发时节点参数就是 `None`。mode
            // （add-agent-mode-selection）对 ACP 侧随 spawn 透传（只有 claude
            // 驱动会把它落成 `--permission-mode`）。
            self.acp
                .spawn_with(prompt, project_dir, agent, model, mode, None)
                .await
        }
    }

    /// 0-turn placeholder: route to the backend that would own the session,
    /// so neither spawns an agent child for an empty prompt (P2).
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
                cause: format!("进程内后端不承载远端会话放置（节点 {remote}）"),
            });
        }
        Self::validate_agent_id(agent)?;
        if agent == "native" {
            let key = self.native.spawn(String::new(), project_dir).await?;
            // 与 spawn_with 同缝：占位会话记住创建时选定的模型（4.2）。
            if let Some(m) = model {
                let _ = self.native.set_session_model(key.clone(), m).await;
            }
            Ok(key)
        } else {
            self.acp
                .create_placeholder(project_dir, agent, model, mode, None)
                .await
        }
    }

    async fn message(&self, key: ChannelKey, message: String) -> Result<(), SessionRejection> {
        self.route(&key).message(key, message).await
    }

    async fn close(&self, key: ChannelKey) -> Result<CloseReport, SessionRejection> {
        self.route(&key).close(key).await
    }

    /// 归档恢复（fix-webui-qa-defects 2.2）：按 key 分发到归属执行体后端。
    /// （3.2）会话身份随恢复链路透传给归属后端；（round4 3.1）命名来源
    /// （label / prompt_preview）一并透传。
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
        self.route(&key)
            .restore_session(
                key,
                session_id,
                project_dir,
                transcript,
                identity,
                label,
                prompt_preview,
            )
            .await
    }

    /// 待批审批读模型（fix-webui-approval-restore-and-session-identity 1.2）：
    /// 按 key 分发到归属执行体后端。
    async fn pending_approvals(
        &self,
        key: ChannelKey,
    ) -> Result<Vec<PendingApproval>, SessionRejection> {
        self.route(&key).pending_approvals(key).await
    }

    // fix-webui-qa-defects-round5 1.1（D2）：待执行队列管理面按 key 转发到
    // 承载子后端（内嵌形态即 acp 桥的 InProcessBackend）。此前三者落入
    // trait 默认实现恒返 Unavailable——裸 core 内嵌 WebUI 的排队重排/移除
    // 恒 503 的根因。native 会话无队列：转发后落其诚实类型化失败
    // （pending 空表 / remove·move 的 Unavailable），不在此层另造说辞。
    async fn pending(&self, key: ChannelKey) -> Result<Vec<PendingSubmission>, SessionRejection> {
        self.route(&key).pending(key).await
    }

    async fn remove_pending(
        &self,
        key: ChannelKey,
        pending_id: u64,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        self.route(&key).remove_pending(key, pending_id).await
    }

    async fn move_pending(
        &self,
        key: ChannelKey,
        pending_id: u64,
        to_index: usize,
    ) -> Result<Vec<PendingSubmission>, SessionRejection> {
        self.route(&key).move_pending(key, pending_id, to_index).await
    }

    /// 会话命名（fix-webui-approval-restore-and-session-identity 5.1）：按 key
    /// 分发到归属执行体后端。
    async fn set_session_label(
        &self,
        key: ChannelKey,
        label: Option<String>,
    ) -> Result<(), SessionRejection> {
        self.route(&key).set_session_label(key, label).await
    }

    async fn cancel(&self, key: ChannelKey) -> Result<(), SessionRejection> {
        self.route(&key).cancel(key).await
    }

    async fn set_session_model(
        &self,
        key: ChannelKey,
        model_id: String,
    ) -> Result<(), SessionRejection> {
        // wire-webui-sebas-agent-e2e：按 key 分发。原生 key 路由到内核，
        // ACP key 走既有 InProcessBackend 的 Out::SendAcp SetModel。
        self.route(&key).set_session_model(key, model_id).await
    }

    /// （add-agent-mode-selection）按 key 分发：ACP key 走 InProcessBackend
    /// 的 `Out::SendAcp SetMode`（claude 驱动运行时切换）；native key 落到
    /// 内核后端的诚实不可用（native 不承载 mode）。
    async fn set_session_mode(
        &self,
        key: ChannelKey,
        mode: String,
    ) -> Result<(), SessionRejection> {
        self.route(&key).set_session_mode(key, mode).await
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

    async fn execution_bodies(
        &self,
    ) -> Option<Vec<sebas_webui::session_backend::ExecutionBodyStatus>> {
        // A1.1：三类不可达对逐体上报同义——body 不可用，cause 原样透传。
        let to_body = |name: &str, r: Reachability| match r {
            Reachability::Reachable => sebas_webui::session_backend::ExecutionBodyStatus {
                name: name.into(),
                ok: true,
                cause: None,
            },
            Reachability::StartupFailed { cause }
            | Reachability::AuthRejected { cause }
            | Reachability::Disconnected { cause } => {
                sebas_webui::session_backend::ExecutionBodyStatus {
                    name: name.into(),
                    ok: false,
                    cause: Some(cause),
                }
            }
        };
        let acp = to_body("acp", self.acp.reachability().await);
        let native = to_body("native", self.native.reachability().await);
        Some(vec![acp, native])
    }

    fn permission_requests(&self) -> Option<broadcast::Receiver<PermissionNotice>> {
        Some(self.notices.subscribe())
    }

    async fn answer_permission(&self, request_id: &str, decision: PermissionDecision) -> bool {
        if self
            .native
            .answer_permission(request_id, decision.clone())
            .await
        {
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
    use sebas_acp::claude::session::{AcpCommand, AcpEvent, Decision};
    use sebas_agent::llm::fake::FakeLlmClient;
    use sebas_agent::policy::NetworkMode;
    use sebas_channels::ChannelKey;
    use sebas_dispatch::engine::Out;
    use sebas_dispatch::state::{Mapping, SessionMap};

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
        // 会话键是 canonical URL-safe 编码形（webui 路由/turn 流/ACP 通路的
        // 同一 wire 形，webui-ws-rpc 契约）；JSON 字符串形会让 review-card
        // 的精确匹配永远丢弃 native 审批卡。
        assert_eq!(
            notice.session_id,
            sebas_channels::key::encode_session_key(&key),
            "session id is the canonical encoded key"
        );
        assert!(
            notice.session_id.starts_with("feishu%00agent-"),
            "native notice session_id must be the URL-safe encoded shape, got {:?}",
            notice.session_id
        );
        // 泊车审批读模型：决策前枚举到该请求，决策投递后清空。
        let parked = backend.pending_approvals(key.clone()).await.expect("read model");
        assert_eq!(parked.len(), 1, "the gated call is parked");
        assert_eq!(parked[0].request_id, notice.request_id);
        assert_eq!(parked[0].tool_name, "bash");
        assert!(
            backend
                .answer_permission(&notice.request_id, PermissionDecision::AllowOnce)
                .await,
            "answer must reach the pending request"
        );
        let parked = backend.pending_approvals(key.clone()).await.expect("read model");
        assert!(parked.is_empty(), "answered request must leave the registry");

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
        assert!(
            joined.contains("bash"),
            "tool trace in transcript: {joined}"
        );
        assert!(
            joined.contains("policy"),
            "policy event in transcript: {joined}"
        );
        assert!(
            joined.contains("gated call was approved"),
            "completion text in transcript: {joined}"
        );

        // close 后 sessions 清空。
        assert!(backend.close(key).await.is_ok());
        assert!(backend.snapshot().await.is_empty());
    }

    // （session-slash-commands 2.3）native 路径确认：AgentEvent 词汇不动、
    // native 会话快照的命令表恒空——composer 据此不渲染命令面板、`/` 前缀
    // 按普通文本放行（诚实退化）。
    #[tokio::test]
    async fn native_session_snapshot_has_no_command_surface() {
        let backend = NativeAgentBackend::with_manager(manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");
        let info = backend
            .snapshot()
            .await
            .into_iter()
            .find(|s| s.channel_key() == key)
            .expect("native session in snapshot");
        assert!(
            info.available_commands.is_empty(),
            "native sessions must never advertise commands: {:?}",
            info.available_commands
        );
        backend.close(key).await.unwrap();
    }

    /// 纯文本脚本 manager：一回合一段收尾文本，无工具、无审批卡。
    fn plain_manager() -> SessionManager {
        let llm = FakeLlmClient::scripted(vec![FakeLlmClient::say("turn answer")]);
        SessionManager::new(
            Arc::new(llm),
            ToolRegistry::with_sandbox(
                Duration::from_secs(10),
                sebas_agent::policy::SandboxMode::Firewall,
            ),
            SessionConfig::default(),
        )
        .with_policy(Arc::new(PolicyEngine::new(PolicyConfig::default())))
        .with_approver(sebas_agent::policy::ApproverHub::new())
    }

    async fn info_for(backend: &NativeAgentBackend, key: &ChannelKey) -> SessionInfo {
        backend
            .snapshot()
            .await
            .into_iter()
            .find(|s| s.channel_key() == *key)
            .expect("native session in snapshot")
    }

    /// （fix-webui-qa-round11 1.2，design D1）native 生命周期真值回填：
    /// 开轮 → 卡相位 OnIt（`SessionStatus::derive` 读 Working、turn_engaged
    /// 真——composer 停止钮的数据源）；正常结束 → Done；首条消息落命名锚
    /// 且后续消息绝不移动它。派生保持 `SessionStatus::derive` 单点。
    #[tokio::test]
    async fn native_session_backfills_card_phase_and_title() {
        let backend = NativeAgentBackend::with_manager(plain_manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("first message".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");

        // 开轮即 OnIt：spawn 返回后快照读 Working（不是 Queued）。
        let info = info_for(&backend, &key).await;
        assert_eq!(info.phase, Some(CardPhase::OnIt), "spawn 开轮即 OnIt");
        assert_eq!(info.status, sebas_domain::session::SessionPhase::Active);
        assert_eq!(
            sebas_webui::models::SessionStatus::derive(&info.status, info.phase.as_ref()),
            sebas_webui::models::SessionStatus::Working,
        );
        assert!(info.turn_engaged, "在飞事实随开轮置真（停止钮数据源）");
        assert_eq!(
            info.first_prompt_preview.as_deref(),
            Some("first message"),
            "首条消息即命名锚"
        );

        // 回合正常结束 → Done、在飞复位；命名锚不动。
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if info_for(&backend, &key).await.phase == Some(CardPhase::Done) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("turn must settle to Done");
        let info = info_for(&backend, &key).await;
        assert_eq!(
            sebas_webui::models::SessionStatus::derive(&info.status, info.phase.as_ref()),
            sebas_webui::models::SessionStatus::Done,
            "native 会话结束后 derive 单点读 Done"
        );
        assert!(!info.turn_engaged, "终态在飞复位");
        assert_eq!(info.first_prompt_preview.as_deref(), Some("first message"));

        // 第二条消息：开新一轮（回 OnIt），命名锚仍是首条。
        backend
            .message(key.clone(), "second message".into())
            .await
            .expect("message");
        let info = info_for(&backend, &key).await;
        assert_eq!(info.phase, Some(CardPhase::OnIt));
        assert!(info.turn_engaged);
        assert_eq!(
            info.first_prompt_preview.as_deref(),
            Some("first message"),
            "后续消息绝不移动命名锚"
        );
        backend.close(key).await.unwrap();
    }

    /// 占位创建（空 prompt）诚实呈 Queued（无相位、未命名、不在飞）；
    /// 首条 `message()` 落相位与命名锚——rail/历史/计数不再被恒空输入钉死。
    #[tokio::test]
    async fn placeholder_native_session_stays_queued_until_first_message() {
        let backend = NativeAgentBackend::with_manager(plain_manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn(String::new(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");

        let info = info_for(&backend, &key).await;
        assert_eq!(info.phase, None, "占位创建无回合 → 无卡相位");
        assert_eq!(
            sebas_webui::models::SessionStatus::derive(&info.status, info.phase.as_ref()),
            sebas_webui::models::SessionStatus::Queued,
        );
        assert!(!info.turn_engaged);
        assert!(info.first_prompt_preview.is_none(), "尚无消息 → 未命名");

        backend
            .message(key.clone(), "hello".into())
            .await
            .expect("message");
        let info = info_for(&backend, &key).await;
        assert_eq!(info.phase, Some(CardPhase::OnIt));
        assert_eq!(
            sebas_webui::models::SessionStatus::derive(&info.status, info.phase.as_ref()),
            sebas_webui::models::SessionStatus::Working,
        );
        assert_eq!(info.first_prompt_preview.as_deref(), Some("hello"));
        backend.close(key).await.unwrap();
    }

    // rail-declutter-unread 1.1（native 侧）：msg_count 在 transcript flush
    // 处累计——一回合的流式正文（一次 flush）计 1，工具痕迹（📖/✓）不计；
    // 第二回合再 +1。badge 数据源的单测钉住口径。
    #[tokio::test]
    async fn native_msg_count_counts_reply_flushes_not_tool_noise() {
        let backend = NativeAgentBackend::with_manager(manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");

        // 回合一：bash 工具调用（gated，先放行；2 条痕迹）+ 收尾文本（1 次
        // flush）。
        let mut notices = backend.permission_requests().expect("native has notices");
        let notice = tokio::time::timeout(Duration::from_secs(10), notices.recv())
            .await
            .expect("notice timeout")
            .expect("notice");
        assert!(
            backend
                .answer_permission(&notice.request_id, PermissionDecision::AllowOnce)
                .await,
            "decision must reach the pending request"
        );
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
        let info = backend
            .snapshot()
            .await
            .into_iter()
            .find(|s| s.channel_key() == key)
            .expect("native session in snapshot");
        // fold-tool-calls-into-process-tree 3.3：工具痕迹升为一等 tool 条目
        // 后，`count_chat_messages` 对它与 ACP 面同口径——tool 不计数但打断
        // markdown 连续段。段落变为：⏳ 审批痕迹一段 + 🛡 策略痕迹与收尾正文
        // 一段 = 2（工具痕迹自身恒 0——「tool traces must not add」不回归；
        // 增量来自 markdown 段被 tool 打断，非工具计数）。
        assert_eq!(
            info.msg_count, 2,
            "tool traces add nothing; the approval trace and the policy+reply run count one each"
        );

        backend.close(key).await.unwrap();
    }

    #[tokio::test]
    async fn dual_routes_on_backend_hint_and_prefix() {
        let acp: Arc<dyn SessionBackend> = Arc::new(
            sebas_webui::session_backend::InProcessBackend::new(make_router().await),
        );
        let dual = DualSessionBackend::new(acp, NativeAgentBackend::with_manager(manager()));
        // backend hint = native → key 前缀 agent-；创建时选定的模型随 spawn
        // 生效于会话级 override（4.2：选中生效于快照）。
        let key = dual
            .spawn_with(
                "go".into(),
                None,
                "native",
                Some("m-spawn".into()),
                None,
                None,
            )
            .await
            .expect("spawn native");
        assert!(DualSessionBackend::is_native(&key), "{:?}", key.reference);
        let info = dual
            .snapshot()
            .await
            .into_iter()
            .find(|s| s.channel_key() == key)
            .expect("native session in snapshot");
        assert_eq!(
            info.current_model.as_deref(),
            Some("m-spawn"),
            "spawn-time model must land in the session snapshot"
        );
        // 默认（无 hint）→ acp 路径：agent 前缀之外的 key。
        // D2：agent 必填——"claude" 在测试配置中不存在，但词汇合法；路由
        // 到 acp 侧（key 非 native 前缀即为断言点）。
        let acp_key = dual
            .spawn_with("hi".into(), None, "claude", None, None, None)
            .await
            .expect("spawn acp");
        assert!(!DualSessionBackend::is_native(&acp_key));
    }

    // wire-webui-sebas-agent-e2e 2.2：set_session_model 按 key 分发——native
    // key 命中内核（override 反映在快照 current_model），unknown key 返回
    // typed rejection，不再无条件转发 ACP。
    #[tokio::test]
    async fn dual_set_session_model_routes_native_key_and_rejects_unknown() {
        let acp: Arc<dyn SessionBackend> = Arc::new(
            sebas_webui::session_backend::InProcessBackend::new(make_router().await),
        );
        let dual = DualSessionBackend::new(acp, NativeAgentBackend::with_manager(manager()));

        // native key：spawn（隔离 workdir）→ 放行 gated 调用 → 等 turn 收尾，
        // 让 set_model 走内核空闲期路径（turn 中收下、下一 turn 才生效）。
        let ws = tempfile::tempdir().unwrap();
        let key = dual
            .spawn_with(
                "go".into(),
                Some(ws.path().to_string_lossy().into()),
                "native",
                None,
                None,
                None,
            )
            .await
            .expect("spawn native");
        let mut notices = dual.permission_requests().expect("dual has notices");
        let notice = tokio::time::timeout(Duration::from_secs(10), notices.recv())
            .await
            .expect("notice timeout")
            .expect("notice");
        assert!(
            dual.answer_permission(&notice.request_id, PermissionDecision::AllowOnce)
                .await,
            "decision must reach the pending request"
        );
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let turns = dual.turns(key.clone(), 0).await.unwrap();
                let joined: String = turns.iter().map(|t| t.content.clone()).collect();
                if joined.contains("gated call was approved") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;

        dual.set_session_model(key.clone(), "m-kernel".into())
            .await
            .expect("native set_model must reach the kernel");
        let info = dual
            .snapshot()
            .await
            .into_iter()
            .find(|s| s.channel_key() == key)
            .expect("native session in snapshot");
        assert_eq!(
            info.current_model.as_deref(),
            Some("m-kernel"),
            "override must show up in the session snapshot"
        );

        // unknown key（native 前缀但不存在）→ typed rejection，且不建会话。
        let bogus = ChannelKey::new("feishu", "agent-doesnotexist");
        match dual.set_session_model(bogus, "m-x".into()).await {
            Err(SessionRejection::UnknownSession { .. }) => {}
            other => panic!("expected UnknownSession, got {other:?}"),
        }
        assert_eq!(dual.snapshot().await.len(), 1, "no session may be created");
    }

    // fix-webui-qa-defects-round5 1.1：pending 管理面按 key 转发。acp 会话的
    // move/remove 到达 acp 桥（类型化 PendingRejected——转发前会落在复合层
    // 默认 Unavailable、WebUI 恒 503）；native 会话无队列，得到其诚实失败
    // （pending 空表 / remove·move 的 Unavailable）。
    #[tokio::test]
    async fn dual_pending_management_routes_by_key() {
        let acp: Arc<dyn SessionBackend> = Arc::new(
            sebas_webui::session_backend::InProcessBackend::new(make_router().await),
        );
        let dual = DualSessionBackend::new(acp, NativeAgentBackend::with_manager(manager()));

        // acp 侧（web key 路由到 InProcessBackend）：未知会话/id 同形为
        // PendingRejected::Unknown（桥的类型化拒绝原样透传）。
        let acp_key = ChannelKey::new("web", "web-round5-pending");
        for op in [
            dual.remove_pending(acp_key.clone(), 7).await.err().unwrap(),
            dual.move_pending(acp_key.clone(), 7, 0).await.err().unwrap(),
        ] {
            assert!(
                matches!(op, SessionRejection::PendingRejected { .. }),
                "acp-hosted session must reach the acp bridge, got {op:?}"
            );
        }
        assert!(
            dual.pending(acp_key).await.unwrap().is_empty(),
            "acp-side pending read must come from the bridge, not a composite default"
        );

        // native 侧（fix-webui-qa-round8 2.2）：pending 管理面 = 宿主影子
        // 队列——未知会话一律类型化 UnknownSession（不再落 trait 默认的
        // Unavailable）；已存在会话无排队提交 = 空表。
        let native_key = ChannelKey::new("feishu", "agent-round5native");
        match dual.pending(native_key.clone()).await {
            Err(SessionRejection::UnknownSession { .. }) => {}
            other => panic!("unknown native session must reject, got {other:?}"),
        }
        for op in [
            dual.remove_pending(native_key.clone(), 7)
                .await
                .err()
                .unwrap(),
            dual.move_pending(native_key.clone(), 7, 0)
                .await
                .err()
                .unwrap(),
        ] {
            match &op {
                SessionRejection::UnknownSession { .. } => {}
                other => panic!("unknown native session must reject queue ops, got {other:?}"),
            }
        }
    }

    // workbench-agent-wire-fix D2：wire 词汇收紧后，旧 backend 值与非法
    // agent id 一律 typed rejection 且不建会话。
    #[tokio::test]
    async fn invalid_agent_values_reject_without_session() {
        let acp: Arc<dyn SessionBackend> = Arc::new(
            sebas_webui::session_backend::InProcessBackend::new(make_router().await),
        );
        let dual = DualSessionBackend::new(acp, NativeAgentBackend::with_manager(manager()));

        // 词汇形式非法：driver 命名空间残留（acp:*）、空串。
        for agent in ["acp", "acp:claude", ""] {
            let err = dual
                .spawn_with("hi".into(), None, agent, None, None, None)
                .await
                .expect_err("legacy/empty agent value must reject");
            match &err {
                SessionRejection::BackendUnavailable { backend, .. } => {
                    assert_eq!(backend, agent);
                }
                other => panic!("expected BackendUnavailable, got {other:?}"),
            }
            assert!(
                err.to_string().contains("执行体不可用"),
                "wording must name the backend, not the core: {err}"
            );
        }
        // 占位创建同受校验；且全程未产生任何会话。
        assert!(
            dual.create_placeholder(None, "acp:claude", None, None, None)
                .await
                .is_err()
        );
        assert!(
            dual.snapshot().await.is_empty(),
            "no session may be created"
        );

        // 合法 agent id（含大小写敏感的普通 id、native）放行路由。
        for agent in ["claude", "native"] {
            dual.spawn_with("hi".into(), None, agent, None, None, None)
                .await
                .unwrap_or_else(|e| panic!("agent {agent:?} must route: {e}"));
        }
    }

    // fix-webui-detached-status 1.2：native 缺凭据的拒绝指名执行体，
    // 不再复用"核心不可达"文案。
    #[tokio::test]
    async fn native_missing_credentials_rejection_names_the_backend() {
        let acp: Arc<dyn SessionBackend> = Arc::new(
            sebas_webui::session_backend::InProcessBackend::new(make_router().await),
        );
        let native = NativeAgentBackend::with_manager_arc(
            Arc::new(manager()),
            Some(
                "native backend needs SEBAS_AGENT_PROVIDER_API_KEY (or SEBAS_AGENT_ROUTER_URL)"
                    .into(),
            ),
            vec!["claude-sonnet-4-5".into()],
            "claude-sonnet-4-5".into(),
        );
        let dual = DualSessionBackend::new(acp, native);

        let err = dual
            .spawn_with("hi".into(), None, "native", None, None, None)
            .await
            .expect_err("native without credentials must reject");
        let text = err.to_string();
        assert!(text.contains("执行体不可用: native"), "{text}");
        assert!(!text.contains("核心不可达"), "{text}");
        assert!(dual.snapshot().await.is_empty());
    }

    /// 出站接收端必须保活：router 发送在通道关闭时会 panic。
    async fn make_router() -> sebas_dispatch::DispatchHandle {
        let (router, mut out_rx) =
            sebas_dispatch::DispatchHandle::new(sebas_dispatch::SessionMap::new());
        tokio::spawn(async move { while out_rx.recv().await.is_some() {} });
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
        let (router, mut out_rx) = sebas_dispatch::DispatchHandle::new(map);

        let acp: Arc<dyn SessionBackend> = Arc::new(
            sebas_webui::session_backend::InProcessBackend::new(router.clone()),
        );
        let dual =
            DualSessionBackend::new(acp.clone(), NativeAgentBackend::with_manager(manager()));

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

    // ── fix-webui-streaming-liveness 2.1：pump 期间可见增量事件 ────────────

    /// 一轮两段文本的脚本 turn：kernel 逐段回调 TextDelta，宿主 pump 逐条
    /// 落账并广播 turn 事件。
    fn two_chunk_turn() -> sebas_agent::llm::LlmTurn {
        sebas_agent::llm::LlmTurn {
            content: vec![
                sebas_agent::message::ContentBlock::Text {
                    text: "chunk one ".into(),
                },
                sebas_agent::message::ContentBlock::Text {
                    text: "chunk two".into(),
                },
            ],
            stop_reason: sebas_agent::llm::StopReason::EndTurn,
            // （add-local-usage-statistics 3.1）脚本 turn 不上报 usage（全
            // None = 只计请求数的诚实形态）。
            usage: Default::default(),
        }
    }

    #[tokio::test]
    async fn native_pump_streams_turn_events_during_the_turn() {
        // D2：delta 落账即广播——两段文本作为**独立帧**先于回合收尾标记
        // （🗒 turn summary，kernel 回合结束才发）到达，position 单调、内容与
        // transcript 一致（文本不再攒到回合边界整块 flush——旧实现下两段会
        // 合成收尾时刻的一条整块条目，turn 流压根不存在）。
        let manager = {
            let llm = FakeLlmClient::scripted(vec![two_chunk_turn()]);
            SessionManager::new(
                Arc::new(llm),
                ToolRegistry::with_sandbox(Duration::from_secs(10), SandboxMode::Firewall),
                SessionConfig::default(),
            )
        };
        let backend = NativeAgentBackend::with_manager(manager);
        // 先订阅 turn 流，再 spawn（broadcast 只转发订阅后的事件）。
        let mut turns = backend.subscribe_turn_events();
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");

        let deadline = Duration::from_secs(10);
        let mut streamed: Vec<TurnEntry> = Vec::new();
        loop {
            let event = tokio::time::timeout(deadline, turns.recv())
                .await
                .expect("event timeout")
                .expect("turn stream open");
            for entry in event.entries {
                let is_summary = entry.content.contains("turn summary");
                streamed.push(entry);
                if is_summary {
                    // 回合收尾标记到达：流式窗口结束。
                    break;
                }
            }
            if streamed.iter().any(|e| e.content.contains("turn summary")) {
                break;
            }
        }

        // 两段文本 = 两条独立 turn 事件，先于收尾标记到达。汇总行只钉前缀：
        // 耗时字段随机器负载波动（0/1ms），精确匹配会间歇性翻红。
        // （fix-webui-qa-round8 2.1）spawn 的 prompt 条目先行（position 0），
        // 两段正文增量与其后各后移一位。
        let contents: Vec<&str> = streamed.iter().map(|e| e.content.as_str()).collect();
        assert_eq!(
            &contents[..3],
            &["go", "chunk one ", "chunk two"],
            "prompt first, then deltas as separate live entries: {streamed:?}"
        );
        assert!(
            contents[3].starts_with("🗒 turn summary — 1 model calls, 0 tools, ")
                && contents[3].ends_with("ms"),
            "turn-end marker content unexpected: {streamed:?}"
        );
        assert_eq!(
            streamed.iter().map(|e| e.position).collect::<Vec<_>>(),
            vec![0, 1, 2, 3],
            "positions assigned monotonically"
        );

        // transcript（快照面）与 turn 流完全一致——同一段代码产出。
        let snapshotted = backend.turns(key.clone(), 0).await.unwrap();
        assert_eq!(snapshotted, streamed, "turn stream == transcript");

        backend.close(key).await.unwrap();
    }

    // ── extend-test-model-scenarios 3.3/3.4：thinking 落账与零输出提示 ──────

    /// 一段 thinking + 一段正文的脚本 turn（`test/thinking` 的 wire 形状）。
    fn thinking_turn() -> sebas_agent::llm::LlmTurn {
        sebas_agent::llm::LlmTurn {
            content: vec![
                sebas_agent::message::ContentBlock::Thinking {
                    thinking: "weighing the options".into(),
                },
                sebas_agent::message::ContentBlock::Text {
                    text: "final answer".into(),
                },
            ],
            stop_reason: sebas_agent::llm::StopReason::EndTurn,
            usage: Default::default(),
        }
    }

    /// 收集一轮的 turn 流（到 turn summary 收尾标记为止）。
    async fn collect_turn(
        turns: &mut broadcast::Receiver<TurnStreamEvent>,
    ) -> Vec<TurnEntry> {
        let deadline = Duration::from_secs(10);
        let mut streamed: Vec<TurnEntry> = Vec::new();
        loop {
            let event = tokio::time::timeout(deadline, turns.recv())
                .await
                .expect("event timeout")
                .expect("turn stream open");
            let mut done = false;
            for entry in event.entries {
                if entry.content.contains("turn summary") {
                    done = true;
                }
                streamed.push(entry);
            }
            if done {
                return streamed;
            }
        }
    }

    fn backend_with(llm: FakeLlmClient) -> Arc<NativeAgentBackend> {
        let manager = SessionManager::new(
            Arc::new(llm),
            ToolRegistry::with_sandbox(Duration::from_secs(10), SandboxMode::Firewall),
            SessionConfig::default(),
        );
        NativeAgentBackend::with_manager(manager)
    }

    /// （add-local-usage-statistics 3.1）快照 `usage` 如实携带：上报了 token
    /// 的回合把 summary 累计折算进 `SessionInfo.usage`（芯片形状）；从未上报
    /// 的会话保持 `None`（门控语义：不冒充全零）。
    #[tokio::test]
    async fn native_session_snapshot_carries_reported_usage_and_none_otherwise() {
        let backend = backend_with(FakeLlmClient::scripted(vec![
            FakeLlmClient::say_with_usage("done", 5, 8),
        ]));
        let mut turns = backend.subscribe_turn_events();
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");
        // 等回合收尾标记（🗒 summary 落账先于其 Updated 广播）。
        let deadline = Duration::from_secs(10);
        loop {
            let event = tokio::time::timeout(deadline, turns.recv())
                .await
                .expect("event timeout")
                .expect("turn stream open");
            if event
                .entries
                .iter()
                .any(|e| e.content.contains("turn summary"))
            {
                break;
            }
        }
        let snapshotted = backend.snapshot().await;
        let mine = snapshotted
            .iter()
            .find(|s| s.key == key.reference)
            .expect("native session in snapshot");
        let usage = mine
            .usage
            .as_ref()
            .expect("reported usage rides the snapshot");
        assert_eq!(
            usage.model.as_deref(),
            Some(SessionConfig::default().model.as_str())
        );
        assert_eq!(usage.total_input, 5);
        assert_eq!(usage.total_output, 8);

        // 对照组：无 usage 的回合，快照 usage 保持缺席。
        let backend2 = backend_with(FakeLlmClient::scripted(vec![two_chunk_turn()]));
        let mut turns2 = backend2.subscribe_turn_events();
        let ws2 = tempfile::tempdir().unwrap();
        let key2 = backend2
            .spawn("go".into(), Some(ws2.path().to_string_lossy().into()))
            .await
            .expect("spawn");
        loop {
            let event = tokio::time::timeout(deadline, turns2.recv())
                .await
                .expect("event timeout")
                .expect("turn stream open");
            if event
                .entries
                .iter()
                .any(|e| e.content.contains("turn summary"))
            {
                break;
            }
        }
        let snapshotted = backend2.snapshot().await;
        let mine = snapshotted
            .iter()
            .find(|s| s.key == key2.reference)
            .expect("native session in snapshot");
        assert!(
            mine.usage.is_none(),
            "unreported session must not fake zeros: {:?}",
            mine.usage
        );
    }

    #[tokio::test]
    async fn native_pump_projects_thinking_as_its_own_entry() {
        // 3.3：thinking delta 落一条 `element_type = "thinking"` 条目（此前
        // 原生面直接丢弃），顺序在正文之前，且不计入可见回复段数。
        let backend = backend_with(FakeLlmClient::scripted(vec![thinking_turn()]));
        let mut turns = backend.subscribe_turn_events();
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");

        let streamed = collect_turn(&mut turns).await;
        let kinds: Vec<(&str, &str)> = streamed
            .iter()
            .map(|e| (e.element_type.as_str(), e.content.as_str()))
            .collect();
        // （fix-webui-qa-round8 2.1）spawn 的 prompt 条目先行，其后 thinking
        // 仍以独立条目先于正文落账。
        assert_eq!(
            &kinds[..3],
            &[
                ("markdown", "go"),
                ("thinking", "weighing the options"),
                ("markdown", "final answer")
            ],
            "thinking must land as its own entry before the text: {streamed:?}"
        );
        // 汇总行的耗时字段随机器负载波动（0/1ms），只钉前缀不钉 ms 值。
        let summary = kinds[3].1;
        assert!(
            kinds[3].0 == "markdown"
                && summary.starts_with("🗒 turn summary — 1 model calls, 0 tools, ")
                && summary.ends_with("ms"),
            "turn summary entry unexpected: {streamed:?}"
        );
        // 可见回复段数（派生口径）不数 thinking。
        let snapshotted = backend.turns(key.clone(), 0).await.unwrap();
        assert_eq!(snapshotted, streamed, "turn stream == transcript");
        backend.close(key).await.unwrap();
    }

    #[tokio::test]
    async fn native_pump_appends_zero_output_notice_for_a_silent_turn() {
        // 3.4：回合正常收尾但零可见输出 → 在收尾标记之前补一条 `notice`
        // 合成提示（与 ACP 引擎面同文案、同 element_type）。
        let silent = sebas_agent::llm::LlmTurn {
            content: vec![],
            stop_reason: sebas_agent::llm::StopReason::EndTurn,
            usage: Default::default(),
        };
        let backend = backend_with(FakeLlmClient::scripted(vec![silent]));
        let mut turns = backend.subscribe_turn_events();
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");

        let streamed = collect_turn(&mut turns).await;
        // （fix-webui-qa-round8 2.1）spawn 的 prompt 条目先行：通知与摘要各后移一位。
        assert_eq!(streamed.len(), 3, "prompt, notice then summary: {streamed:?}");
        assert_eq!(streamed[0].kind, sebas_domain::session::TurnKind::Prompt);
        assert_eq!(streamed[1].element_type, sebas_domain::vocabulary::TurnElementType::Notice);
        assert_eq!(streamed[1].position, 1);
        assert!(
            streamed[1].content.contains("回合已结束且无输出"),
            "notice text comes from the shared domain constant: {}",
            streamed[1].content
        );
        assert!(
            streamed[2].content.contains("turn summary"),
            "turn-end marker still lands after the notice: {streamed:?}"
        );
        // notice 不是可见回复段：本轮唯一计入的 markdown 是收尾摘要，notice
        // 贡献 0（`count_chat_messages` 跳过 notice）。
        let info = backend
            .session_info(&NativeAgentBackend::encode_key(&key))
            .await
            .expect("session info");
        assert_eq!(
            info.msg_count, 1,
            "only the turn-summary markdown counts; the notice adds no segment"
        );
        backend.close(key).await.unwrap();
    }

    #[tokio::test]
    async fn native_pump_skips_the_notice_when_the_turn_had_output() {
        // 3.4 对照面：有正文的正常回合不补 notice（避免把正常回合染成零输出）。
        let backend = backend_with(FakeLlmClient::scripted(vec![two_chunk_turn()]));
        let mut turns = backend.subscribe_turn_events();
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");

        let streamed = collect_turn(&mut turns).await;
        assert!(
            streamed
                .iter()
                .all(|e| e.element_type != sebas_domain::vocabulary::TurnElementType::Notice),
            "a turn with text must not carry a zero-output notice: {streamed:?}"
        );
        backend.close(key).await.unwrap();
    }

    /// fold-tool-calls-into-process-tree 3.2：webui 面的工具痕迹升为一等
    /// tool 条目——`element_type = "tool"` + 结构化标题（`Read · <path>`
    /// 形态）+ 调用/结果相等的 call id；且 `turn_visible_output` 记账语义
    /// 不变（`tool` 与 `markdown` 同在可见输出表：纯工具回合不补零输出
    /// notice）。
    #[tokio::test]
    async fn native_tool_traces_land_as_first_class_tool_entries() {
        let backend = backend_with(FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![(
                "tc-read-9",
                "read",
                serde_json::json!({"path": "src/lib.rs"}),
            )]),
            FakeLlmClient::say("done after read"),
        ]));
        let mut turns = backend.subscribe_turn_events();
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");

        let streamed = collect_turn(&mut turns).await;
        let tools: Vec<_> = streamed
            .iter()
            .filter(|e| e.element_type == sebas_domain::vocabulary::TurnElementType::Tool)
            .collect();
        assert_eq!(tools.len(), 2, "call + result both land as tool entries: {streamed:?}");
        assert_eq!(
            tools[0].tool_use_id.as_deref(),
            Some("tc-read-9"),
            "invocation carries the upstream call id"
        );
        assert_eq!(
            tools[0].title.as_deref(),
            Some("read · src/lib.rs"),
            "structured title reuses the dispatch key-order rule"
        );
        assert_eq!(
            tools[1].tool_use_id.as_deref(),
            Some("tc-read-9"),
            "result carries the pairing id"
        );
        assert_eq!(tools[1].title.as_deref(), Some("✓ read"));
        // 记账语义不变：本轮可见输出 = 工具条目（非空），收尾不得补零输出
        // notice（tool 与 markdown 同在可见输出表）。
        assert!(
            streamed
                .iter()
                .all(|e| e.element_type != sebas_domain::vocabulary::TurnElementType::Notice),
            "a tool-only turn is visible output, never zero-output: {streamed:?}"
        );
        backend.close(key).await.unwrap();
    }

    // ── fix-webui-streaming-liveness 2.2：webui 面 × IM 桥面对账 ───────────

    #[tokio::test]
    async fn native_transcript_parity_between_webui_face_and_im_bridge_face() {
        // 同一脚本（多段文本）分别经 webui 面（NativeAgentBackend pump）与
        // IM 桥面（native_dispatch_bridge → router turn_log）驱动，两侧
        // transcript 条目序列必须一致——spec「webui 面与 IM 面对流式粒度
        // SHALL 一致」的验收，钉住 2.1 改动的逐 delta 落账路径。
        //
        // 脚本刻意不含 gated 工具：⏳/🛡/🗒 审批与回合摘要是 webui 面独有
        // 的过程呈现（IM 桥的 pump 本就不渲染这些 AgentEvent，属既有呈现
        // 差异、不属流式粒度）；本对账钉的是内容生产路径（delta 落账）。
        let make_manager = || {
            let llm = FakeLlmClient::scripted(vec![two_chunk_turn()]);
            SessionManager::new(
                Arc::new(llm),
                ToolRegistry::with_sandbox(Duration::from_secs(10), SandboxMode::Firewall),
                SessionConfig::default(),
            )
            .with_policy(Arc::new(
                sebas_agent::policy::PolicyEngine::new(Default::default()),
            ))
            .with_approver(sebas_agent::policy::ApproverHub::new())
        };

        // ── webui 面：spawn → 等收尾文本落账。
        let backend = NativeAgentBackend::with_manager(make_manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("webui spawn");
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let joined: String = backend
                    .turns(key.clone(), 0)
                    .await
                    .unwrap()
                    .iter()
                    .map(|e| e.content.clone())
                    .collect();
                if joined.contains("turn summary") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("webui turn must complete");
        let webui_turns = backend.turns(key.clone(), 0).await.unwrap();

        // ── IM 桥面：prompt → 等落账。
        use sebas_dispatch::native_bridge::NativeSessionBridge;
        let (router, mut out_rx) = sebas_dispatch::DispatchHandle::new(sebas_dispatch::state::SessionMap::new());
        tokio::spawn(async move { while out_rx.recv().await.is_some() {} });
        let bridge = crate::native_dispatch_bridge::DispatchNativeBridge::new(
            Arc::new(make_manager()),
            router.clone(),
        );
        let bridge_key = sebas_channels::ChannelKey::new("feishu", "agent-parity");
        bridge.clone().prompt(bridge_key.clone(), "go".into());
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if router.session_exists(&bridge_key).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("bridge session registered");
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let turns = router.session_turns(&bridge_key, 0).await.unwrap_or_default();
                let joined: String = turns.iter().map(|e| e.content.clone()).collect();
                if joined.contains("chunk two") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("bridge turn must complete");
        let bridge_turns = router.session_turns(&bridge_key, 0).await.unwrap_or_default();

        // ── 对账：两侧的流式内容条目（position/kind/element_type/content）
        // 全等；webui 面允许在此之外多出它独有的回合摘要呈现（🗒 SessionSummary，
        // IM 桥的 pump 本就不渲染该事件——既有呈现差异，不属流式粒度）。
        assert!(!webui_turns.is_empty(), "webui face must have transcript");
        // （fix-webui-qa-round8 2.1）webui 面独有操作者 prompt 条目（seed 等
        // 价，IM 桥的 pump 不渲染提交本身）——对账只比内容条目（kind=content）。
        let webui_content: Vec<&TurnEntry> = webui_turns
            .iter()
            .filter(|e| e.kind == sebas_domain::session::TurnKind::Content)
            .collect();
        // position 不进内容对账：webui 面的 prompt 条目让内容条目整体后移
        // 一位（两侧各自单调），其余（kind/element_type/content）逐条全等。
        let semantic_content = |e: &TurnEntry| {
            (
                e.kind.clone(),
                e.element_type.clone(),
                e.content.clone(),
            )
        };
        assert_eq!(
            webui_content[..bridge_turns.len()]
                .iter()
                .map(|e| semantic_content(e))
                .collect::<Vec<_>>(),
            bridge_turns
                .iter()
                .map(|e| semantic_content(e))
                .collect::<Vec<_>>(),
            "webui face and IM bridge face must render identical streamed transcripts"
        );
        assert!(
            webui_content[bridge_turns.len()..]
                .iter()
                .all(|e| e.content.contains("turn summary")),
            "webui-only extras must be presentation traces only: {:?}",
            &webui_turns[bridge_turns.len()..]
        );

        backend.close(key).await.unwrap();
    }

    // ── fix-webui-qa-round8：native 转录补全 / 影子队列 / 模型留痕 / 取消中性 ──

    struct SlowTool;

    #[async_trait::async_trait]
    impl sebas_agent::tools::Tool for SlowTool {
        fn name(&self) -> &'static str {
            "slow"
        }
        fn description(&self) -> String {
            "slow stub for queue-window testing".into()
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &sebas_agent::tools::ToolCtx,
        ) -> sebas_agent::message::ToolOutput {
            tokio::time::sleep(Duration::from_millis(400)).await;
            sebas_agent::message::ToolOutput::ok("slow-ok")
        }
    }

    /// 无策略引擎（工具不门控）+ 慢工具：turn 1 卡在工具执行窗口，为影子
    /// 队列的「提交即记 → 终态推进」提供确定性在飞窗。
    fn slow_manager() -> SessionManager {
        let llm = FakeLlmClient::scripted(vec![
            FakeLlmClient::call_tools(vec![("t1", "slow", serde_json::json!({}))]),
            FakeLlmClient::say("turn1 done"),
            FakeLlmClient::say("turn2 done"),
            FakeLlmClient::say("turn3 done"),
        ]);
        SessionManager::new(
            Arc::new(llm),
            ToolRegistry::from_tools(vec![Arc::new(SlowTool)]),
            SessionConfig::default(),
        )
    }

    async fn wait_until_content(backend: &NativeAgentBackend, key: &ChannelKey, needle: &str) {
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let turns = backend.turns(key.clone(), 0).await.unwrap();
                if turns.iter().any(|t| t.content.contains(needle)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .expect("content deadline");
    }

    /// （2.1）native 会话转录为操作者提交渲染 prompt 条目：spawn 首条与
    /// message 追加都在回复之前（`kind = "prompt"`，内容 = 提交原文）。
    #[tokio::test]
    async fn native_transcript_renders_operator_prompts() {
        let backend = NativeAgentBackend::with_manager(slow_manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn(
                "first message".into(),
                Some(ws.path().to_string_lossy().into()),
            )
            .await
            .expect("spawn");
        // spawn 同步落账：返回即可断言首条 prompt 条目。
        let turns = backend.turns(key.clone(), 0).await.unwrap();
        assert_eq!(turns[0].kind, sebas_domain::session::TurnKind::Prompt);
        assert_eq!(turns[0].content, "first message");

        // 追加消息同样先落 prompt 条目（在飞窗口内提交）。
        backend
            .message(key.clone(), "second message".into())
            .await
            .unwrap();
        let turns = backend.turns(key.clone(), 0).await.unwrap();
        assert!(
            turns.iter().any(|t| t.kind == sebas_domain::session::TurnKind::Prompt
                && t.content == "second message"),
            "the appended submission lands a prompt entry: {turns:?}"
        );
        backend.close(key).await.unwrap();
    }

    /// （2.2）影子队列三相：busy 提交即记入（pending 可见）→ 终态帧对账
    /// 推进（队头弹出）→ 会话终结清空（未知会话拒绝）。remove/move 的可用
    /// 子集一并在在飞窗内断言。
    #[tokio::test]
    async fn native_shadow_queue_records_advances_and_clears() {
        let backend = NativeAgentBackend::with_manager(slow_manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");
        // 等 turn 1 进入慢工具窗口（transcript 出现工具痕迹即已开轮）。
        wait_until_content(&backend, &key, "slow").await;

        // 相位 1（记入）：在飞提交进入影子队列，快照 pending 非空、投递序。
        backend
            .message(key.clone(), "q-first".into())
            .await
            .unwrap();
        backend
            .message(key.clone(), "q-second".into())
            .await
            .unwrap();
        let view = backend.pending(key.clone()).await.expect("pending read");
        let texts: Vec<&str> = view.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, vec!["q-first", "q-second"], "投递序");
        let info = backend
            .snapshot()
            .await
            .into_iter()
            .find(|s| s.channel_key() == key)
            .expect("native session in snapshot");
        assert_eq!(
            info.pending.len(), 2,
            "SessionInfo.pending exposes the shadow queue"
        );

        // 可用子集：重排 + 移除。q-second 上移到队头，再移除它。
        let view = backend
            .move_pending(key.clone(), view[1].id, 0)
            .await
            .expect("move");
        assert_eq!(view[0].text, "q-second");
        let view = backend
            .remove_pending(key.clone(), view[0].id)
            .await
            .expect("remove");
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].text, "q-first");
        // 未知 id → 类型化 Unknown。
        let err = backend
            .remove_pending(key.clone(), u64::MAX)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            SessionRejection::PendingRejected {
                reason: sebas_domain::session::PendingReason::Unknown
            }
        ));

        // 相位 2（推进）：turn 1 终态 + summary 落账后，pump 出队队头并投递
        // 内核（宿主驱动）——q-first 开轮执行，队列随之清空；被移除的
        // q-second 不再执行（其应答文本永不出现）。
        wait_until_content(&backend, &key, "turn1 done").await;
        wait_until_content(&backend, &key, "turn2 done").await;
        let view = backend.pending(key.clone()).await.expect("pending read");
        assert!(view.is_empty(), "terminal frames drain the shadow queue");
        // 被移除条目的执行否决：给足轮转时间后其应答仍不在转录里。
        tokio::time::sleep(Duration::from_millis(600)).await;
        let joined: String = backend
            .turns(key.clone(), 0)
            .await
            .unwrap()
            .iter()
            .map(|t| t.content.clone())
            .collect();
        assert!(
            !joined.contains("turn3 done"),
            "the removed submission must never execute: {joined}"
        );

        // 相位 3（清空）：close 后未知会话拒绝。
        backend.close(key.clone()).await.unwrap();
        let err = backend.pending(key).await.unwrap_err();
        assert!(matches!(err, SessionRejection::UnknownSession { .. }));
    }

    /// （2.2 review 补修）忙时入队两条 → 按序执行：remove 撤不掉的队头在
    /// pump 终态推进下先开轮，次条随后——宿主持有队列的顺序语义与 ACP 面
    /// 一致，且每条排队提交的应答都真实到达。
    #[tokio::test]
    async fn native_shadow_queue_executes_in_order_after_the_turn() {
        let backend = NativeAgentBackend::with_manager(slow_manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");
        wait_until_content(&backend, &key, "slow").await;
        backend
            .message(key.clone(), "s-first".into())
            .await
            .unwrap();
        backend
            .message(key.clone(), "s-second".into())
            .await
            .unwrap();

        // 两条排队提交按队序各开一轮：turn1（慢工具）→ s-first → s-second。
        wait_until_content(&backend, &key, "turn1 done").await;
        wait_until_content(&backend, &key, "turn2 done").await;
        wait_until_content(&backend, &key, "turn3 done").await;

        // 执行顺序钉死：turn1 的应答先于 s-first 的应答，s-first 先于
        // s-second（队列推进逐条出队，绝不插队/并发开轮）。
        let turns = backend.turns(key.clone(), 0).await.unwrap();
        let position_of = |needle: &str| {
            turns
                .iter()
                .find(|t| t.content.contains(needle))
                .map(|t| t.position)
                .unwrap_or_else(|| panic!("{needle} missing"))
        };
        let (p1, p2, p3) = (
            position_of("turn1 done"),
            position_of("turn2 done"),
            position_of("turn3 done"),
        );
        assert!(p1 < p2 && p2 < p3, "queue order must be respected: {p1} {p2} {p3}");
        let view = backend.pending(key.clone()).await.expect("pending read");
        assert!(view.is_empty(), "the queue drains after both turns");
        backend.close(key).await.unwrap();
    }

    /// （5.2）native override 路径的模型切换留痕：条目含新旧模型名，from 取
    /// 当前生效模型（override 或内核默认）。
    #[tokio::test]
    async fn native_model_override_lands_model_change_entry() {
        let backend = NativeAgentBackend::with_manager(slow_manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");
        wait_until_content(&backend, &key, "slow").await;

        backend
            .set_session_model(key.clone(), "test/long".into())
            .await
            .expect("set model");
        let turns = backend.turns(key.clone(), 0).await.unwrap();
        let entry = turns
            .iter()
            .find(|e| e.element_type.as_str() == "model_change")
            .expect("model_change entry");
        let payload: serde_json::Value =
            serde_json::from_str(&entry.content).expect("payload is JSON");
        assert_eq!(payload["from"], "claude-sonnet-4-5", "from = 内核默认模型");
        assert_eq!(payload["to"], "test/long");

        // 二次切换：from = 上一次的 override 值。
        backend
            .set_session_model(key.clone(), "test/text".into())
            .await
            .expect("set model again");
        let turns = backend.turns(key.clone(), 0).await.unwrap();
        let entries: Vec<&TurnEntry> = turns
            .iter()
            .filter(|e| e.element_type.as_str() == "model_change")
            .collect();
        assert_eq!(entries.len(), 2);
        let payload: serde_json::Value =
            serde_json::from_str(&entries[1].content).expect("payload is JSON");
        assert_eq!(payload["from"], "test/long");
        assert_eq!(payload["to"], "test/text");
        backend.close(key).await.unwrap();
    }

    /// （7.3）操作者取消 = 中性「已取消」条目（notice），不再是 ⚠ 错误形态。
    #[tokio::test]
    async fn native_cancel_lands_a_neutral_notice() {
        let backend = NativeAgentBackend::with_manager(slow_manager());
        let ws = tempfile::tempdir().unwrap();
        let key = backend
            .spawn("go".into(), Some(ws.path().to_string_lossy().into()))
            .await
            .expect("spawn");
        wait_until_content(&backend, &key, "slow").await;
        backend.cancel(key.clone()).await.expect("cancel in flight");
        wait_until_content(&backend, &key, "回合已取消").await;
        let turns = backend.turns(key.clone(), 0).await.unwrap();
        let entry = turns
            .iter()
            .find(|e| e.content.contains("回合已取消"))
            .expect("cancel notice");
        assert_eq!(
            entry.element_type.as_str(),
            "notice",
            "中性 notice，不是错误"
        );
        backend.close(key).await.unwrap();
    }
}
