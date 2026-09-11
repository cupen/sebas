//! 真实执行体：把一个 agent 子进程跑在**节点**上（add-remote-execution-node 6.3 /
//! 7.1 / 7.2）。
//!
//! 三件事在这里落地：
//!
//! 1. **ACP 子进程**：用 `sebas-acp` 的 `SessionManager` 起真正的 agent（Claude Code
//!    CLI 或通用 ACP agent），把它的事件词汇翻译成节点日志条目。执行体是同步 trait，
//!    而 ACP 是异步的：这里用**一条专属线程 + 自己的 current-thread 运行时**做桥，
//!    宿主的同步调用通过通道进入、通过 condvar 取回，宿主对这些毫不知情。
//! 2. **provider 归属**（7.1）：凭据留在节点本地（profile 里只写「去哪个 env 取」），
//!    控制面按名字选；期望与实际不同时如实回报差异与成因，绝不静默换一个。
//! 3. **经主控 router 出网**（7.2）：`upstream = control-plane-router` 时节点零凭据，
//!    用**控制面在握手时告知的** router 地址注入 `ANTHROPIC_BASE_URL`。控制面没说
//!    地址就如实拒绝——节点不知道主控在哪，猜一个地址等于把流量发进黑洞。
//!
//! ## 门控（mode）的诚实边界
//!
//! ACP 路径上权限请求由 agent 主动发出。`claude` 专用驱动的 `PreToolUse` 钩子覆盖
//! 每一次工具调用，所以它**能**强制 mode（配置里显式声明才算）；通用 ACP 驱动没有
//! 全量拦截点，因此 `enforces_mode` 只能是 `false`——宿主仍按期望模式尽力门控它看得
//! 见的请求，但如实回报「没有可声称生效的 mode」，而不是把期望值回显成已生效。

use crate::config::{AgentDriverKind, NodeBodyConfig, ProviderProtocol, Upstream};
use crate::session::{
    BodyEvent, BodyFactory, BodySpec, EchoOnlyFactory, ExecutionBody, GateRequest, MadeBody,
    Rejected,
};
use sebas_acp::claude::{AgentEntry, SessionManager};
use sebas_acp::{AcpCommand, AcpDriver, AcpEvent, AgentDriver, ClaudeDriver};
use sebas_node_link::{ApprovalDecision, GateCategory, SessionMode, SessionRejectCode};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// agent 握手的启动超时（ACP initialize/load 在里面完成）。
pub const ACP_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// 把一条输入交给驱动器的确认超时。
pub const ACP_ACK_TIMEOUT: Duration = Duration::from_secs(15);
/// 等待模型切换**结果**（`ModelChanged` 或错误）的超时。
pub const ACP_MODEL_TIMEOUT: Duration = Duration::from_secs(10);
/// 投递输入后给第一波输出留的短窗口（更长的输出由 `drain` 继续交）。
pub const ACP_FIRST_EVENT_GRACE: Duration = Duration::from_millis(100);

/// 控制面在握手里告知的 router 端点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterEndpoint {
    /// 地址（含 scheme）。
    pub url: String,
    /// 访问凭据（控制面未要求鉴权 → `None`）。
    pub token: Option<String>,
}

/// 节点与链路共享的 router 端点槽：链路在握手成功后填、断链时清空。
pub type RouterCell = Arc<Mutex<Option<RouterEndpoint>>>;

// ── provider 解析（7.1 / 7.2）────────────────────────────────────────────────

/// 一个会话的 provider 期望 → 实际（env 注入 + desired/effective 回报）。
#[derive(Debug)]
struct ProviderResolution {
    /// 注入子进程的 env。
    env: Vec<(String, String)>,
    /// 实际生效的 provider。
    provider: Option<String>,
    /// 控制面期望的 provider（没给 → `None`）。
    desired: Option<String>,
    /// 期望与实际不同时的成因（相同 → `None`）。
    cause: Option<String>,
}

/// 节点本地执行体工厂（真实 ACP 执行体 + 自带的 echo）。
pub struct NodeBodyFactory {
    cfg: NodeBodyConfig,
    router: RouterCell,
}

impl NodeBodyFactory {
    /// 以节点配置构造。
    pub fn new(cfg: NodeBodyConfig) -> Self {
        Self {
            cfg,
            router: Arc::new(Mutex::new(None)),
        }
    }

    /// 与链路共享的 router 端点槽。
    pub fn router_cell(&self) -> RouterCell {
        Arc::clone(&self.router)
    }

    /// 控制面告知（或撤回）router 端点。
    pub fn note_router_endpoint(&self, endpoint: Option<RouterEndpoint>) {
        *self.router.lock().unwrap_or_else(|e| e.into_inner()) = endpoint;
    }

