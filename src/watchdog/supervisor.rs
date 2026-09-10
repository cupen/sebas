//! 每服务一个监督 task：spawn、readiness 门、崩溃退避、命令处理。
//!
//! 设计（design.md D1/D2）：每个受管服务一个 tokio task，独占持有自己的
//! child、崩溃计数器、期望状态；对外只暴露命令 mpsc + 快照。core 特有的
//! 「新二进制未就绪 → 自动回滚」通过可选钩子注入，监督循环本身对所有服务
//! 一视同仁。崩溃退避状态（窗口/上限/超限冷却）封装在 [`CrashPolicy`]，
//! 可独立同步单测。

use crate::error::Result;
use crate::watchdog::EXIT_BIND_FAILED;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tracing::{error, info, warn};

/// 崩溃计数窗口：超过该间隔未崩溃则计数重置。
const CRASH_WINDOW: Duration = Duration::from_secs(3600);
/// 窗口内连续崩溃上限：超过后进入冷却（睡眠后重置计数继续监督）。
const MAX_CRASHES: u32 = 3;
/// 超限冷却时长（watchdog 绝不因 child 崩溃而退出）。
const OVER_LIMIT_COOLDOWN: Duration = Duration::from_secs(30);
/// 崩溃后重启前的固定等待。
pub const RESTART_DELAY: Duration = Duration::from_secs(1);
/// spawn 失败（缺二进制等）后的重试等待。
const SPAWN_RETRY_DELAY: Duration = Duration::from_secs(5);
/// 优雅停止宽限期：SIGTERM → 宽限 → SIGKILL。
const STOP_GRACE: Duration = Duration::from_secs(5);
/// spawn 失败计数窗口（fail-fast-on-startup-errors D7）：与 crash 窗口同为
/// 1 h——距上次失败超过窗口即重置计数，瞬态故障不累积成终态。
pub const SPAWN_FAILURE_WINDOW: Duration = Duration::from_secs(3600);
/// 连续 spawn 失败的默认上限（`[watchdog] max_spawn_failures` 可配）。
pub const DEFAULT_MAX_SPAWN_FAILURES: u32 = 3;

// ─── 身份与状态 ────────────────────────────────────────────

/// 受管服务名。监督循环对所有名字一视同仁；core 特有行为经钩子注入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceName {
    Core,
    WebUi,
    Router,
    Im,
}

impl ServiceName {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServiceName::Core => "core",
            ServiceName::WebUi => "webui",
            ServiceName::Router => "router",
            ServiceName::Im => "im",
        }
    }
}

/// 服务观测状态（对外快照；真实值，绝不同步硬编码）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    /// 已 spawn，等待 readiness 信号（或无 readiness 概念的服务）。
    Starting,
    Running,
    /// 崩溃后等待退避/冷却结束，即将重启。
    Restarting,
    /// 期望关闭（ServiceSet off / Stop）：child 已停，不重启。
    Stopped,
    /// 配置层从未启用：没有 child，也没有运行时覆盖。
    Disabled,
    /// 服务因 bind 失败等外部原因进入降级态，不自动重试；
    /// 等待 Restart 命令复位后重新 spawn。
    Degraded,
    /// fail-fast-on-startup-errors：连续 spawn 失败达到上限的终态。服务
    /// 不再重试、监督 task 结束；watchdog 整体以 EX_TEMPFAIL (75) 退出。
    /// 只有 Restart 命令（新 watchdog 生命周期）能离开该态。
    FailedStartup,
}

/// 一次（累计的）spawn 失败记录：`sebas ctl status` 的 `startup_failure`
/// 字段数据源（fail-fast-on-startup-errors D6）。窗口内每次失败都更新，
/// 成功 spawn/ready 后清空。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFailureInfo {
    /// 本窗口内连续失败次数（终态时 == 上限）。
    pub count: u32,
    /// 最近一次失败的 stderr 摘要（spawn 错误消息 / 退出分类）。
    pub last_stderr: String,
    /// 最近一次失败的 Unix 秒时间戳（ISO 化留给展示层）。
    pub at_unix: i64,
}

/// 服务达到 `failed-startup` 终态时经 watch 通道上报给 watchdog 主循环的
/// 事件（任务 2.1/2.3：watchdog 收到后 shutdown 并以 75 退出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFailureEvent {
    pub service: ServiceName,
    pub info: StartupFailureInfo,
}

/// 启动失败事件的共享通道形状：`None` = 无终态失败。
pub type StartupFailureTx = watch::Sender<Option<StartupFailureEvent>>;
pub type StartupFailureRx = watch::Receiver<Option<StartupFailureEvent>>;

/// 当前时刻的 Unix 秒（失败记录时间戳）。
pub(crate) fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 复用 control 面的期望态命名，避免两套词汇。
pub use crate::watchdog::control::DesiredState;

/// 单服务快照。`started_at` 供 uptime 计算；`startup_failure` 是窗口内
/// 最近一次 spawn 失败的累计记录（无失败 = None）。
#[derive(Debug, Clone)]
pub struct ServiceSnapshot {
    pub name: ServiceName,
    pub state: ServiceState,
    pub desired: DesiredState,
    pub pid: Option<u32>,
    pub started_at: Option<Instant>,
    pub startup_failure: Option<StartupFailureInfo>,
}

// ─── 崩溃退避（纯状态机，同步单测） ─────────────────────────

/// 按服务独立的崩溃退避策略。
#[derive(Debug)]
pub struct CrashPolicy {
    window: Duration,
    max_crashes: u32,
    restart_delay: Duration,
    over_limit_cooldown: Duration,
    count: u32,
    last_crash: Option<Instant>,
}

/// 登记一次崩溃后的决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashDecision {
    /// 窗口内未超限：等待 delay 后重启。
    Restart { delay: Duration },
    /// 超限：冷却 delay（期间不 spawn），计数已重置，冷却后继续监督。
    CoolDown { delay: Duration },
}

