pub mod auth;
pub mod confirmation;
pub mod control;
pub mod control_rpc;
pub mod events;
pub mod executor;
pub mod services;
pub mod updater;

use crate::config::WatchdogConfig;
use crate::error::{Result, SebasError};
use crate::ipc::{ChildMsg, ParentIpc};
use crate::upgrade;
use crate::watchdog::control::{
    Actor, ControlRequest, ControlResponse, ControlService, UpdateKind,
};
use crate::watchdog::executor::{ControlExecutor, PostAction};
use crate::watchdog::updater::{SubprocessUpdaterRunner, UpdatePlan, UpdaterRunner};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

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

fn create_control_secret() -> String {
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{pid:x}-{ts:x}")
}

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
    /// Restart requests produced by in-process control adapters (RPC/WebUI).
    restart_rx: tokio::sync::mpsc::UnboundedReceiver<PostAction>,
    /// Per-watchdog-instance secret accepted by the private control RPC.
    control_secret: String,
    /// True right after a non-dry-run upgrade completes and the watchdog is about
    /// to (re)start the core child with the freshly-installed binary. Lets the
    /// loop classify a crash-before-Ready as `NewBinaryNotReady` and roll back.
    just_performed_update: bool,
    /// Whether the *current* core child reported `ChildMsg::Ready`.
    received_ready: bool,
}

impl Watchdog {
    pub fn new(config: WatchdogConfig, config_path: String) -> Self {
        let control = Arc::new(Mutex::new(ControlService::new()));
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self::with_control(config, config_path, control, rx, create_control_secret())
    }