    /// 解析一个会话的 provider 归属与 spawn env。
    ///
    /// 三条诚实规则：
    /// - 期望的 profile 不存在 / 凭据 env 缺失 → **typed 拒绝**（不是换一个继续跑）；
    /// - 控制面没指定而节点配了 `default_provider` → 应用它，但 desired 仍是 `None`
    ///   并把成因写清楚（「节点默认」不是「控制面选的就是它」）；
    /// - `upstream = control-plane-router` 但控制面没告知地址 → **拒绝**，不猜地址。
    fn resolve_provider(&self, requested: Option<&str>) -> Result<ProviderResolution, Rejected> {
        match self.cfg.upstream {
            Upstream::ControlPlaneRouter => {
                let endpoint = self
                    .router
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let Some(endpoint) = endpoint else {
                    return Err(Rejected::new(
                        SessionRejectCode::ProviderUnavailable,
                        "节点配置 upstream = control-plane-router，但控制面没有在握手时告知 \
                         router 地址（HelloAck.router_url 缺失）；节点不猜测地址，也不退回本地凭据",
                    ));
                };
                if !(endpoint.url.starts_with("http://") || endpoint.url.starts_with("https://")) {
                    return Err(Rejected::new(
                        SessionRejectCode::ProviderUnavailable,
                        format!(
                            "控制面告知的 router 地址 {:?} 不是 http(s) 端点；节点不猜测可用地址",
                            endpoint.url
                        ),
                    ));
                }
                // 主控 router 对外一律是 Anthropic 形状的 API（无论后面接什么上游）。
                let mut env = vec![("ANTHROPIC_BASE_URL".to_string(), endpoint.url.clone())];
                if let Some(token) = endpoint.token.as_deref().filter(|t| !t.is_empty()) {
                    env.push(("ANTHROPIC_AUTH_TOKEN".to_string(), token.to_string()));
                }
                let desired = requested.map(str::to_string);
                let cause = desired.as_deref().and_then(|d| {
                    (d != "control-plane-router").then(|| {
                        format!(
                            "节点配置 upstream = control-plane-router（零凭据、模型流量经主控 \
                             router 出网）：期望的本地 profile {d:?} 未应用"
                        )
                    })
                });
                Ok(ProviderResolution {
                    env,
                    provider: Some("control-plane-router".to_string()),
                    desired,
                    cause,
                })
            }
            Upstream::Local => {
                let desired = requested.map(str::to_string);
                // 控制面没选时应用节点默认；desired 仍如实记为「没选」。
                let applied = desired
                    .clone()
                    .or_else(|| self.cfg.default_provider.clone());
                let Some(name) = applied else {
                    // 没有选中的 profile：不注入任何 provider env（agent 用自己的环境）。
                    return Ok(ProviderResolution {
                        env: Vec::new(),
                        provider: None,
                        desired: None,
                        cause: None,
                    });
                };
                let profile = self.cfg.profile(&name).ok_or_else(|| {
                    let configured = if self.cfg.provider_profiles.is_empty() {
                        "（未配置任何 provider profile）".to_string()
                    } else {
                        self.cfg
                            .provider_profiles
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    Rejected::new(
                        SessionRejectCode::ProviderUnavailable,
                        format!("本节点没有 provider profile {name:?}：{configured}"),
                    )
                })?;

                let mut env: Vec<(String, String)> = Vec::new();
                let (url_var, key_var) = match profile.protocol {
                    ProviderProtocol::Anthropic => ("ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"),
                    ProviderProtocol::OpenAi => ("OPENAI_BASE_URL", "OPENAI_API_KEY"),
                };
                env.push((url_var.to_string(), profile.base_url.clone()));
                if let Some(key_env) = &profile.api_key_env {
                    // 凭据留在节点上：profile 只说去哪取，取不到就如实拒绝——带着空密钥
                    // 起 agent 只会把失败推迟到第一次模型调用，而且看不出为什么。
                    let value = std::env::var(key_env).map_err(|e| {
                        Rejected::new(
                            SessionRejectCode::ProviderUnavailable,
                            format!(
                                "provider profile {:?} 的凭据环境变量 {key_env} 在节点上不可用（{e}）；\
                                 凭据留在节点本地，控制面不下发",
                                profile.name
                            ),
                        )
                    })?;
                    if value.trim().is_empty() {
                        return Err(Rejected::new(
                            SessionRejectCode::ProviderUnavailable,
                            format!(
                                "provider profile {:?} 的凭据环境变量 {key_env} 是空值；\
                                 节点不会带着空密钥启动 agent",
                                profile.name
                            ),
                        ));
                    }
                    env.push((key_var.to_string(), value));
                }
                let cause = desired.is_none().then(|| {
                    format!("控制面未指定 provider；节点按配置的 default_provider 应用了 {name:?}")
                });
                Ok(ProviderResolution {
                    env,
                    provider: Some(name),
                    desired,
                    cause,
                })
            }
        }
    }
}

impl BodyFactory for NodeBodyFactory {
    fn make(&self, spec: &BodySpec<'_>) -> Result<MadeBody, Rejected> {
        // echo 与宿主配合门控（宿主就是它的拦截点），与节点配置无关。
        if spec.kind == "echo" || spec.kind.is_empty() {
            return EchoOnlyFactory.make(spec);
        }
        let section = self.cfg.agent(spec.kind).ok_or_else(|| {
            Rejected::new(
                SessionRejectCode::UnsupportedAgentKind,
                format!(
                    "本节点未配置 agent kind {:?}（已配置：{}）",
                    spec.kind,
                    if self.cfg.agents.is_empty() {
                        "（无）".to_string()
                    } else {
                        self.cfg.agents.keys().cloned().collect::<Vec<_>>().join(", ")
                    }
                ),
            )
        })?;
        let driver = section.driver.ok_or_else(|| {
            Rejected::new(
                SessionRejectCode::UnsupportedAgentKind,
                format!(
                    "agent kind {:?} 未配置 driver（运行时不在这里）；配置 driver = \"claude\" 或 \"acp\" 才算接入",
                    spec.kind
                ),
            )
        })?;

        // 工作目录：项目目录优先；无项目会话必须有 default_work_dir——**不退回节点
        // 进程当前目录**（那会让 agent 在一个谁也没选过的目录里干活）。
        let work_dir = match (spec.project_dir, self.cfg.default_work_dir.as_deref()) {
            (Some(project), _) => project.to_path_buf(),
            (None, Some(default)) => default.to_path_buf(),
            (None, None) => {
                return Err(Rejected::new(
                    SessionRejectCode::UnusableProjectDir,
                    "无项目会话需要一个工作目录：节点未配置 default_work_dir，会话也未指定 \
                     project_dir；不会退回节点进程的当前目录",
                ));
            }
        };
        if !work_dir.is_dir() {
            return Err(Rejected::new(
                SessionRejectCode::UnusableProjectDir,
                format!(
                    "工作目录 {} 在本节点上不可用（不是目录或不存在）",
                    work_dir.display()
                ),
            ));
        }

        let resolution = self.resolve_provider(spec.provider)?;

        let executable = section
            .command
            .clone()
            .unwrap_or_else(|| spec.kind.to_string());
        let mut command = Vec::with_capacity(section.args.len() + 1);
        command.push(executable);
        command.extend(section.args.iter().cloned());
        // Claude 专用驱动在 argv 上选模型（`--model`）；通用 ACP 驱动走 ACP 的
        // `session/set_config_option`（见 `set_model`），不在 argv 上塞。
        if driver == AgentDriverKind::Claude
            && let Some(model) = spec.model
        {
            command.push("--model".to_string());
            command.push(model.to_string());
        }

        let desired_mode = spec.mode.and_then(SessionMode::parse);
        let body = AcpBody::start(AcpBodyConfig {
            kind: spec.kind.to_string(),
            driver,
            command,
            work_dir,
            extra_env: resolution.env.clone(),
            model: spec.model.map(str::to_string),
            desired_mode,
            // 「能强制 mode」由节点配置说，且只有全量拦截点的驱动才允许声明 true
            // （`config::validate` 已拦住其余情形）。
            enforces_mode: section.enforces_mode.unwrap_or(false),
            ..AcpBodyConfig::default()
        })?;

        Ok(MadeBody {
            body: Box::new(body),
            provider: resolution.provider,
            desired_provider: resolution.desired,
            provider_cause: resolution.cause,
        })
    }