impl CrashPolicy {
    pub fn new(
        window: Duration,
        max_crashes: u32,
        restart_delay: Duration,
        over_limit_cooldown: Duration,
    ) -> Self {
        Self {
            window,
            max_crashes,
            restart_delay,
            over_limit_cooldown,
            count: 0,
            last_crash: None,
        }
    }

    /// 生产默认策略。
    pub fn default_policy() -> Self {
        Self::new(
            CRASH_WINDOW,
            MAX_CRASHES,
            RESTART_DELAY,
            OVER_LIMIT_COOLDOWN,
        )
    }

    /// 登记一次崩溃并返回下一步动作。永不返回「放弃」——watchdog 不退出。
    pub fn register_crash(&mut self) -> CrashDecision {
        let now = Instant::now();
        if let Some(last) = self.last_crash
            && now.duration_since(last) > self.window
        {
            self.count = 0;
        }
        self.count += 1;
        self.last_crash = Some(now);

        if self.count > self.max_crashes {
            self.count = 0;
            CrashDecision::CoolDown {
                delay: self.over_limit_cooldown,
            }
        } else {
            CrashDecision::Restart {
                delay: self.restart_delay,
            }
        }
    }

    /// 非 crash 原因的重启（ServiceRestart 命令）不计数。
    pub fn reset(&mut self) {
        self.count = 0;
        self.last_crash = None;
    }
}

// ─── spawn 失败上限（纯状态机，同步单测） ───────────────────

/// 连续 spawn 失败的计数策略（fail-fast-on-startup-errors 2.1，D1/D7）。
/// 与 [`CrashPolicy`] 不同：达到上限不是冷却，而是**终态**——调用方（监督
/// task）必须停止重试并触发 watchdog 退出。
#[derive(Debug)]
pub struct SpawnFailurePolicy {
    window: Duration,
    max: u32,
    count: u32,
    last_failure: Option<Instant>,
}

/// 登记一次 spawn 失败后的决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnFailureDecision {
    /// 未达上限：按 1 s 退避重试（窗口内每次失败都写结构化日志 + 可见）。
    Retry,
    /// 连续失败达到上限：服务进入 `failed-startup` 终态，watchdog 以 75 退出。
    Terminal,
}

impl SpawnFailurePolicy {
    pub fn new(window: Duration, max: u32) -> Self {
        // 上限至少为 1：0 会让第一次失败就直接终态（且除零语义无意义）。
        let max = max.max(1);
        Self {
            window,
            max,
            count: 0,
            last_failure: None,
        }
    }

    /// 登记一次失败。距上次失败超过窗口则计数先重置（D7b）。
    pub fn register_failure(&mut self) -> SpawnFailureDecision {
        let now = Instant::now();
        if let Some(last) = self.last_failure
            && now.duration_since(last) > self.window
        {
            self.count = 0;
        }
        self.count += 1;
        self.last_failure = Some(now);
        if self.count >= self.max {
            SpawnFailureDecision::Terminal
        } else {
            SpawnFailureDecision::Retry
        }
    }

    /// 成功 spawn（无 readiness 门）/ ready（有门）后重置（D7a）。
    pub fn reset(&mut self) {
        self.count = 0;
        self.last_failure = None;
    }

    /// 当前连续失败计数。
    pub fn count(&self) -> u32 {
        self.count
    }
}

// ─── 命令与服务句柄 ────────────────────────────────────────

/// 对监督 task 的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCommand {
    /// 期望态置 Enabled：未运行则 spawn。
    Start,
    /// 期望态置 Disabled：停 child，不重启。
    Stop,
    /// 立即重启。core 上 `is_upgrade` 标记下一次 spawn 是新安装的二进制，
    /// 其未就绪即退将被分类为 NewBinaryNotReady（走回滚钩子而非崩溃计数）。
    Restart { is_upgrade: bool },
    /// watchdog exited：停 child 并结束监督 task。
    Shutdown,
}

/// 外部持有的单服务句柄。
#[derive(Debug, Clone)]
pub struct ServiceHandle {
    pub name: ServiceName,
    tx: mpsc::Sender<ServiceCommand>,
    snapshot: Arc<Mutex<ServiceSnapshot>>,
    /// 终态启动失败事件的接收端（克隆共享同一 watch 通道）。`changed()`
    /// 返回后读 `borrow()` 即得 `Some(StartupFailureEvent)`。
    startup_failures: StartupFailureRx,
}

impl ServiceHandle {
    pub async fn snapshot(&self) -> ServiceSnapshot {
        self.snapshot.lock().await.clone()
    }

    pub async fn send(&self, cmd: ServiceCommand) -> bool {
        self.tx.send(cmd).await.is_ok()
    }

    /// 测试/内部用：直接读共享快照指针。
    pub fn shared_snapshot(&self) -> Arc<Mutex<ServiceSnapshot>> {
        self.snapshot.clone()
    }

    /// 终态启动失败事件的接收端（watchdog 主循环 select 用）。
    pub fn startup_failures(&self) -> StartupFailureRx {
        self.startup_failures.clone()
    }
}

// ─── spawn 抽象 ────────────────────────────────────────────

/// 一次 spawn 出来的服务实例：child + 可选 readiness 门。
pub struct SpawnedInstance {
    pub child: Box<dyn ManagedChild>,
    /// readiness 信号（core 的管道 ready 握手）。`None` = 无 readiness 概念，
    /// spawn 成功即视为 Running。
    pub readiness: Option<oneshot::Receiver<()>>,
}

/// 服务实例抽象，便于 FakeChild 注入测试监督循环。
#[async_trait::async_trait]
pub trait ManagedChild: Send {
    fn pid(&self) -> Option<u32>;
    /// 等待退出，返回退出码（不可得时 None）。
    async fn wait(&mut self) -> Option<i32>;
    /// 优雅停止：SIGTERM → 宽限 → SIGKILL。
    async fn stop(&mut self);
}

