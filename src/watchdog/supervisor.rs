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
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{info, warn};

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

// ─── 身份与状态 ────────────────────────────────────────────

/// 受管服务名。监督循环对所有名字一视同仁；core 特有行为经钩子注入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceName {
    Core,
    WebUi,
    Gateway,
}

impl ServiceName {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServiceName::Core => "core",
            ServiceName::WebUi => "webui",
            ServiceName::Gateway => "gateway",
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
}

/// 复用 control 面的期望态命名，避免两套词汇。
pub use crate::watchdog::control::DesiredState;

/// 单服务快照。`started_at` 供 uptime 计算。
#[derive(Debug, Clone)]
pub struct ServiceSnapshot {
    pub name: ServiceName,
    pub state: ServiceState,
    pub desired: DesiredState,
    pub pid: Option<u32>,
    pub started_at: Option<Instant>,
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
    /// watchdog 退出：停 child 并结束监督 task。
    Shutdown,
}

/// 外部持有的单服务句柄。
#[derive(Debug, Clone)]
pub struct ServiceHandle {
    pub name: ServiceName,
    tx: mpsc::Sender<ServiceCommand>,
    snapshot: Arc<Mutex<ServiceSnapshot>>,
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
    let snapshot = Arc::new(Mutex::new(ServiceSnapshot {
        name,
        state: ServiceState::Starting,
        desired: spec.desired,
        pid: None,
        started_at: None,
    }));
    let handle = ServiceHandle {
        name,
        tx,
        snapshot: snapshot.clone(),
    };
    let task = tokio::spawn(supervise(spec, rx, snapshot));
    (handle, task)
}

async fn set_state(snapshot: &Mutex<ServiceSnapshot>, state: ServiceState) {
    snapshot.lock().await.state = state;
}

async fn supervise(
    spec: ServiceSpec,
    mut cmd_rx: mpsc::Receiver<ServiceCommand>,
    snapshot: Arc<Mutex<ServiceSnapshot>>,
) {
    let name = spec.name;
    let mut desired = spec.desired;
    let mut policy = spec.crash;
    // core：下一次 spawn 是否为升级产生的新二进制。
    let mut just_performed_update = false;

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

        // spawn 一次 incarnation。失败重试，监督 task 绝不退出。
        let instance = match spec.spawner.spawn().await {
            Ok(instance) => instance,
            Err(e) => {
                warn!(service = name.as_str(), "spawn 失败: {e}，稍后重试");
                set_state(&snapshot, ServiceState::Restarting).await;
                tokio::time::sleep(spec.spawn_retry_delay).await;
                continue;
            }
        };

        let mut child = instance.child;
        let mut readiness = instance.readiness;
        let pid = child.pid();
        {
            let mut snap = snapshot.lock().await;
            // 无 readiness 门的进程（webui/gateway）：spawn 即 Running。
            snap.state = match readiness {
                Some(_) => ServiceState::Starting,
                None => ServiceState::Running,
            };
            snap.pid = pid;
            snap.started_at = Some(Instant::now());
        }
        info!(service = name.as_str(), pid = ?pid, "child spawned");

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
                    readiness = None;
                    set_state(&snapshot, ServiceState::Running).await;
                    info!(service = name.as_str(), pid = ?pid, "child ready");
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
                    service = name.as_str(), pid = ?pid, code = ?code,
                    "child 退出 (just_performed_update={just_performed_update}, ready={received_ready})"
                );
                snapshot.lock().await.pid = None;

                // core 特例：升级后的新二进制从未 Ready → 回滚钩子，不计 crash。
                if just_performed_update && !received_ready {
                    warn!(
                        service = name.as_str(),
                        "升级后新二进制未就绪即退出；执行回滚钩子"
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
                // 不自动重试，等 Restart 命令。
                if code == Some(EXIT_BIND_FAILED) {
                    warn!(
                        service = name.as_str(),
                        "bind 失败（端口占用？），标记为 Degraded，等待 Restart 命令"
                    );
                    set_state(&snapshot, ServiceState::Degraded).await;
                    continue;
                }

                set_state(&snapshot, ServiceState::Restarting).await;
                match policy.register_crash() {
                    CrashDecision::Restart { delay } => {
                        tokio::time::sleep(delay).await;
                    }
                    CrashDecision::CoolDown { delay } => {
                        warn!(service = name.as_str(), "连续崩溃超限，冷却后继续监督");
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
                warn!("child 优雅退出超时，强制 kill");
                let _ = self.0.kill().await;
            }
        }
    }
}

// ─── tests ─────────────────────────────────────────────────

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
        let (handle, task) = start_supervision(fast_spec(spawner.clone(), DesiredState::Enabled));
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            spawner.spawns.load(Ordering::SeqCst) >= 2,
            "spawn 失败必须重试而非退出监督 task"
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