    fn available_kinds(&self) -> Vec<String> {
        let mut kinds = vec!["echo".to_string()];
        for (kind, section) in &self.cfg.agents {
            if section.driver.is_some() {
                kinds.push(kind.clone());
            }
        }
        kinds.sort();
        kinds.dedup();
        kinds
    }
}

// ── ACP 执行体 ──────────────────────────────────────────────────────────────

/// 构造参数（可注入超时，测试不必等默认时长）。
#[derive(Debug, Clone)]
pub struct AcpBodyConfig {
    /// kind slug（会话路由用）。
    pub kind: String,
    /// 驱动这个 kind 的运行时。
    pub driver: AgentDriverKind,
    /// 完整 argv。
    pub command: Vec<String>,
    /// 子进程工作目录。
    pub work_dir: PathBuf,
    /// 注入子进程的 env（provider 派生）。
    pub extra_env: Vec<(String, String)>,
    /// 期望模型。
    pub model: Option<String>,
    /// 期望 mode。
    pub desired_mode: Option<SessionMode>,
    /// 是否能强制 mode（节点配置声明）。
    pub enforces_mode: bool,
    /// ACP 握手超时。
    pub startup_timeout: Duration,
    /// 投递输入的确认超时。
    pub ack_timeout: Duration,
    /// 模型切换结果等待超时。
    pub model_timeout: Duration,
    /// 第一波输出等待窗口。
    pub first_event_grace: Duration,
}

impl Default for AcpBodyConfig {
    fn default() -> Self {
        Self {
            kind: String::new(),
            driver: AgentDriverKind::Claude,
            command: Vec::new(),
            work_dir: PathBuf::new(),
            extra_env: Vec::new(),
            model: None,
            desired_mode: None,
            enforces_mode: false,
            startup_timeout: ACP_STARTUP_TIMEOUT,
            ack_timeout: ACP_ACK_TIMEOUT,
            model_timeout: ACP_MODEL_TIMEOUT,
            first_event_grace: ACP_FIRST_EVENT_GRACE,
        }
    }
}

/// 同步门面的内部共享：worker 往里放事件，宿主法通过这些方法取。
struct Shared {
    state: Mutex<SharedState>,
    cv: Condvar,
}

struct SharedState {
    events: VecDeque<BodyEvent>,
    /// 是否有在飞的 turn（`cancel` 如实回报的依据）。
    turn_active: bool,
    /// 执行体已退出（成因）。
    dead: Option<String>,
}

impl Shared {
    fn new() -> Self {
        Self {
            state: Mutex::new(SharedState {
                events: VecDeque::new(),
                turn_active: false,
                dead: None,
            }),
            cv: Condvar::new(),
        }
    }

    fn push(&self, event: BodyEvent) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.events.push_back(event);
        drop(st);
        self.cv.notify_all();
    }