/// spawn 工厂抽象。
#[async_trait::async_trait]
pub trait ServiceSpawner: Send + Sync {
    async fn spawn(&self) -> Result<SpawnedInstance>;
}

/// core 专属钩子：升级后的新二进制未就绪即退出时调用（自动回滚）。
/// 钩子返回后监督循环继续用（回滚后的）二进制重启。
pub type UnreadyAfterUpgradeHook =
    Arc<dyn Fn() -> futures_util::future::BoxFuture<'static, ()> + Send + Sync>;

/// 服务规格。
pub struct ServiceSpec {
    pub name: ServiceName,
    pub spawner: Arc<dyn ServiceSpawner>,
    /// 初始期望态（ServiceManager 三层合成后的结果）。
    /// `Disabled` 且配置层从未启用时，快照态应报告 `Disabled`；
    /// 监督 task 内部统一用 Stopped 表示「不 spawn」，Disabled 语义由
    /// manager 在快照层标注。
    pub desired: DesiredState,
    /// 崩溃退避参数（测试注入小值）。
    pub crash: CrashPolicy,
    pub restart_delay: Duration,
    pub spawn_retry_delay: Duration,
    /// core：NewBinaryNotReady 自动回滚钩子。
    pub on_unready_after_upgrade: Option<UnreadyAfterUpgradeHook>,
    /// 连续 spawn 失败上限（fail-fast-on-startup-errors D1，默认 3）。
    pub max_spawn_failures: u32,
    /// 终态启动失败事件的上报通道（watchdog 主循环据此退出 75）。`None` =
    /// 单测形态，只置快照状态不上报。
    pub startup_failure_tx: Option<StartupFailureTx>,
}

impl ServiceSpec {
    pub fn new(name: ServiceName, spawner: Arc<dyn ServiceSpawner>, desired: DesiredState) -> Self {
        Self {
            name,
            spawner,
            desired,
            crash: CrashPolicy::default_policy(),
            restart_delay: RESTART_DELAY,
            spawn_retry_delay: SPAWN_RETRY_DELAY,
            on_unready_after_upgrade: None,
            max_spawn_failures: DEFAULT_MAX_SPAWN_FAILURES,
            startup_failure_tx: None,
        }
    }
}

// ─── 监督循环 ──────────────────────────────────────────────

/// 服务实例退出的原因分类（监督循环内部使用）。
enum Exit {
    /// child 自行退出（崩溃）。
    Crashed(Option<i32>),
    /// Stop 命令：期望态已置 Disabled。
    Stopped,
    /// Restart 命令：立即进入下一轮 spawn。
    Restarted,
}

/// 启动一个服务的监督 task。返回外部句柄 + JoinHandle。
pub fn start_supervision(spec: ServiceSpec) -> (ServiceHandle, tokio::task::JoinHandle<()>) {
    let name = spec.name;
    let (tx, rx) = mpsc::channel(16);
    let (fail_tx, fail_rx) = watch::channel(None);
    let snapshot = Arc::new(Mutex::new(ServiceSnapshot {
        name,
        state: ServiceState::Starting,
        desired: spec.desired,
        pid: None,
        started_at: None,
        startup_failure: None,
    }));
    let handle = ServiceHandle {
        name,
        tx,
        snapshot: snapshot.clone(),
        startup_failures: fail_rx,
    };
    let task = tokio::spawn(supervise(spec, rx, snapshot, fail_tx));
    (handle, task)
}

async fn set_state(snapshot: &Mutex<ServiceSnapshot>, state: ServiceState) {
    snapshot.lock().await.state = state;
}

/// 把一次 spawn 失败写进快照（`sebas ctl status` 的 startup_failure 数据源，
/// spec「spawn failure within limit still logged」：未达上限也必须可见）。
async fn record_failure(snapshot: &Mutex<ServiceSnapshot>, count: u32, cause: &str) -> StartupFailureInfo {
    let info = StartupFailureInfo {
        count,
        last_stderr: cause.to_string(),
        at_unix: now_unix_secs(),
    };
    snapshot.lock().await.startup_failure = Some(info.clone());
    info
}

/// 进入 `failed-startup` 终态：置状态、上报事件、日志。监督 task 随即结束。
async fn enter_failed_startup(
    snapshot: &Mutex<ServiceSnapshot>,
    fail_tx: Option<&StartupFailureTx>,
    service: ServiceName,
    info: StartupFailureInfo,
) {
    error!(
        service = service.as_str(),
        count = info.count,
        last_error = %info.last_stderr,
        "spawn failures reached the limit; entering failed-startup (terminal), watchdog exits 75"
    );
    set_state(snapshot, ServiceState::FailedStartup).await;
    if let Some(tx) = fail_tx {
        let _ = tx.send(Some(StartupFailureEvent {
            service,
            info: info.clone(),
        }));
    }
}