    pub fn with_control(
        config: WatchdogConfig,
        config_path: String,
        control: Arc<Mutex<ControlService>>,
        restart_rx: tokio::sync::mpsc::UnboundedReceiver<PostAction>,
        control_secret: String,
    ) -> Self {
        Self {
            crash_count: 0,
            last_crash_time: None,
            config_path,
            config,
            control,
            restart_rx,
            control_secret,
            just_performed_update: false,
            received_ready: false,
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
            .env("SEBAS_CONTROL_SECRET", &self.control_secret)
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
            let msg = tokio::select! {
                msg = ipc.recv() => msg,
                action = self.restart_rx.recv() => {
                    match action {
                        // 控制 RPC / WebUI 触发的重启：若是一次升级产生的新二进制，
                        // 标记 `just_performed_update` 供回滚判定。
                        Some(PostAction::RestartCore { is_upgrade }) => {
                            if is_upgrade {
                                info!("control-plane 升级完成，重启以载入新二进制");
                                self.just_performed_update = true;
                            }
                            return Ok(IpcOutcome::RestartRequested);
                        }
                        Some(PostAction::None) => continue,
                        None => return Ok(IpcOutcome::ChildExited),
                    }
                }
            };

            match msg {
                Ok(ChildMsg::Ready) => {
                    info!(
                        "sebas 子进程就绪 (just_performed_update={})",
                        self.just_performed_update
                    );
                    self.received_ready = true;
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
        &mut self,
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
            // 非 rollback 的实升级会安装新二进制，启动下一次 child 前记住这个
            // 事实，供 `run()` 回滚判定用。
            if !rollback {
                info!("{action} 完成，标记 just_performed_update 以便回滚判定");
                self.just_performed_update = true;
            }
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
pub async fn run_watchdog(config: WatchdogConfig, config_path: String, debug: bool) -> Result<()> {
    init_watchdog_tracing();
    let control = Arc::new(Mutex::new(ControlService::new()));
    let (restart_tx, restart_rx) = tokio::sync::mpsc::unbounded_channel();
    let executor = ControlExecutor::new(
        control.clone(),
        Arc::new(SubprocessUpdaterRunner),
        config.clone(),
        config_path.clone(),
        restart_tx,
    );
    let socket_path = control_rpc::default_socket_path();
    let sock_for_rpc = socket_path.clone();
    let executor_for_rpc = executor.clone();
    // Per-watchdog-instance startup secret for private control RPC.
    // No persistence: restarting watchdog invalidates outstanding local clients.
    let secret = create_control_secret();
    let secret_for_rpc = secret.clone();
    tokio::spawn(async move {
        if let Err(e) = control_rpc::serve(sock_for_rpc, secret_for_rpc, executor_for_rpc).await {
            tracing::error!("control RPC server error: {e}");
        }
    });
    info!(
        "watchdog control RPC listening at {}",
        socket_path.display()
    );

    let mut webui_child = spawn_webui_process(&config, &config_path, &secret).await;

    // Debug 模式：额外 spawn 一个独立 `gateway --debug` HTTP 子进程（固定端口、
    // 内置 `test` 模型自应答、不转发上游），与 webui 相同的进程生命周期归 watchdog
    // 所有，方便本地 `curl` 调试。配置沿用同一个 toml，监听地址来自 `[gateway] listen`
    // （默认 127.0.0.1:8787）。
    let mut gateway_child = if debug {
        spawn_debug_gateway_process(&config_path, &secret).await
    } else {
        None
    };

    let mut watchdog = Watchdog::with_control(config, config_path, control, restart_rx, secret);
    let result = watchdog.run().await;
    if let Some(child) = webui_child.as_mut() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    if let Some(child) = gateway_child.as_mut() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    result
}

impl Watchdog {
    async fn run(&mut self) -> Result<()> {
        loop {
            // 新子进程启动，重置 Ready 标记。
            self.received_ready = false;

            let mut child = match self.spawn_child().await {
                Ok(c) => c,
                Err(e) => {
                    // spawn 失败（缺二进制等）：不能退出 watchdog，延迟后重试。
                    error!("spawn_child 失败: {e}，5s 后重试");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
            let Some(stdout) = child.stdout.take() else {
                error!(
                    "子进程 stdout 不可用（pid={}），5s 后重试",
                    child.id().unwrap_or(0)
                );
                let _ = child.kill().await;
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            };
            let Some(stdin) = child.stdin.take() else {
                error!(
                    "子进程 stdin 不可用（pid={}），5s 后重试",
                    child.id().unwrap_or(0)
                );
                let _ = child.kill().await;
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            };
            let pid = child.id().unwrap_or(0);
            let ipc = ParentIpc::new(stdin, stdout);
            match self.handle_ipc(ipc).await? {
                IpcOutcome::RestartRequested => {
                    info!(
                        "重启请求，终止当前 core (pid={}, just_performed_update={})",
                        pid, self.just_performed_update
                    );
                    Self::terminate_child(&mut child).await;
                }
                IpcOutcome::ChildExited => {
                    warn!(
                        "core 子进程意外退出 (pid={}, just_performed_update={})",
                        pid, self.just_performed_update
                    );
                    let _ = child.wait().await;
                }
            }

            // 刚升级完、新二进制从未 Ready 就退出 → 分类为 NewBinaryNotReady，
            // 自动回滚到上一版本。不计入 crash 计数。
            if self.just_performed_update && !self.received_ready {
                warn!("升级后新二进制未就绪即崩溃；自动回滚到上一版本");
                if let Err(e) = self.rollback_to_previous().await {
                    error!("自动回滚失败: {e}（watchdog 保持运行）");
                } else {
                    info!("自动回滚成功，使用上一版本继续运行");
                }
                // 回滚后 watchdog(本进程) 仍跑在旧版本逻辑上，二进制已切回上一版。
                // 清标记，继续用下一个子进程。
                self.just_performed_update = false;
                tokio::time::sleep(Duration::from_millis(RESTART_DELAY_MS)).await;
                continue;
            }
            // 否则（无升级，或新二进制一旦 Ready 过），只按普通 crash 策略重试。
            self.just_performed_update = false;

            if !self.should_restart() {
                // 连续崩溃过多：绝不退出 watchdog，降频重试（保留 webui/gateway）。
                warn!(
                    "连续崩溃过多（{} 次），暂停 30s 后降频重试，watchdog 保持存活",
                    MAX_CRASHES
                );
                tokio::time::sleep(Duration::from_secs(30)).await;
                // 重置计数，避免永久停摆；下次仍走同一正常重试路径。
                self.crash_count = 0;
                continue;
            }

            tokio::time::sleep(Duration::from_millis(RESTART_DELAY_MS)).await;
        }
    }

    /// 有可回滚备份时回滚 `current` 软链到上一版本；无备份 / 失败都返回 Err。
    async fn has_rollback_backup(&self) -> bool {
        let data_dir = crate::upgrade::data_dir(&self.config);
        data_dir.join("rollback").join("sebas").exists()
    }

    /// 自动回滚：加升级锁 → `upgrade::rollback()` → 释放锁。
    async fn rollback_to_previous(&self) -> Result<()> {
        let data_dir = crate::upgrade::data_dir(&self.config);
        if !self.has_rollback_backup().await {
            return Err(SebasError::Upgrade(
                "没有可回滚的版本（rollback/sebas 不存在）".into(),
            ));
        }
        info!("开始回滚，data_dir={}", data_dir.display());
        crate::upgrade::try_lock(&data_dir)?;
        let result = crate::upgrade::rollback(&data_dir);
        crate::upgrade::unlock(&data_dir);
        result?;
        info!("回滚完成，current 已切回上一版本");
        Ok(())
    }
}

async fn spawn_webui_process(
    config: &WatchdogConfig,
    config_path: &str,
    control_secret: &str,
) -> Option<Child> {
    use crate::watchdog::services::should_start_watchdog_webui;

    if !should_start_watchdog_webui(&config.webui) {
        return None;
    }

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            warn!("cannot determine current exe for webui process: {e}");
            return None;
        }
    };

    match Command::new(&exe)
        .arg("webui")
        .arg("--config")
        .arg(config_path)
        .env("SEBAS_CONTROL_SECRET", control_secret)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => {
            info!(
                "webui process spawned (pid={}), dashboard at {}:{}",
                child.id().unwrap_or(0),
                config.webui.host,
                config.webui.port,
            );
            Some(child)
        }
        Err(e) => {
            warn!("failed to spawn webui process: {e}");
            None
        }
    }
}

/// `watchdog --debug`：额外 spawn 一个独立的 `gateway --debug` HTTP 子进程，
/// 生命周期归 watchdog 所有（kill_on_drop + 退出时回收），与 webui 相同模式。
/// 内置 `test` 模型由 gateway 自身应答（`gateway://self` 短路，不转发上游），
/// 监听地址来自 `[gateway] listen`（默认 `127.0.0.1:8787`），方便本地 curl 调试。
async fn spawn_debug_gateway_process(config_path: &str, control_secret: &str) -> Option<Child> {
    // 解析 `[gateway] listen` 以便日志给出实际监听地址；解析失败则回退到
    // 空 `[gateway]` 段的默认值（真正绑定/启动由 gateway 子进程自己负责）。
    let listen = std::fs::read_to_string(config_path)
        .ok()
        .and_then(|raw| gateway::config::GatewayConfig::parse(&raw).ok())
        .map(|cfg| cfg.listen.clone())
        .and_then(|listen| (!listen.is_empty()).then_some(listen))
        .unwrap_or_else(|| {
            gateway::config::GatewayConfig::parse("[gateway]")
                .map(|cfg| cfg.listen)
                .unwrap_or_else(|_| "127.0.0.1:8787".to_string())
        });

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            warn!("cannot determine current exe for debug gateway process: {e}");
            return None;
        }
    };

    match Command::new(&exe)
        .arg("gateway")
        .arg("--config")
        .arg(config_path)
        .arg("--debug")
        .env("SEBAS_CONTROL_SECRET", control_secret)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => {
            info!(
                "debug gateway process spawned (pid={}), HTTP at http://{} (debug test-provider, no upstream)",
                child.id().unwrap_or(0),
                listen,
            );
            Some(child)
        }
        Err(e) => {
            warn!("failed to spawn debug gateway process: {e}");
            None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{WatchdogStorageConfig, WatchdogUpgradeConfig};
    use std::fs;

    fn test_watchdog(tmp_data_dir: &std::path::Path) -> Watchdog {
        let cfg = WatchdogConfig {
            upgrade: WatchdogUpgradeConfig::default(),
            storage: WatchdogStorageConfig {
                data_dir: tmp_data_dir.display().to_string(),
                keep_versions: 1,
            },
            webui: Default::default(),
        };
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Watchdog::with_control(
            cfg,
            "/nonexistent/config.toml".into(),
            Arc::new(Mutex::new(ControlService::new())),
            rx,
            "test-secret".into(),
        )
    }

    #[test]
    fn crash_window_resets_after_max_crashes() {
        let tmp = std::env::temp_dir().join("sebas-wd-crash-test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let mut wd = test_watchdog(&tmp);
        wd.crash_count = 0;
        wd.last_crash_time = None;

        // 3 次 crash 都在窗口内 → 前 3 次允许重启，第 4 次拒绝。
        assert!(wd.should_restart(), "1st should restart");
        assert!(wd.should_restart(), "2nd should restart");
        assert!(wd.should_restart(), "3rd should restart");
        assert!(!wd.should_restart(), "4th must be denied within window");

        // 模拟时间窗口过期（>1h 前），计数应重置。
        wd.last_crash_time =
            Some(std::time::Instant::now() - Duration::from_secs(CRASH_WINDOW_SECS + 1));
        wd.crash_count = MAX_CRASHES;
        assert!(wd.should_restart(), "after window expiry, counter resets");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn has_rollback_backup_reflects_data_dir() {
        let tmp = std::env::temp_dir().join("sebas-wd-backup-test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let wd_none = test_watchdog(&tmp);
        let rt = tokio::runtime::Runtime::new().unwrap();
        // 无 backup → false。
        rt.block_on(async {
            assert!(!wd_none.has_rollback_backup().await);
        });

        fs::create_dir_all(tmp.join("rollback")).unwrap();
        fs::write(tmp.join("rollback").join("sebas"), b"bak").unwrap();
        let wd_has = test_watchdog(&tmp);
        rt.block_on(async {
            assert!(wd_has.has_rollback_backup().await);
        });

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rollback_to_previous_restores_previous_version() {
        let tmp = std::env::temp_dir().join("sebas-wd-rollback-test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // 安装 v1 → v2（v1 成为 rollback 备份），此时 current 指向 v2。
        let v1 = tmp.join("sv1");
        fs::write(&v1, b"v1").unwrap();
        upgrade::install_version(&v1, "1.0.0", &tmp).unwrap();
        let v2 = tmp.join("sv2");
        fs::write(&v2, b"v2").unwrap();
        upgrade::install_version(&v2, "2.0.0", &tmp).unwrap();

        // 有新有旧，应能回滚到上一版本。
        let wd = test_watchdog(&tmp);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            wd.rollback_to_previous()
                .await
                .expect("rollback should succeed");
        });
        let target = fs::read_link(tmp.join("current")).unwrap();
        assert_eq!(target, std::path::Path::new("versions/rollback"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rollback_without_backup_is_err() {
        let tmp = std::env::temp_dir().join("sebas-wd-rollback-empty");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let wd = test_watchdog(&tmp);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(wd.rollback_to_previous());
        assert!(err.is_err(), "no backup should fail rollback");

        let _ = fs::remove_dir_all(&tmp);
    }
}