    fn set_turn_active(&self, active: bool) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.turn_active = active;
    }

    fn turn_active(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .turn_active
    }

    fn mark_dead(&self, cause: impl Into<String>) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if st.dead.is_none() {
            st.dead = Some(cause.into());
        }
        st.turn_active = false;
        drop(st);
        self.cv.notify_all();
    }

    /// 取出队列里的条目，**取到第一个门为止（含）**。
    ///
    /// 门之后的条目必须留在队列里：它们还没获准发生，交出去就会被落账成「已发生」。
    fn take_until_gate(&self) -> Vec<BodyEvent> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        while let Some(event) = st.events.pop_front() {
            let is_gate = event.gate.is_some();
            out.push(event);
            if is_gate {
                break;
            }
        }
        out
    }

    /// 等最多 `grace`，直到有事件可交（或执行体退出）。
    fn wait_for_events(&self, grace: Duration) {
        if grace.is_zero() {
            return;
        }
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !st.events.is_empty() || st.dead.is_some() {
            return;
        }
        let (st, _) = self
            .cv
            .wait_timeout_while(st, grace, |s| s.events.is_empty() && s.dead.is_none())
            .unwrap_or_else(|e| e.into_inner());
        drop(st);
    }
}

/// worker 退出守卫：无论正常返回还是 panic，都唤醒等着的宿主调用。
struct WorkerGuard {
    shared: Arc<Shared>,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.shared.mark_dead("执行体已退出");
    }
}

/// 宿主 → worker 的作业。
enum Job {
    Prompt {
        text: String,
        ack: std::sync::mpsc::Sender<Result<(), String>>,
    },
    Resolve {
        token: String,
        decision: ApprovalDecision,
        ack: std::sync::mpsc::Sender<Result<(), String>>,
    },
    SetModel {
        model: String,
        ack: std::sync::mpsc::Sender<Result<String, String>>,
    },
    Cancel {
        ack: std::sync::mpsc::Sender<Result<bool, String>>,
    },
    Close,
}

/// ACP 子进程执行体（宿主看到的同步门面）。
pub struct AcpBody {
    kind: String,
    enforces_mode: bool,
    desired_mode: Option<SessionMode>,
    model: Option<String>,
    jobs: tokio::sync::mpsc::UnboundedSender<Job>,
    shared: Arc<Shared>,
    worker: Option<std::thread::JoinHandle<()>>,
    closed: bool,
    ack_timeout: Duration,
    model_timeout: Duration,
    first_event_grace: Duration,
}

impl AcpBody {
    /// 起一个 agent 子进程并完成 ACP 握手（在专属线程的运行时里）。
    ///
    /// 握手失败 → typed 拒绝，成因指名**命令与错误**（「找不到可执行文件」与
    /// 「agent 不按 ACP 说话」是两类要修的问题，不该糊成一句「不可用」）。
    pub fn start(cfg: AcpBodyConfig) -> Result<Self, Rejected> {
        let (job_tx, job_rx) = tokio::sync::mpsc::unbounded_channel();
        let shared = Arc::new(Shared::new());
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        let kind = cfg.kind.clone();
        // 这些是门面要如实回报的值，必须在把 cfg 移进 worker 线程**之前**取出。
        let enforced = cfg.enforces_mode;
        let desired_mode = cfg.desired_mode;
        let effective_model = cfg.model.clone();
        let startup_timeout = cfg.startup_timeout;
        let worker_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name(format!("sebas-node-acp-{kind}"))
            .spawn(move || {
                let _guard = WorkerGuard {
                    shared: Arc::clone(&worker_shared),
                };
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("无法建立执行体运行时：{e}")));
                        return;
                    }
                };
                runtime.block_on(worker_main(cfg, job_rx, worker_shared, ready_tx));
            })
            .map_err(|e| {
                Rejected::new(
                    SessionRejectCode::NodeError,
                    format!("无法启动执行体线程：{e}"),
                )
            })?;

        let mut body = Self {
            kind,
            enforces_mode: enforced,
            desired_mode,
            model: effective_model,
            jobs: job_tx,
            shared,
            worker: Some(handle),
            closed: false,
            ack_timeout: ACP_ACK_TIMEOUT,
            model_timeout: ACP_MODEL_TIMEOUT,
            first_event_grace: ACP_FIRST_EVENT_GRACE,
        };

        // 握手等在启动超时 + 余量之内：线程活着但没回话也要如实给出成因。
        match ready_rx.recv_timeout(startup_timeout + Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(body),
            Ok(Err(cause)) => {
                body.close();
                Err(Rejected::new(
                    SessionRejectCode::UnsupportedAgentKind,
                    cause,
                ))
            }
            Err(_) => {
                body.close();
                Err(Rejected::new(
                    SessionRejectCode::UnsupportedAgentKind,
                    format!(
                        "启动 agent kind {:?} 超时（{:?} 内没有完成 ACP 握手）",
                        body.kind, startup_timeout
                    ),
                ))
            }
        }
    }

    fn send_job(&self, job: Job) -> Result<(), String> {
        self.jobs
            .send(job)
            .map_err(|_| format!("执行体 {} 已退出，无法投递", self.kind))
    }

    fn take_available(&mut self, out: &mut Vec<BodyEvent>, grace: Duration) {
        self.shared.wait_for_events(grace);
        out.extend(self.shared.take_until_gate());
    }

    /// 覆盖超时/窗口（测试用，避免等真实时长）。
    pub fn with_timings(
        mut self,
        ack_timeout: Duration,
        model_timeout: Duration,
        first_event_grace: Duration,
    ) -> Self {
        self.ack_timeout = ack_timeout;
        self.model_timeout = model_timeout;
        self.first_event_grace = first_event_grace;
        self
    }
}