/// 监督 task 主循环。fail_tx 在终态失败时发送 `Some(event)`。
async fn supervise(
    spec: ServiceSpec,
    mut cmd_rx: mpsc::Receiver<ServiceCommand>,
    snapshot: Arc<Mutex<ServiceSnapshot>>,
    fail_tx: watch::Sender<Option<StartupFailureEvent>>,
) {
    let name = spec.name;
    let mut desired = spec.desired;
    let mut policy = spec.crash;
    // core：下一次 spawn 是否为升级产生的新二进制。
    let mut just_performed_update = false;
    // 连续 spawn 失败计数（fail-fast-on-startup-errors 2.1）。
    let mut spawn_policy = SpawnFailurePolicy::new(SPAWN_FAILURE_WINDOW, spec.max_spawn_failures);

    info!(service = name.as_str(), "supervision task started");

    loop {
        // 期望关闭：不 spawn，等命令。
        if desired == DesiredState::Disabled {
            set_state(&snapshot, ServiceState::Stopped).await;
            match cmd_rx.recv().await {
                Some(ServiceCommand::Start) | Some(ServiceCommand::Restart { .. }) => {
                    desired = DesiredState::Enabled;
                    snapshot.lock().await.desired = desired;
                    policy.reset();
                    continue;
                }
                Some(ServiceCommand::Stop) | Some(ServiceCommand::Shutdown) | None => {
                    return;
                }
            }
        }

        // 降级态：bind 失败等外部原因，不自动重试，等 Restart/Stop 命令。
        if snapshot.lock().await.state == ServiceState::Degraded {
            match cmd_rx.recv().await {
                Some(ServiceCommand::Restart { .. }) | Some(ServiceCommand::Start) => {
                    policy.reset();
                    set_state(&snapshot, ServiceState::Restarting).await;
                    continue;
                }
                Some(ServiceCommand::Stop) => {
                    desired = DesiredState::Disabled;
                    set_state(&snapshot, ServiceState::Stopped).await;
                    continue;
                }
                Some(ServiceCommand::Shutdown) | None => {
                    let mut snap = snapshot.lock().await;
                    snap.state = ServiceState::Stopped;
                    snap.pid = None;
                    return;
                }
            }
        }

        // spawn 一次 incarnation。fail-fast-on-startup-errors：窗口内连续
        // 失败按 1 s 退避重试（每次写结构化日志 + 快照可见），达到上限进入
        // `failed-startup` 终态并让 watchdog 退出 75，不再无限重试。
        let instance = match spec.spawner.spawn().await {
            Ok(instance) => instance,
            Err(e) => {
                let cause = e.to_string();
                match spawn_policy.register_failure() {
                    SpawnFailureDecision::Retry => {
                        warn!(
                            service = name.as_str(),
                            count = spawn_policy.count(),
                            limit = spec.max_spawn_failures,
                            "spawn failed: {cause}, will retry"
                        );
                        record_failure(&snapshot, spawn_policy.count(), &cause).await;
                        set_state(&snapshot, ServiceState::Restarting).await;
                        tokio::time::sleep(spec.spawn_retry_delay).await;
                        continue;
                    }
                    SpawnFailureDecision::Terminal => {
                        let info = record_failure(&snapshot, spawn_policy.count(), &cause).await;
                        enter_failed_startup(&snapshot, Some(&fail_tx), name, info).await;
                        return;
                    }
                }
            }
        };

        // 成功 spawn。无 readiness 门的服务 spawn 即 Running（ready 等价），
        // 失败计数就此清零（D7a）；有门的等服务发 ready 再清。
        if instance.readiness.is_none() {
            spawn_policy.reset();
            snapshot.lock().await.startup_failure = None;
        }

        let mut child = instance.child;
        let mut readiness = instance.readiness;
        // 启动是否已被确认（无门服务 spawn 即确认；有门服务等 ready 信号）。
        // 只有「启动未确认」的退出才计入 failed-startup 计数器——已确认后
        // 的退出是运行期崩溃，走既有 crash backoff（永不放弃）。
        let mut startup_confirmed = readiness.is_none();
        let pid = child.pid();
        {
            let mut snap = snapshot.lock().await;
            // 无 readiness 门的进程（webui/router）：spawn 即 Running。
            snap.state = match readiness {
                Some(_) => ServiceState::Starting,
                None => ServiceState::Running,
            };
            snap.pid = pid;
            snap.started_at = Some(Instant::now());
        }
        info!(service = name.as_str(), pid = %pid_str(pid), "child spawned");

        let mut received_ready = false;
        let exit = loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(ServiceCommand::Start) => continue, // 已在运行
                        Some(ServiceCommand::Stop) => {
                            desired = DesiredState::Disabled;
                            snapshot.lock().await.desired = desired;
                            child.stop().await;
                            break Exit::Stopped;
                        }
                        Some(ServiceCommand::Restart { is_upgrade }) => {
                            if is_upgrade {
                                just_performed_update = true;
                            }
                            policy.reset();
                            child.stop().await;
                            break Exit::Restarted;
                        }
                        Some(ServiceCommand::Shutdown) | None => {
                            child.stop().await;
                            // shutdown_all 轮询快照等待全部停稳；不置
                            // Stopped 会让等待白转满 10s 超时。
                            let mut snap = snapshot.lock().await;
                            snap.state = ServiceState::Stopped;
                            snap.pid = None;
                            info!(service = name.as_str(), "supervision task stopped");
                            return;
                        }
                    }
                }
                code = child.wait() => break Exit::Crashed(code),
                // readiness 门：Ok 信号到达即 Running；门异常（reader 随子进程
                // 退出而中止，或 None）保持 pending，交给 wait 分支分类退出。
                _ = async {
                    match readiness.as_mut() {
                        Some(rx) => {
                            if rx.await.is_err() {
                                std::future::pending::<()>().await;
                            }
                        }
                        None => std::future::pending::<()>().await,
                    }
                }, if readiness.is_some() => {
                    received_ready = true;
                    startup_confirmed = true;
                    readiness = None;
                    // 达到 ready = 启动成功：spawn 失败计数清零（D7a）。
                    spawn_policy.reset();
                    snapshot.lock().await.startup_failure = None;
                    set_state(&snapshot, ServiceState::Running).await;
                    info!(service = name.as_str(), pid = %pid_str(pid), "child ready");
                }
            }
        };

        match exit {
            Exit::Stopped => {
                let mut snap = snapshot.lock().await;
                snap.state = ServiceState::Stopped;
                snap.pid = None;
                continue;
            }
            Exit::Restarted => {
                // 期望态仍是 Enabled，立即 respawn。
                continue;
            }
            Exit::Crashed(code) => {
                warn!(
                    service = name.as_str(), pid = %pid_str(pid), code = %code_str(code), ready = received_ready,
                    "child exited"
                );
                snapshot.lock().await.pid = None;

                // core 特例：升级后的新二进制从未 Ready → 回滚钩子，不计 crash。
                if just_performed_update && !received_ready {
                    warn!(
                        service = name.as_str(),
                        "new binary exited without ready after upgrade, running rollback hook"
                    );
                    if let Some(hook) = spec.on_unready_after_upgrade.as_ref() {
                        hook().await;
                    }
                    just_performed_update = false;
                    set_state(&snapshot, ServiceState::Restarting).await;
                    tokio::time::sleep(spec.restart_delay).await;
                    continue;
                }
                just_performed_update = false;

                // 退出码 75 = bind 失败（如端口占用）→ 标记 Degraded，
                // 不自动重试，等 Restart 命令。（既有降级语义，不变。）
                if code == Some(EXIT_BIND_FAILED) {
                    warn!(
                        service = name.as_str(),
                        "bind failed (port in use?), marking Degraded, waiting for Restart"
                    );
                    set_state(&snapshot, ServiceState::Degraded).await;
                    continue;
                }

                // fail-fast-on-startup-errors（spec「early-fatal counts toward
                // startup-failure limit」）：启动未确认（未 ready / 无门服务
                // 刚 spawn 即退）的 early-fatal 退出与 spawn 失败同等待遇，
                // 计入终态计数器；达到上限同样终态化。
                if !startup_confirmed {
                    let cause = format!(
                        "child exited before ready (code {code_str}, ready=false)",
                        code_str = code_str(code)
                    );
                    match spawn_policy.register_failure() {
                        SpawnFailureDecision::Retry => {
                            record_failure(&snapshot, spawn_policy.count(), &cause).await;
                        }
                        SpawnFailureDecision::Terminal => {
                            let info =
                                record_failure(&snapshot, spawn_policy.count(), &cause).await;
                            enter_failed_startup(&snapshot, Some(&fail_tx), name, info).await;
                            return;
                        }
                    }
                }

                set_state(&snapshot, ServiceState::Restarting).await;
                match policy.register_crash() {
                    CrashDecision::Restart { delay } => {
                        tokio::time::sleep(delay).await;
                    }
                    CrashDecision::CoolDown { delay } => {
                        warn!(service = name.as_str(), "crash loop limit reached, cooling down");
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
    }
}

// ─── 真实进程 adapter ──────────────────────────────────────

/// 真实子进程：包装 tokio Child，实现 ManagedChild。
pub struct ProcessChild(pub tokio::process::Child);
#[async_trait::async_trait]
impl ManagedChild for ProcessChild {
    fn pid(&self) -> Option<u32> {
        self.0.id()
    }

    async fn wait(&mut self) -> Option<i32> {
        self.0.wait().await.ok().and_then(|s| s.code())
    }

    async fn stop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.id() {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
        #[cfg(not(unix))]
        let _ = self.0.start_kill();

        match tokio::time::timeout(STOP_GRACE, self.0.wait()).await {
            Ok(_) => {}
            Err(_) => {
                warn!("child graceful exit timed out, killing");
                let _ = self.0.kill().await;
            }
        }
    }
}

// ─── tests ─────────────────────────────────────────────────

/// `Option` 值在日志里输出裸数字（未知输出 `-`），不输出 `Some(1234)`。
fn pid_str(pid: Option<u32>) -> String {
    pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into())
}

fn code_str(code: Option<i32>) -> String {
    code.map(|c| c.to_string()).unwrap_or_else(|| "-".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SebasError;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 可控的 FakeChild：exit 信号经 watch 触发，退出码可配置。
    struct FakeChild {
        pid: u32,
        exited: tokio::sync::watch::Receiver<bool>,
        exit_triggered: tokio::sync::watch::Sender<bool>,
        exit_code: i32,
    }

    struct FakeSpawner {
        spawns: AtomicUsize,
        /// spawn 后多少毫秒自动退出（0 = 不自动退出，等命令）。
        auto_exit_ms: u64,
        /// spawn 是否直接失败。
        fail: bool,
        /// child 退出码（默认 1）。
        exit_code: i32,
    }

    impl FakeSpawner {
        fn auto(exit_ms: u64) -> Arc<Self> {
            Arc::new(Self {
                spawns: AtomicUsize::new(0),
                auto_exit_ms: exit_ms,
                fail: false,
                exit_code: 1,
            })
        }

        fn bind_failed() -> Arc<Self> {
            Arc::new(Self {
                spawns: AtomicUsize::new(0),
                auto_exit_ms: 10,
                fail: false,
                exit_code: EXIT_BIND_FAILED,
            })
        }
    }

    #[async_trait::async_trait]
    impl ServiceSpawner for FakeSpawner {
        async fn spawn(&self) -> Result<SpawnedInstance> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(SebasError::Upgrade("fake spawn failure".into()));
            }
            let (tx, rx) = tokio::sync::watch::channel(false);
            let auto_ms = self.auto_exit_ms;
            let exit_tx = tx.clone();
            let exit_code = self.exit_code;
            tokio::spawn(async move {
                if auto_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(auto_ms)).await;
                    let _ = exit_tx.send(true);
                }
            });
            Ok(SpawnedInstance {
                child: Box::new(FakeChild {
                    pid: 4242,
                    exited: rx.clone(),
                    exit_triggered: tx,
                    exit_code,
                }),
                readiness: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl ManagedChild for FakeChild {
        fn pid(&self) -> Option<u32> {
            Some(self.pid)
        }

        async fn wait(&mut self) -> Option<i32> {
            let mut exited = self.exited.clone();
            while !*exited.borrow() {
                if exited.changed().await.is_err() {
                    return None;
                }
            }
            Some(self.exit_code)
        }

        async fn stop(&mut self) {
            let _ = self.exit_triggered.send(true);
        }
    }

    fn fast_spec(spawner: Arc<dyn ServiceSpawner>, desired: DesiredState) -> ServiceSpec {
        let mut spec = ServiceSpec::new(ServiceName::WebUi, spawner, desired);
        spec.crash = CrashPolicy::new(
            Duration::from_millis(50),
            3,
            Duration::from_millis(10),
            Duration::from_millis(30),
        );
        spec.restart_delay = Duration::from_millis(10);
        spec.spawn_retry_delay = Duration::from_millis(10);
        spec
    }

    #[tokio::test]
    async fn crash_policy_counters_and_cool_down() {
        let mut p = CrashPolicy::new(
            Duration::from_secs(100),
            3,
            Duration::from_millis(10),
            Duration::from_secs(30),
        );
        for _ in 0..3 {
            assert!(matches!(p.register_crash(), CrashDecision::Restart { .. }));
        }
        // 第 4 次：超限 → 冷却 + 计数重置。
        assert!(matches!(p.register_crash(), CrashDecision::CoolDown { .. }));
        // 冷却后计数已重置，再次崩溃回到普通 Restart。
        assert!(matches!(p.register_crash(), CrashDecision::Restart { .. }));
    }

    #[tokio::test]
    async fn unexpected_exit_restarts_with_backoff() {
        let spawner = FakeSpawner::auto(20); // 20ms 后自动退出
        let (handle, task) = start_supervision(fast_spec(spawner.clone(), DesiredState::Enabled));
        // 等足够多次崩溃-重启循环发生。
        tokio::time::sleep(Duration::from_millis(150)).await;
        let spawns = spawner.spawns.load(Ordering::SeqCst);
        assert!(spawns >= 2, "意外退出必须触发重启, spawns={spawns}");
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn stop_command_prevents_restart() {
        let spawner = FakeSpawner::auto(0); // 不自动退出
        let (handle, task) = start_supervision(fast_spec(spawner.clone(), DesiredState::Enabled));
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = handle.send(ServiceCommand::Stop).await;
        tokio::time::sleep(Duration::from_millis(80)).await;
        let snap = handle.snapshot().await;
        assert_eq!(snap.state, ServiceState::Stopped);
        assert_eq!(snap.desired, DesiredState::Disabled);
        assert_eq!(spawner.spawns.load(Ordering::SeqCst), 1, "Stop 后不得重启");
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn over_limit_cools_down_then_keeps_supervising() {
        let spawner = FakeSpawner::auto(1); // spawn 后 1ms 即退
        let mut spec = fast_spec(spawner.clone(), DesiredState::Enabled);
        spec.crash = CrashPolicy::new(
            Duration::from_secs(100),
            2,
            Duration::from_millis(5),
            Duration::from_millis(20),
        );
        spec.restart_delay = Duration::from_millis(5);
        let (handle, task) = start_supervision(spec);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let spawns = spawner.spawns.load(Ordering::SeqCst);
        assert!(
            spawns > 3,
            "超限后冷却并继续监督（永不放弃）, spawns={spawns}"
        );
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn spawn_failure_retries() {
        let spawner = Arc::new(FakeSpawner {
            spawns: AtomicUsize::new(0),
            auto_exit_ms: 0,
            fail: true,
            exit_code: 1,
        });
        let mut spec = fast_spec(spawner.clone(), DesiredState::Enabled);
        // 上限调高：本用例只验证「限内重试不退出监督 task」。
        spec.max_spawn_failures = 100;
        let (handle, task) = start_supervision(spec);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            spawner.spawns.load(Ordering::SeqCst) >= 2,
            "spawn 失败必须重试而非退出监督 task"
        );
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    // ── fail-fast-on-startup-errors 2.1：连续 spawn 失败的终态化 ──

    #[tokio::test]
    async fn spawn_failure_hits_limit_and_enters_failed_startup() {
        let spawner = Arc::new(FakeSpawner {
            spawns: AtomicUsize::new(0),
            auto_exit_ms: 0,
            fail: true,
            exit_code: 1,
        });
        let mut spec = fast_spec(spawner.clone(), DesiredState::Enabled);
        spec.max_spawn_failures = 3;
        let (handle, task) = start_supervision(spec);
        let mut failures = handle.startup_failures();

        // 恰好 3 次尝试后进入终态（1/2 次重试，第 3 次终止）。
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if handle.snapshot().await.state == ServiceState::FailedStartup {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "3 次连续 spawn 失败必须进入 failed-startup 终态"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            spawner.spawns.load(Ordering::SeqCst),
            3,
            "达到上限后不得再重试"
        );
        // 终态事件经 watch 通道上报（watchdog 主循环据此退出 75）。
        failures.changed().await.expect("failure channel open");
        let event = failures.borrow().clone();
        let event = event.expect("terminal failure event must be Some");
        assert_eq!(event.service, ServiceName::WebUi);
        assert_eq!(event.info.count, 3);
        assert!(
            event.info.last_stderr.contains("fake spawn failure"),
            "event carries the stderr summary: {event:?}"
        );
        // 快照带失败记录（ctl status 的数据源）。
        let snap = handle.snapshot().await;
        let sf = snap.startup_failure.expect("startup_failure recorded");
        assert_eq!(sf.count, 3);
        // 终态后监督 task 已结束：再等也不会有新 spawn。
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(spawner.spawns.load(Ordering::SeqCst), 3);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn core_spawn_failure_hits_limit_and_enters_failed_startup() {
        // enable-core-by-default 2.1：core 恒启动，其 spawn 连败同样进入
        // failed-startup 终态并上报 core 事件（watchdog 主循环据此退出 75）。
        let spawner = Arc::new(FakeSpawner {
            spawns: AtomicUsize::new(0),
            auto_exit_ms: 0,
            fail: true,
            exit_code: 1,
        });
        let mut spec = fast_spec(spawner, DesiredState::Enabled);
        spec.name = ServiceName::Core;
        spec.max_spawn_failures = 3;
        let (handle, task) = start_supervision(spec);
        let mut failures = handle.startup_failures();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while handle.snapshot().await.state != ServiceState::FailedStartup {
            assert!(
                tokio::time::Instant::now() < deadline,
                "core 连续 spawn 失败 3 次必须进入 failed-startup 终态"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        failures.changed().await.expect("failure channel open");
        let event = failures.borrow().clone().expect("terminal event must be Some");
        assert_eq!(event.service, ServiceName::Core);
        assert!(handle.snapshot().await.startup_failure.is_some());
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn spawn_failure_limit_is_configurable() {
        // N=1：一次失败即终态（D1 备选边界，spec R2 要求可配）。
        let spawner = Arc::new(FakeSpawner {
            spawns: AtomicUsize::new(0),
            auto_exit_ms: 0,
            fail: true,
            exit_code: 1,
        });
        let mut spec = fast_spec(spawner.clone(), DesiredState::Enabled);
        spec.max_spawn_failures = 1;
        let (handle, _task) = start_supervision(spec);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while handle.snapshot().await.state != ServiceState::FailedStartup {
            assert!(
                tokio::time::Instant::now() < deadline,
                "N=1 时一次失败必须终态"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(spawner.spawns.load(Ordering::SeqCst), 1);

        // N=10：3 次失败仍在重试（未达上限不终态）。
        let spawner10 = Arc::new(FakeSpawner {
            spawns: AtomicUsize::new(0),
            auto_exit_ms: 0,
            fail: true,
            exit_code: 1,
        });
        let mut spec10 = fast_spec(spawner10.clone(), DesiredState::Enabled);
        spec10.max_spawn_failures = 10;
        let (handle10, task10) = start_supervision(spec10);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let spawns = spawner10.spawns.load(Ordering::SeqCst);
        assert!(
            (3..10).contains(&spawns),
            "N=10 时 3 次失败应仍在重试（实际 {spawns} 次）"
        );
        assert_ne!(handle10.snapshot().await.state, ServiceState::FailedStartup);
        assert!(handle10.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task10).await;
    }

    #[tokio::test]
    async fn ready_resets_the_spawn_failure_counter() {
        // 失败 2 次 → 成功 ready（ready 后 5ms 退出）→ 再失败 2 次：每次成功
        // ready 都清零计数，永远不到上限（D7a）。
        struct FlakyThenReady {
            spawns: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl ServiceSpawner for FlakyThenReady {
            async fn spawn(&self) -> Result<SpawnedInstance> {
                let n = self.spawns.fetch_add(1, Ordering::SeqCst);
                if n % 4 < 2 {
                    // 每轮前两次失败。
                    return Err(SebasError::Upgrade("flaky".into()));
                }
                let (ready_tx, ready_rx) = oneshot::channel();
                let (exit_tx, exited) = tokio::sync::watch::channel(false);
                let sig_tx = exit_tx.clone();
                tokio::spawn(async move {
                    let _ = ready_tx.send(());
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    let _ = sig_tx.send(true);
                });
                Ok(SpawnedInstance {
                    child: Box::new(FakeChild {
                        pid: 7,
                        exited,
                        exit_triggered: exit_tx,
                        exit_code: 1,
                    }),
                    readiness: Some(ready_rx),
                })
            }
        }
        let spawner = Arc::new(FlakyThenReady {
            spawns: AtomicUsize::new(0),
        });
        let mut spec = fast_spec(spawner.clone(), DesiredState::Enabled);
        spec.max_spawn_failures = 3;
        let (handle, task) = start_supervision(spec);
        tokio::time::sleep(Duration::from_millis(150)).await;
        let spawns = spawner.spawns.load(Ordering::SeqCst);
        assert!(spawns >= 6, "ready 后计数重置，应持续重试: {spawns}");
        assert_ne!(
            handle.snapshot().await.state,
            ServiceState::FailedStartup,
            "ready 之间的失败组不得累积成终态"
        );
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn early_fatal_before_ready_counts_toward_the_limit() {
        // 有 readiness 门的 child 每次以非 75 退出且从未 ready → 与 spawn
        // 失败同等待遇，第 3 次进入终态（spec「early-fatal counts」场景）。
        struct EarlyFatal;
        #[async_trait::async_trait]
        impl ServiceSpawner for EarlyFatal {
            async fn spawn(&self) -> Result<SpawnedInstance> {
                let (exit_tx, exited) = tokio::sync::watch::channel(false);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    let _ = exit_tx.send(true);
                });
                Ok(SpawnedInstance {
                    child: Box::new(FakeChild {
                        pid: 9,
                        exited,
                        exit_triggered: tokio::sync::watch::channel(false).0,
                        exit_code: 1, // 非 75
                    }),
                    readiness: Some(oneshot::channel().1), // 发送端即弃，ready 永不到达
                })
            }
        }
        let mut spec = fast_spec(Arc::new(EarlyFatal), DesiredState::Enabled);
        spec.max_spawn_failures = 3;
        let (handle, task) = start_supervision(spec);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while handle.snapshot().await.state != ServiceState::FailedStartup {
            assert!(
                tokio::time::Instant::now() < deadline,
                "early-fatal 退出 3 次必须进入 failed-startup 终态"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let sf = handle
            .snapshot()
            .await
            .startup_failure
            .expect("early-fatal recorded");
        assert!(sf.last_stderr.contains("before ready"), "{sf:?}");
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn post_ready_crash_never_enters_failed_startup() {
        // ready 之后的退出是运行期崩溃：走既有 crash backoff（冷却续跑），
        // 绝不进入 failed-startup。
        let spawner = FakeSpawner::auto(1); // spawn 后 1ms 即退；readiness None → spawn 即确认
        let mut spec = fast_spec(spawner.clone(), DesiredState::Enabled);
        spec.max_spawn_failures = 2; // 刻意很小
        let (handle, task) = start_supervision(spec);
        tokio::time::sleep(Duration::from_millis(120)).await;
        let spawns = spawner.spawns.load(Ordering::SeqCst);
        assert!(spawns > 3, "运行期崩溃持续监督: {spawns}");
        assert_ne!(
            handle.snapshot().await.state,
            ServiceState::FailedStartup,
            "已确认启动的崩溃不得计入 failed-startup"
        );
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn unready_after_upgrade_invokes_hook_without_crash_count() {
        use std::sync::Mutex as StdMutex;
        let hook_calls = Arc::new(StdMutex::new(0u32));
        let calls = hook_calls.clone();
        let spawner = FakeSpawner::auto(5); // 立刻退出（未 ready）
        let mut spec = fast_spec(spawner.clone(), DesiredState::Enabled);
        spec.on_unready_after_upgrade = Some(Arc::new(move || {
            let calls = calls.clone();
            Box::pin(async move {
                *calls.lock().unwrap() += 1;
            })
        }));
        let (handle, task) = start_supervision(spec);
        // 触发一次「升级重启」。
        tokio::time::sleep(Duration::from_millis(10)).await;
        handle
            .send(ServiceCommand::Restart { is_upgrade: true })
            .await;
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(
            *hook_calls.lock().unwrap() >= 1,
            "升级后未 Ready 退出必须触发回滚钩子"
        );
        // 不计 crash：普通重启延迟（5-10ms）下 spawns 应持续增长。
        assert!(spawner.spawns.load(Ordering::SeqCst) >= 2);
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn bind_failed_exit_code_marks_degraded() {
        let spawner = FakeSpawner::bind_failed();
        let (handle, task) = start_supervision(fast_spec(spawner.clone(), DesiredState::Enabled));
        // bind_failed 的 auto_exit_ms=10，等足够时间让 child 退出。
        tokio::time::sleep(Duration::from_millis(60)).await;
        let snap = handle.snapshot().await;
        assert_eq!(
            snap.state,
            ServiceState::Degraded,
            "退出码 75 应标记为 Degraded"
        );
        // 不应自动重试（spawns 保持 1）。
        assert_eq!(
            spawner.spawns.load(Ordering::SeqCst),
            1,
            "Degraded 后不自动重试"
        );
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn core_bind_failed_before_ready_marks_degraded_not_failed_startup() {
        // harden-core-channel-deployment 3.2：core 的通道 bind 失败（ready
        // 之前以 75 退出）复用 bind-failed 同一分支 → Degraded，而不是 fail-fast 终态，
        // 也不进入无限重启。readiness 门 + ServiceName::Core 还原 core 路径。
        let spawner = FakeSpawner::bind_failed();
        struct GatedBindFailed {
            inner: Arc<FakeSpawner>,
        }
        #[async_trait::async_trait]
        impl ServiceSpawner for GatedBindFailed {
            async fn spawn(&self) -> Result<SpawnedInstance> {
                let mut inst = self.inner.spawn().await?;
                // 发送端即弃：ready 永不到达（bind 失败先于 ready）。
                inst.readiness = Some(oneshot::channel().1);
                Ok(inst)
            }
        }
        let spawner: Arc<dyn ServiceSpawner> = Arc::new(GatedBindFailed { inner: spawner });
        let mut spec = fast_spec(spawner, DesiredState::Enabled);
        spec.name = ServiceName::Core;
        let (handle, task) = start_supervision(spec);
        tokio::time::sleep(Duration::from_millis(60)).await;
        let snap = handle.snapshot().await;
        assert_eq!(
            snap.state,
            ServiceState::Degraded,
            "core 在 ready 前以 75 退出应标记 Degraded（而非 failed-startup 终态）"
        );
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn restart_clears_degraded() {
        let spawner = FakeSpawner::bind_failed();
        let (handle, task) = start_supervision(fast_spec(spawner.clone(), DesiredState::Enabled));
        // 等第一次 bind 失败 → Degraded。
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(
            spawner.spawns.load(Ordering::SeqCst),
            1,
            "Degraded 后 spawn 应停止"
        );
        // Restart 命令复位 degraded。
        assert!(handle.send(ServiceCommand::Restart { is_upgrade: false }).await);
        // 等待重新 spawn（bind_failed 的 auto_exit_ms=10，会再次用退出码 75 退出，
        // 但重要的是 spawner 被调用了）。
        tokio::time::sleep(Duration::from_millis(60)).await;
        let snap = handle.snapshot().await;
        // Restart 后再次 bind 失败 → 回到 Degraded。
        // 关键：spawns 增加了（重新 spawn 了）。
        assert!(
            spawner.spawns.load(Ordering::SeqCst) >= 2,
            "Restart 复位后应重新 spawn"
        );
        assert_eq!(snap.state, ServiceState::Degraded);
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }

    #[tokio::test]
    async fn ready_then_exit_counts_as_normal_crash() {
        use std::sync::atomic::AtomicBool;
        let saw_running = Arc::new(AtomicBool::new(false));
        let spawner: Arc<dyn ServiceSpawner> = {
            let spawner = FakeSpawner::auto(50);
            let saw = saw_running.clone();
            // 包一层：spawn 时给 readiness 立即发信号。
            struct ReadyImmediately {
                inner: Arc<FakeSpawner>,
                saw: Arc<AtomicBool>,
            }
            #[async_trait::async_trait]
            impl ServiceSpawner for ReadyImmediately {
                async fn spawn(&self) -> Result<SpawnedInstance> {
                    let mut inst = self.inner.spawn().await?;
                    let (_tx, rx) = oneshot::channel();
                    let tx = _tx;
                    tokio::spawn(async move {
                        let _ = tx.send(());
                    });
                    inst.readiness = Some(rx);
                    let _ = &self.saw;
                    Ok(inst)
                }
            }
            Arc::new(ReadyImmediately {
                inner: spawner,
                saw,
            })
        };
        let (handle, task) = start_supervision(fast_spec(spawner, DesiredState::Enabled));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let snap = handle.snapshot().await;
        if snap.state == ServiceState::Running {
            saw_running.store(true, Ordering::SeqCst);
        }
        assert!(
            saw_running.load(Ordering::SeqCst),
            "readiness 信号到达后状态应为 Running（实际 {:?}）",
            snap.state
        );
        assert!(handle.send(ServiceCommand::Shutdown).await);
        let _ = tokio::time::timeout(Duration::from_millis(200), task).await;
    }
}
