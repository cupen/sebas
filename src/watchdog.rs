pub mod control;
pub mod control_rpc;
pub mod updater;

use crate::config::WatchdogConfig;
use crate::error::{Result, SebasError};
use crate::ipc::{ChildMsg, ParentIpc};
use crate::upgrade;
use crate::watchdog::control::{
    Actor, ControlRequest, ControlResponse, ControlService, UpdateKind,
};
use crate::watchdog::updater::{SubprocessUpdaterRunner, UpdatePlan, UpdaterRunner};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// 崩溃计数器重置间隔（秒）
const CRASH_WINDOW_SECS: u64 = 3600;
/// 连续崩溃上限
const MAX_CRASHES: u32 = 3;
/// 崩溃后重启等待时间（毫秒）
const RESTART_DELAY_MS: u64 = 1000;
/// 等待子进程优雅退出时间（秒）
const SHUTDOWN_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IpcOutcome {
    ChildExited,
    RestartRequested,
}

/// 版本号
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Watchdog {
    /// 连续崩溃计数
    crash_count: u32,
    /// 上次崩溃时间
    last_crash_time: Option<std::time::Instant>,
    /// 配置文件路径
    config_path: String,
    /// watchdog 配置
    config: WatchdogConfig,
    /// control-plane state shared by adapters.
    control: Arc<Mutex<ControlService>>,
}

impl Watchdog {
    pub fn new(config: WatchdogConfig, config_path: String) -> Self {
        Self::with_control(
            config,
            config_path,
            Arc::new(Mutex::new(ControlService::new())),
        )
    }

    pub fn with_control(
        config: WatchdogConfig,
        config_path: String,
        control: Arc<Mutex<ControlService>>,
    ) -> Self {
        Self {
            crash_count: 0,
            last_crash_time: None,
            config_path,
            config,
            control,
        }
    }

    async fn spawn_child(&self) -> Result<Child> {
        let exe = std::env::current_exe()
            .map_err(|e| SebasError::Upgrade(format!("无法确定 sebas 子进程路径: {e}")))?;

        info!(
            "启动 sebas 子进程: {} run --config {}",
            exe.display(),
            self.config_path
        );

        let child = Command::new(&exe)
            .arg("run")
            .arg("--config")
            .arg(&self.config_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .env("SEBAS_IPC", "1")
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| SebasError::Upgrade(format!("启动子进程失败: {e}")))?;

        Ok(child)
    }

    async fn terminate_child(child: &mut Child) {
        #[cfg(unix)]
        if let Some(pid) = child.id() {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }

        #[cfg(not(unix))]
        if let Err(e) = child.start_kill() {
            warn!("请求停止子进程失败: {e}");
        }

        match tokio::time::timeout(Duration::from_secs(SHUTDOWN_TIMEOUT_SECS), child.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => warn!("等待子进程退出失败: {e}"),
            Err(_) => {
                warn!("子进程优雅退出超时，强制 kill");
                let _ = child.kill().await;
            }
        }
    }

    async fn handle_ipc(&mut self, mut ipc: ParentIpc) -> Result<IpcOutcome> {
        // 现有 IPC 逻辑先保留，后续 Phase 1/3 再迁到 ControlService/RPC。
        loop {
            match ipc.recv().await {
                Ok(ChildMsg::Ready) => {
                    info!("sebas 子进程就绪");
                    break;
                }
                Ok(ChildMsg::Upgrade { dry_run }) => {
                    info!("收到升级请求 (dry_run: {dry_run})");
                    match self.run_update(false, dry_run, false, &mut ipc).await {
                        Ok(true) => return Ok(IpcOutcome::RestartRequested),
                        Ok(false) => {}
                        Err(e) => {
                            warn!("升级失败: {e}");
                            let _ = ipc.error(&format!("升级失败: {e}")).await;
                        }
                    }
                }
                Ok(ChildMsg::UpgradeDev { dry_run }) => {
                    info!("收到 dev 升级请求 (dry_run: {dry_run})");
                    match self.run_update(true, dry_run, false, &mut ipc).await {
                        Ok(true) => return Ok(IpcOutcome::RestartRequested),
                        Ok(false) => {}
                        Err(e) => {
                            warn!("dev 升级失败: {e}");
                            let _ = ipc.error(&format!("dev 升级失败: {e}")).await;
                        }
                    }
                }
                Ok(ChildMsg::Rollback) => {
                    info!("收到回滚请求");
                    match self.run_update(false, false, true, &mut ipc).await {
                        Ok(true) => return Ok(IpcOutcome::RestartRequested),
                        Ok(false) => {}
                        Err(e) => {
                            warn!("回滚失败: {e}");
                            let _ = ipc.error(&format!("回滚失败: {e}")).await;
                        }
                    }
                }
                Err(e) => {
                    warn!("IPC 连接断开: {e}");
                    break;
                }
            }
        }
        Ok(IpcOutcome::ChildExited)
    }