impl ExecutionBody for AcpBody {
    fn agent_kind(&self) -> String {
        self.kind.clone()
    }

    fn model(&self) -> Option<String> {
        self.model.clone()
    }

    fn mode(&self) -> Option<String> {
        if self.enforces_mode {
            self.desired_mode.map(|m| m.as_str().to_string())
        } else {
            // 强制不了就**不声称**；期望值仍在 SessionSummary.desired_mode 里。
            None
        }
    }

    fn enforces_mode(&self) -> bool {
        self.enforces_mode
    }

    fn prompt(&mut self, text: &str, out: &mut Vec<BodyEvent>) -> Result<(), String> {
        if self.closed {
            return Err("会话已关闭".into());
        }
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        self.send_job(Job::Prompt {
            text: text.to_string(),
            ack: ack_tx,
        })?;
        match ack_rx.recv_timeout(self.ack_timeout) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(format!(
                    "投递输入后 {:?} 内执行体没有回应",
                    self.ack_timeout
                ));
            }
        }
        // 一轮 turn 的输出不会恰好落在一次调用里：真实 agent 可能跑很久、也可能
        // 先停在权限门上。这里只等一个短窗口，其余交给 drain（日志保真度不受影响）。
        self.take_available(out, self.first_event_grace);
        Ok(())
    }

    fn resolve_gate(
        &mut self,
        resume_token: &str,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        self.send_job(Job::Resolve {
            token: resume_token.to_string(),
            decision,
            ack: ack_tx,
        })?;
        match ack_rx.recv_timeout(self.ack_timeout) {
            Ok(result) => result,
            Err(_) => Err(format!(
                "把门控结论送给执行体后 {:?} 内没有回应",
                self.ack_timeout
            )),
        }
    }

    fn drain(&mut self, out: &mut Vec<BodyEvent>) -> Result<(), String> {
        self.take_available(out, Duration::ZERO);
        Ok(())
    }

    fn cancel(&mut self) -> Result<bool, String> {
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        self.send_job(Job::Cancel { ack: ack_tx })?;
        match ack_rx.recv_timeout(self.ack_timeout) {
            Ok(result) => result,
            Err(_) => Err(format!("取消请求 {:?} 内没有回应", self.ack_timeout)),
        }
    }

    fn set_model(&mut self, model: &str) -> Result<Option<String>, String> {
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        self.send_job(Job::SetModel {
            model: model.to_string(),
            ack: ack_tx,
        })?;
        match ack_rx.recv_timeout(self.model_timeout) {
            // 只有驱动**确认**生效（`ModelChanged`）才回报成功；Claude 专用驱动
            // 明确回「不支持，模型未变」，那就如实回错、不谎报切换成功。
            Ok(Ok(effective)) => {
                self.model = Some(effective.clone());
                Ok(Some(effective))
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!(
                "等待模型切换结果超过 {:?}（未确认生效，不谎报成功）",
                self.model_timeout
            )),
        }
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let _ = self.jobs.send(Job::Close);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AcpBody {
    fn drop(&mut self) {
        self.close();
    }
}

/// worker 主循环：建会话 → 泵事件 → 处理宿主作业。
async fn worker_main(
    cfg: AcpBodyConfig,
    mut jobs: tokio::sync::mpsc::UnboundedReceiver<Job>,
    shared: Arc<Shared>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let driver: Arc<dyn AgentDriver> = match cfg.driver {
        AgentDriverKind::Claude => Arc::new(ClaudeDriver),
        AgentDriverKind::Acp => Arc::new(AcpDriver),
    };
    let mut agents: HashMap<String, AgentEntry> = HashMap::new();
    agents.insert(
        cfg.kind.clone(),
        AgentEntry {
            driver,
            startup_timeout: cfg.startup_timeout,
        },
    );
    let manager = SessionManager::new(cfg.kind.clone(), agents);
    let work_dir = cfg.work_dir.to_string_lossy().to_string();

    let session = match manager
        .create_session(
            &cfg.kind,
            cfg.command.clone(),
            Some(work_dir),
            cfg.extra_env.clone(),
            String::new(),
        )
        .await
    {
        Ok(session) => session,
        Err(e) => {
            let _ = ready.send(Err(format!(
                "启动 agent {:?} 失败：{e}",
                cfg.command.join(" ")
            )));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let mut prompts_sent: u64 = 0;
    let mut pending_model: Option<std::sync::mpsc::Sender<Result<String, String>>> = None;

    loop {
        tokio::select! {
            job = jobs.recv() => {
                let Some(job) = job else { break };
                match job {
                    Job::Prompt { text, ack } => {
                        let command = if prompts_sent == 0 {
                            AcpCommand::CreateSession { session_id: session.clone(), prompt: text }
                        } else {
                            AcpCommand::ContinueSession { session_id: session.clone(), prompt: text }
                        };
                        let sent = manager.send(&session, command).await.map_err(|e| e.to_string());
                        if sent.is_ok() {
                            prompts_sent += 1;
                            shared.set_turn_active(true);
                        }
                        let _ = ack.send(sent);
                    }
                    Job::Resolve { token, decision, ack } => {
                        let result = manager
                            .send(&session, AcpCommand::PermissionReply {
                                session_id: session.clone(),
                                request_id: token,
                                decision: to_acp_decision(decision),
                            })
                            .await
                            .map_err(|e| e.to_string());
                        let _ = ack.send(result);
                    }
                    Job::SetModel { model, ack } => {
                        // 结果由事件确认（ModelChanged / Error），不在 send 返回时下结论。
                        match manager.set_model(&session, model).await {
                            Ok(()) => pending_model = Some(ack),
                            Err(e) => { let _ = ack.send(Err(format!("切换模型失败：{e}"))); }
                        }
                    }
                    Job::Cancel { ack } => {
                        let active = shared.turn_active();
                        if active {
                            let _ = manager
                                .send(&session, AcpCommand::Cancel { session_id: session.clone() })
                                .await;
                        }
                        let _ = ack.send(Ok(active));
                    }
                    Job::Close => {
                        manager.kill(&session).await;
                        return;
                    }
                }
            }
            event = manager.next_event(&session) => {
                match event {
                    Some(event) => {
                        let terminal = matches!(&event, AcpEvent::Error { terminal: true, .. });
                        match &event {
                            AcpEvent::ModelChanged { model_id, .. } => {
                                if let Some(ack) = pending_model.take() {
                                    let _ = ack.send(Ok(model_id.clone()));
                                }
                            }
                            AcpEvent::Error { message, .. } => {
                                if let Some(ack) = pending_model.take() {
                                    let _ = ack.send(Err(message.clone()));
                                }
                                if terminal {
                                    shared.push(terminal_body_event(message));
                                    shared.mark_dead(message.clone());
                                    manager.kill(&session).await;
                                    return;
                                }
                            }
                            AcpEvent::Finished { .. } => shared.set_turn_active(false),
                            _ => {}
                        }
                        for body_event in translate_event(&event) {
                            shared.push(body_event);
                        }
                    }
                    None => {
                        // 事件流关闭 = 子进程退出：如实写一条终态错误（会话相位转 exited）。
                        let cause = "agent 事件流已关闭（子进程退出）".to_string();
                        shared.push(terminal_body_event(&cause));
                        shared.mark_dead(cause);
                        manager.kill(&session).await;
                        return;
                    }
                }
            }
        }
    }

    manager.kill(&session).await;
}

/// 节点链路的三档决定 → ACP 驱动的三档决定。
///
/// 两侧各有一份同名枚举（链路契约不依赖 `sebas-acp`），这里是唯一的翻译点：
/// 三档一一对应，没有第四种可能（`escalate` 是原生内核专属，不过链路）。
fn to_acp_decision(decision: ApprovalDecision) -> sebas_acp::Decision {
    match decision {
        ApprovalDecision::AllowOnce => sebas_acp::Decision::AllowOnce,
        ApprovalDecision::AllowSession => sebas_acp::Decision::AllowSession,
        ApprovalDecision::Deny => sebas_acp::Decision::Deny,
    }
}

/// 一条终态错误条目（宿主据此把会话标为 `exited` 并上报 `Exited`）。
fn terminal_body_event(cause: &str) -> BodyEvent {    BodyEvent {
        kind: "error".into(),
        text: cause.to_string(),
        data: Some(serde_json::json!({ "terminal": true })),
        gate: None,
    }
}

/// 把 ACP 事件翻译成节点日志条目（**不过河驱动内部词表**：这里就翻成粗粒度条目）。
fn translate_event(event: &AcpEvent) -> Vec<BodyEvent> {
    let mut out = Vec::new();
    let plain = |kind: &str, text: String| BodyEvent {
        kind: kind.to_string(),
        text,
        data: None,
        gate: None,
    };
    match event {
        AcpEvent::TextDelta { delta, .. } => out.push(plain("output", delta.clone())),
        AcpEvent::ThinkingDelta { delta, .. } => out.push(plain("thinking", delta.clone())),
        AcpEvent::ToolStart {
            tool_name, args, ..
        } => out.push(BodyEvent {
            kind: "tool_start".into(),
            text: tool_name.clone(),
            data: Some(serde_json::json!({ "tool": tool_name, "args": args })),
            gate: None,
        }),
        AcpEvent::ToolProgress {
            tool_name,
            progress,
            ..
        } => out.push(BodyEvent {
            kind: "tool_progress".into(),
            text: progress.clone(),
            data: Some(serde_json::json!({ "tool": tool_name })),
            gate: None,
        }),
        AcpEvent::ToolEnd {
            tool_name, result, ..
        } => out.push(BodyEvent {
            kind: "tool_end".into(),
            text: result.clone(),
            data: Some(serde_json::json!({ "tool": tool_name })),
            gate: None,
        }),
        AcpEvent::PermissionRequest {
            request_id,
            tool_name,
            args,
            ..
        } => {
            let category = classify_tool(tool_name);
            out.push(BodyEvent {
                kind: "permission_request".into(),
                text: tool_name.clone(),
                data: Some(serde_json::json!({
                    "tool": tool_name,
                    "args": args,
                    "request_id": request_id,
                })),
                gate: Some(GateRequest {
                    tool: tool_name.clone(),
                    category,
                    // 回执句柄：宿主拿到控制面决定后靠它把 agent 从门上放走。
                    resume_token: Some(request_id.clone()),
                }),
            });
        }
        AcpEvent::Finished { .. } => out.push(plain("turn_finished", String::new())),
        AcpEvent::Error { message, .. } => out.push(BodyEvent {
            kind: "error".into(),
            text: message.clone(),
            data: Some(serde_json::json!({})),
            gate: None,
        }),
        AcpEvent::UsageUpdate { usage, .. } => out.push(BodyEvent {
            kind: "usage".into(),
            text: String::new(),
            data: serde_json::to_value(usage).ok(),
            gate: None,
        }),
        AcpEvent::ModelChanged { model_id, .. } => out.push(plain("model_changed", model_id.clone())),
    }
    out
}

/// 工具名 → 门控类别。
///
/// 只做**粗分类**（协议上的类别只有三种），按名字里的特征词判定：命令执行类走
/// `Execute`、写文件类走 `Edit`、其余 `Other`。判不出来时归 `Other`（更严：`ask`
/// 会问、`edit` 不会自动放行）——宁可比实际更保守，也不能把执行类误当编辑类放行。
fn classify_tool(name: &str) -> GateCategory {
    let lowered = name.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lowered.contains(n));
    if has(&["edit", "write", "patch", "notebook", "apply_patch", "str_replace"]) {
        return GateCategory::Edit;
    }
    if has(&[
        "bash", "shell", "exec", "command", "terminal", "run", "process",
    ]) {
        return GateCategory::Execute;
    }
    GateCategory::Other
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AgentSection, ProviderProfile, ProviderProfileSection, Upstream,
    };

    fn body_cfg() -> NodeBodyConfig {
        NodeBodyConfig {
            agents: std::collections::BTreeMap::new(),
            provider_profiles: std::collections::BTreeMap::new(),
            default_provider: None,
            upstream: Upstream::Local,
            default_work_dir: None,
        }
    }

    #[test]
    fn tool_names_are_classified_conservatively() {
        assert_eq!(classify_tool("Bash"), GateCategory::Execute);
        assert_eq!(classify_tool("run_command"), GateCategory::Execute);
        assert_eq!(classify_tool("Edit"), GateCategory::Edit);
        assert_eq!(classify_tool("Write"), GateCategory::Edit);
        assert_eq!(classify_tool("MultiEdit"), GateCategory::Edit);
        assert_eq!(classify_tool("Read"), GateCategory::Other);
        // 判不出来 → Other：ask 会问、edit 不会自动放行。
        assert_eq!(classify_tool("mystery_tool"), GateCategory::Other);
    }

    #[test]
    fn a_local_profile_injects_its_protocols_env_and_reports_effective() {
        let mut cfg = body_cfg();
        cfg.provider_profiles.insert(
            "anthropic".into(),
            ProviderProfile {
                name: "anthropic".into(),
                protocol: ProviderProtocol::Anthropic,
                base_url: "https://api.example/anthropic".into(),
                api_key_env: None,
            },
        );
        let factory = NodeBodyFactory::new(cfg);
        let resolved = factory.resolve_provider(Some("anthropic")).unwrap();
        assert_eq!(resolved.provider.as_deref(), Some("anthropic"));
        assert_eq!(resolved.desired.as_deref(), Some("anthropic"));
        assert!(resolved.cause.is_none());
        assert_eq!(
            resolved.env,
            vec![(
                "ANTHROPIC_BASE_URL".to_string(),
                "https://api.example/anthropic".to_string()
            )]
        );
    }

    #[test]
    fn an_unknown_profile_is_refused_with_the_configured_names() {
        let factory = NodeBodyFactory::new(body_cfg());
        let err = factory.resolve_provider(Some("ghost")).unwrap_err();
        assert_eq!(err.code, SessionRejectCode::ProviderUnavailable);
        assert!(err.cause.contains("ghost"), "{}", err.cause);
        assert!(err.cause.contains("未配置"), "{}", err.cause);
    }

    #[test]
    fn a_missing_credential_env_is_refused_not_launched_without_a_key() {
        let mut cfg = body_cfg();
        cfg.provider_profiles.insert(
            "anthropic".into(),
            ProviderProfile {
                name: "anthropic".into(),
                protocol: ProviderProtocol::Anthropic,
                base_url: "https://api.example".into(),
                api_key_env: Some("SEBAS_NODE_TEST_DEFINITELY_UNSET_KEY".into()),
            },
        );
        let factory = NodeBodyFactory::new(cfg);
        let err = factory.resolve_provider(Some("anthropic")).unwrap_err();
        assert_eq!(err.code, SessionRejectCode::ProviderUnavailable);
        assert!(
            err.cause.contains("SEBAS_NODE_TEST_DEFINITELY_UNSET_KEY"),
            "成因必须指名那个环境变量：{}",
            err.cause
        );
    }

    #[test]
    fn a_default_profile_is_applied_but_desired_stays_unset_and_says_why() {
        let mut cfg = body_cfg();
        cfg.default_provider = Some("anthropic".into());
        cfg.provider_profiles.insert(
            "anthropic".into(),
            ProviderProfile {
                name: "anthropic".into(),
                protocol: ProviderProtocol::Anthropic,
                base_url: "https://api.example".into(),
                api_key_env: None,
            },
        );
        let factory = NodeBodyFactory::new(cfg);
        let resolved = factory.resolve_provider(None).unwrap();
        assert_eq!(resolved.provider.as_deref(), Some("anthropic"));
        assert!(
            resolved.desired.is_none(),
            "控制面没选 → desired 就是「没选」，不能记成它选了这个"
        );
        assert!(resolved.cause.as_deref().unwrap().contains("default_provider"));
    }

    #[test]
    fn the_control_plane_router_upstream_needs_the_address_it_was_told() {
        // 没有地址（控制面没告知）→ 拒绝，不猜。
        let mut cfg = body_cfg();
        cfg.upstream = Upstream::ControlPlaneRouter;
        let factory = NodeBodyFactory::new(cfg);
        let err = factory.resolve_provider(None).unwrap_err();
        assert_eq!(err.code, SessionRejectCode::ProviderUnavailable);
        assert!(err.cause.contains("router"), "{}", err.cause);
        assert!(err.cause.contains("没有"), "{}", err.cause);

        // 控制面告知 → 用**它给的**地址（节点零 provider 凭据）。
        let mut cfg = body_cfg();
        cfg.upstream = Upstream::ControlPlaneRouter;
        let factory = NodeBodyFactory::new(cfg);
        factory.note_router_endpoint(Some(RouterEndpoint {
            url: "http://10.0.0.5:8787".into(),
            token: Some("router-token".into()),
        }));
        let resolved = factory.resolve_provider(None).unwrap();
        assert_eq!(resolved.provider.as_deref(), Some("control-plane-router"));
        assert_eq!(
            resolved.env,
            vec![
                (
                    "ANTHROPIC_BASE_URL".to_string(),
                    "http://10.0.0.5:8787".to_string()
                ),
                ("ANTHROPIC_AUTH_TOKEN".to_string(), "router-token".to_string()),
            ]
        );
    }

    #[test]
    fn a_project_less_session_without_a_default_work_dir_is_refused() {
        let mut cfg = body_cfg();
        cfg.agents.insert(
            "claude".into(),
            AgentSection {
                command: Some("/bin/true".into()),
                args: Vec::new(),
                driver: Some(AgentDriverKind::Claude),
                enforces_mode: None,
            },
        );
        let factory = NodeBodyFactory::new(cfg);
        let err = factory
            .make(&BodySpec {
                kind: "claude",
                project_dir: None,
                model: None,
                mode: None,
                provider: None,
            })
            .unwrap_err();
        assert_eq!(err.code, SessionRejectCode::UnusableProjectDir);
        assert!(err.cause.contains("default_work_dir"), "{}", err.cause);
    }

    #[test]
    fn the_factory_only_offers_kinds_whose_runtime_is_wired() {
        let mut cfg = body_cfg();
        cfg.agents.insert(
            "claude".into(),
            AgentSection {
                command: Some("claude".into()),
                args: Vec::new(),
                driver: Some(AgentDriverKind::Claude),
                enforces_mode: Some(true),
            },
        );
        cfg.agents.insert(
            "registered-only".into(),
            AgentSection {
                command: Some("whatever".into()),
                args: Vec::new(),
                driver: None,
                enforces_mode: None,
            },
        );
        let factory = NodeBodyFactory::new(cfg);
        let kinds = factory.available_kinds();
        assert!(kinds.contains(&"echo".to_string()));
        assert!(kinds.contains(&"claude".to_string()));
        assert!(
            !kinds.contains(&"registered-only".to_string()),
            "没接运行时的不算可服务"
        );
    }

    #[test]
    fn provider_profile_section_parses_with_a_default_protocol() {
        // 形状断言：profile 段缺省协议是 Anthropic，字段名与文档一致。
        let section: ProviderProfileSection = toml::from_str(
            "base_url = \"https://api.example\"\napi_key_env = \"KEY\"\n",
        )
        .unwrap();
        assert_eq!(section.protocol, None);
        assert_eq!(section.base_url.as_deref(), Some("https://api.example"));
        assert_eq!(section.api_key_env.as_deref(), Some("KEY"));
    }

    #[test]
    fn draining_stops_at_the_first_gate_so_nothing_past_it_is_logged() {
        // 门之后的条目还没获准发生；执行体必须把它们留在队列里，等决议后再交。
        let shared = Shared::new();
        let output = |text: &str| BodyEvent {
            kind: "output".into(),
            text: text.into(),
            data: None,
            gate: None,
        };
        let gate = BodyEvent {
            kind: "permission_request".into(),
            text: "bash".into(),
            data: None,
            gate: Some(GateRequest {
                tool: "bash".into(),
                category: GateCategory::Execute,
                resume_token: Some("t".into()),
            }),
        };
        shared.push(output("before"));
        shared.push(gate);
        shared.push(output("after"));

        let taken = shared.take_until_gate();
        assert_eq!(taken.len(), 2, "取到门为止（含）");
        assert!(taken[1].gate.is_some());
        let rest = shared.take_until_gate();
        assert_eq!(rest.len(), 1, "门之后的条目仍留在执行体里");
        assert_eq!(rest[0].text, "after");
    }
}