    async fn run_update(
        &self,
        dev: bool,
        dry_run: bool,
        rollback: bool,
        ipc: &mut ParentIpc,
    ) -> Result<bool> {
        let action = if rollback {
            "回滚"
        } else if dev {
            "dev 升级"
        } else {
            "release 升级"
        };
        let _ = ipc.ok(&format!("正在执行 {action}...")).await;

        let operation_id = {
            let mut control = self.control.lock().await;
            let request = if rollback {
                ControlRequest::Rollback { dry_run }
            } else {
                ControlRequest::Update {
                    kind: if dev {
                        UpdateKind::Dev
                    } else {
                        UpdateKind::Release
                    },
                    dry_run,
                    target: None,
                }
            };
            match control.accept(Actor::System, request) {
                ControlResponse::Accepted { operation_id, .. } => {
                    control.mark_running(&operation_id, format!("running {action}"));
                    operation_id
                }
                ControlResponse::Rejected { message, .. } => {
                    return Err(SebasError::Upgrade(message));
                }
            }
        };

        let runner = SubprocessUpdaterRunner;
        let result = runner
            .run(
                &UpdatePlan {
                    config_path: self.config_path.clone(),
                    dev,
                    dry_run,
                    rollback,
                    project_dir: None,
                },
                &self.config,
            )
            .await;

        if let Err(error) = result {
            self.control
                .lock()
                .await
                .mark_error(&operation_id, format!("{action} failed: {error}"));
            return Err(error);
        }

        if dry_run {
            self.control
                .lock()
                .await
                .mark_done(&operation_id, format!("{action} dry-run completed"));
            let _ = ipc.done(&format!("{action} dry-run 完成，无需重启")).await;
            Ok(false)
        } else {
            self.control
                .lock()
                .await
                .mark_done(&operation_id, format!("{action} completed"));
            let _ = ipc.done(&format!("{action} 完成，准备重启")).await;
            Ok(true)
        }
    }

    fn should_restart(&mut self) -> bool {
        let now = std::time::Instant::now();

        if let Some(last) = self.last_crash_time {
            if now.duration_since(last) > Duration::from_secs(CRASH_WINDOW_SECS) {
                self.crash_count = 0;
            }
        }

        self.crash_count += 1;
        self.last_crash_time = Some(now);

        if self.crash_count > MAX_CRASHES {
            false
        } else {
            true
        }
    }
}

/// 运行 watchdog 模式
pub async fn run_watchdog(config: WatchdogConfig, config_path: String) -> Result<()> {
    init_watchdog_tracing();
    let control = Arc::new(Mutex::new(ControlService::new()));
    let socket_path = control_rpc::default_socket_path();
    tokio::spawn(control_rpc::serve(socket_path.clone(), control.clone()));
    info!(
        "watchdog control RPC listening at {}",
        socket_path.display()
    );

    let mut watchdog = Watchdog::with_control(config, config_path, control);
    watchdog.run().await
}

impl Watchdog {
    async fn run(&mut self) -> Result<()> {
        loop {
            let mut child = self.spawn_child().await?;
            let Some(stdout) = child.stdout.take() else {
                return Err(SebasError::Upgrade("子进程 stdout 不可用".into()));
            };
            let Some(stdin) = child.stdin.take() else {
                return Err(SebasError::Upgrade("子进程 stdin 不可用".into()));
            };
            let ipc = ParentIpc::new(stdin, stdout);
            match self.handle_ipc(ipc).await? {
                IpcOutcome::RestartRequested => {
                    Self::terminate_child(&mut child).await;
                }
                IpcOutcome::ChildExited => {
                    let _ = child.wait().await;
                }
            }

            if !self.should_restart() {
                return Err(SebasError::Upgrade("子进程连续崩溃过多，停止重启".into()));
            }

            tokio::time::sleep(Duration::from_millis(RESTART_DELAY_MS)).await;
        }
    }
}

/// 初始化 watchdog 的 tracing subscriber
fn init_watchdog_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}

/// 输出当前版本信息
pub fn print_version() {
    println!("sebas watchdog v{}", VERSION);
    println!("git: {}", upgrade::current_version());
    println!(
        "binary: {}",
        std::env::current_exe().unwrap_or_default().display()
    );
}
