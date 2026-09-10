pub mod auth;
pub mod confirmation;
pub mod control;
pub mod control_rpc;
pub mod events;
pub mod executor;
pub mod services;
pub mod supervisor;
pub mod updater;

use crate::config::WatchdogConfig;
use crate::error::{Result, SebasError};
use crate::ipc::ChildMsg;
use crate::upgrade;
use crate::watchdog::control::{ControlService, DesiredState};
use crate::watchdog::executor::ControlExecutor;
use crate::watchdog::services::ServiceManager;
use crate::watchdog::supervisor::{
    ProcessChild, ServiceName, ServiceSpawner, ServiceSpec, SpawnedInstance, StartupFailureInfo,
    StartupFailureTx,
};
use crate::watchdog::updater::SubprocessUpdaterRunner;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{error, info};

/// 版本号
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// WebUI 子进程 bind 失败时的保留退出码。supervisor 据此区分 bind 失败
/// 与普通 crash，将服务标记为 Degraded 而非自动重试。
pub const EXIT_BIND_FAILED: i32 = 75;

fn create_control_secret() -> String {
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{pid:x}-{ts:x}")
}

/// `~/.sebas/services.json`（期望态 persist 层）。
fn services_persist_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".sebas")
        .join("services.json")
}

/// watchdog 自身的日志初始化。`run_watchdog` 只拿到 `WatchdogConfig`（不含
/// `[log]` 段），沿用旧实现的约定：RUST_LOG 覆盖，默认 info，写 stdout。
/// 漏掉这一步时 watchdog 的所有 info!（socket 监听、子进程 spawn/ready）
/// 会被静默丢弃——表现为「启动后没有任何子进程日志」。
fn init_watchdog_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter =
        EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new(crate::config::DEFAULT_LOG_FILTER));
    let _ = fmt().with_env_filter(filter).try_init();
}

// ─── 各服务 spawner ────────────────────────────────────────

/// core 子进程：`current_exe() run --config <path>` + 管道 readiness 握手。
struct CoreSpawner {
    config_path: String,
    control_secret: String,
    core_secret: String,
}

#[async_trait::async_trait]
impl ServiceSpawner for CoreSpawner {
    async fn spawn(&self) -> Result<SpawnedInstance> {
        let exe = std::env::current_exe()
            .map_err(|e| SebasError::Upgrade(format!("无法确定 sebas 子进程路径: {e}")))?;
        tracing::debug!(
            "starting sebas core child: {} {} --config {}",
            exe.display(),
            crate::CORE_SUBCOMMAND,
            self.config_path
        );

        let mut child = Command::new(&exe)
            .arg(crate::CORE_SUBCOMMAND)
            .arg("--config")
            .arg(&self.config_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .env("SEBAS_IPC", "1")
            .env("SEBAS_CONTROL_SECRET", &self.control_secret)
            .env("SEBAS_CORE_SECRET", &self.core_secret)
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| SebasError::Upgrade(format!("启动子进程失败: {e}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SebasError::Upgrade("core 子进程 stdout 不可用".into()))?;
        // stdin 不再承载命令（Ready-only 协议），drop 无副作用。

        // readiness 监听 + stdout 持续排空。读到 `{"cmd":"ready"}` 发信号后
        // **不能**停止读取：子进程未配 [log] file 时，tracing 也写 stdout，
        // 读端一旦关闭，子进程后续每条日志都会 EPIPE（Broken pipe 刷屏），
        // 且 64KB 管道缓冲写满会把子进程整个卡死。所以读到 EOF 为止，
        // 顺带把非 IPC 行（即子进程日志）转发进 watchdog 日志。
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let mut ready_tx = Some(ready_tx);
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                match serde_json::from_str::<ChildMsg>(&line) {
                    Ok(ChildMsg::Ready) => {
                        if let Some(tx) = ready_tx.take() {
                            let _ = tx.send(());
                        }
                    }
                    Err(_) if line.trim().is_empty() => {}
                    // 非协议行 = 子进程 stdout 日志，转发保可见。
                    Err(_) => {
                        tracing::info!(target: "core", "{line}");
                    }
                }
            }
        });

        Ok(SpawnedInstance {
            child: Box::new(ProcessChild(child)),
            readiness: Some(ready_rx),
        })
    }
}

/// webui 子进程：`current_exe() webui --config <path>`，无 readiness 门。
struct WebUiSpawner {
    config_path: String,
    control_secret: String,
    core_secret: String,
}

#[async_trait::async_trait]
impl ServiceSpawner for WebUiSpawner {
    async fn spawn(&self) -> Result<SpawnedInstance> {
        spawn_aux_process(
            &self.config_path,
            &self.control_secret,
            Some(&self.core_secret),
            None,
            &["webui"],
            "webui",
            &[],
        )
        .await
    }
}

/// im 子进程：`current_exe() im --config <path>`（extract-im-service）。
struct ImSpawner {
    config_path: String,
    control_secret: String,
    core_secret: String,
}

#[async_trait::async_trait]
impl ServiceSpawner for ImSpawner {
    async fn spawn(&self) -> Result<SpawnedInstance> {
        spawn_aux_process(
            &self.config_path,
            &self.control_secret,
            Some(&self.core_secret),
            None,
            &["im"],
            "im",
            &[],
        )
        .await
    }
}

/// router 子进程：`current_exe() router --config <path> [--debug]`。
struct RouterSpawner {
    config_path: String,
    control_secret: String,
    core_secret: String,
    core_socket: String,
    debug: bool,
}

#[async_trait::async_trait]
impl ServiceSpawner for RouterSpawner {
    async fn spawn(&self) -> Result<SpawnedInstance> {
        let mut args = vec!["router"];
        if self.debug {
            args.push("--debug");
        }
        // harden-core-channel-deployment 2.2：router 订阅侧按 D2 从 config
        // 目录发现 secret 文件；`SEBAS_ROUTER_CONFIG` 与 `--config` 同值，
        // 顺带修正 hot-reload 的 config_source 缺省（否则指向 ~/.sebas）。
        spawn_aux_process(
            &self.config_path,
            &self.control_secret,
            Some(&self.core_secret),
            Some(&self.core_socket),
            &args,
            "router",
            &[("SEBAS_ROUTER_CONFIG", self.config_path.as_str())],
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn spawn_aux_process(
    config_path: &str,
    control_secret: &str,
    core_secret: Option<&str>,
    core_socket: Option<&str>,
    args: &[&str],
    label: &str,
    extra_env: &[(&str, &str)],
) -> Result<SpawnedInstance> {
    let exe = std::env::current_exe()
        .map_err(|e| SebasError::Upgrade(format!("无法确定 {label} 子进程路径: {e}")))?;
    let mut cmd = Command::new(&exe);
    for a in args {
        cmd.arg(a);
    }
    cmd.arg("--config")
        .arg(config_path)
        .env("SEBAS_CONTROL_SECRET", control_secret);
    // The standalone WebUI is a core session channel client: it needs the
    // same secret the core gets. The router (5.3 订阅投影) additionally
    // needs the channel path to subscribe to state changes — it discovers
    // the socket exclusively via this env var (core resolves it itself).
    if let Some(core_secret) = core_secret {
        cmd.env("SEBAS_CORE_SECRET", core_secret);
    }
    if let Some(core_socket) = core_socket {
        cmd.env("SEBAS_CORE_SOCKET", core_socket);
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true);

    let child = cmd
        .spawn()
        .map_err(|e| SebasError::Upgrade(format!("启动 {label} 子进程失败: {e}")))?;
    // spawn/exit 由 supervisor 统一记录，这里不再重复打一条。
    Ok(SpawnedInstance {
        child: Box::new(ProcessChild(child)),
        readiness: None,
    })
}

/// core 专属：升级后新二进制未就绪时的自动回滚（spec「New-binary
/// auto-rollback」）。回滚触发写结构化日志（含前/后二进制路径）。
async fn rollback_to_previous(config: &WatchdogConfig) -> Result<()> {
    let data_dir = upgrade::data_dir(config);
    if !data_dir.join("rollback").join("sebas").exists() {
        return Err(SebasError::Upgrade(
            "没有可回滚的版本（rollback/sebas 不存在）".into(),
        ));
    }
    info!(
        "rollback started: current={} backup={}",
        data_dir.join("current").display(),
        data_dir.join("rollback").join("sebas").display()
    );
    upgrade::try_lock(&data_dir)?;
    let result = upgrade::rollback(&data_dir);
    upgrade::unlock(&data_dir);
    result?;
    info!("rollback done, current switched back to previous version");
    Ok(())
}

/// core 专属：升级后新二进制未就绪时的自动回滚（spec「New-binary
/// auto-rollback」，fail-fast-on-startup-errors D4 改写）。回滚失败不再
/// silently continue：记录 `failed-startup` 终态 + 上报事件，watchdog 以
/// EX_TEMPFAIL (75) 退出。回滚触发本身也写结构化日志与时间线事件。
fn rollback_hook(
    config: WatchdogConfig,
    services: ServiceManager,
    control: Arc<Mutex<ControlService>>,
    fail_tx: StartupFailureTx,
) -> super::watchdog::supervisor::UnreadyAfterUpgradeHook {
    use crate::watchdog::supervisor::StartupFailureEvent;
    Arc::new(move || {
        let cfg = config.clone();
        let services = services.clone();
        let control = control.clone();
        let fail_tx = fail_tx.clone();
        Box::pin(async move {
            // 回滚触发事件：前/后二进制路径进时间线，ctl status 可见。
            let data_dir = upgrade::data_dir(&cfg);
            control.lock().await.record_observation(format!(
                "rollback triggered: current -> {} (from {})",
                data_dir.join("rollback").join("sebas").display(),
                data_dir.join("current").display(),
            ));
            match rollback_to_previous(&cfg).await {
                Ok(()) => {
                    info!("auto rollback ok, running previous version");
                    control
                        .lock()
                        .await
                        .record_observation("rollback completed; restarting previous version".to_string());
                }
                Err(e) => {
                    let cause = format!("rollback failed: {e}");
                    error!("{cause}; entering failed-startup, watchdog exits 75");
                    services
                        .record_startup_failure(ServiceName::Core, &cause)
                        .await;
                    let _ = fail_tx.send(Some(StartupFailureEvent {
                        service: ServiceName::Core,
                        info: StartupFailureInfo {
                            count: 1,
                            last_stderr: cause,
                            at_unix: crate::watchdog::supervisor::now_unix_secs(),
                        },
                    }));
                }
            }
        })
    })
}

// ─── 装配 ──────────────────────────────────────────────────

/// 运行 watchdog 模式：ServiceManager + control RPC + 各服务监督 task。
/// fail-fast-on-startup-errors：任一受管服务进入 `failed-startup` 终态
/// （连续 spawn 失败达上限 / rollback 失败）→ shutdown 全部子进程并以
/// EX_TEMPFAIL (75) 退出（经返回 Err → main 统一出口打 startup-failure 摘要）。
pub async fn run_watchdog(
    config: WatchdogConfig,
    config_path: String,
    debug: bool,
    im_enabled_default: bool,
) -> Result<()> {
    init_watchdog_tracing();
    let dbg = debug;
    tracing::info!(debug_enabled = dbg, "watchdog started");
    let control = Arc::new(Mutex::new(ControlService::new()));
    let services = ServiceManager::new(services_persist_path());
    let executor = ControlExecutor::new(
        control.clone(),
        Arc::new(SubprocessUpdaterRunner),
        config.clone(),
        config_path.clone(),
        services.clone(),
    );

    // 终态启动失败事件通道：监督 task / rollback 钩子 → watchdog 主循环。
    let (fail_tx, mut fail_rx) =
        tokio::sync::watch::channel(None::<supervisor::StartupFailureEvent>);

    // 辅助：把上限与失败上报通道接进每个 spec（任务 2.1）。
    let spec_with_fail_fast = |mut spec: ServiceSpec| {
        spec.max_spawn_failures = config.max_spawn_failures.max(1);
        spec.startup_failure_tx = Some(fail_tx.clone());
        spec
    };

    // control RPC（唯一命令面）。
    let socket_path = control_rpc::default_socket_path();
    let sock_for_rpc = socket_path.clone();
    let executor_for_rpc = executor.clone();
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

    // core：恒启动 + readiness 门 + 新二进制未就绪自动回滚。
    // enable-core-by-default：core 是会话核心，无 config 开关、无持久化覆盖
    // ——watchdog 无条件拉起（ServiceManager 对 core 忽略 config/覆盖层）。
    // core session channel 的共享密钥：core 与 webui 子进程各注入一份
    // （SEBAS_CORE_SECRET），socket 之外还叠加同 uid 校验（spec 的双因子）。
    let core_secret = create_control_secret();
    let mut core_spec = spec_with_fail_fast(ServiceSpec::new(
        ServiceName::Core,
        Arc::new(CoreSpawner {
            config_path: config_path.clone(),
            control_secret: secret.clone(),
            core_secret: core_secret.clone(),
        }),
        DesiredState::Enabled,
    ));
    core_spec.on_unready_after_upgrade = Some(rollback_hook(
        config.clone(),
        services.clone(),
        control.clone(),
        fail_tx.clone(),
    ));
    services.register_core(core_spec);

    // webui：config 开关（默认开）。始终注册进 ServiceManager：即使初值停用，
    // 服务页也能看到并重新启用。
    services.register(
        spec_with_fail_fast(ServiceSpec::new(
            ServiceName::WebUi,
            Arc::new(WebUiSpawner {
                config_path: config_path.clone(),
                control_secret: secret.clone(),
                core_secret: core_secret.clone(),
            }),
            DesiredState::Enabled,
        )),
        config.webui.enabled,
    );

    // router：config 开关（默认关）；`--debug` 强制启用 debug 形态
    // （内置 test provider，不转发上游）。同样始终注册以便后续启停。
    // 注入 core channel 的 secret + socket 路径（5.3 订阅投影）：router
    // 订阅状态变更需要与 core 相同的密钥与通道位置——core 自己按同一
    // config 计算路径（channel_path 或缺省），此处直接用同一解析结果。
    let core_channel_path = config
        .core
        .channel_path
        .clone()
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::core_channel::default_socket_path);
    services.register(
        spec_with_fail_fast(ServiceSpec::new(
            ServiceName::Router,
            Arc::new(RouterSpawner {
                config_path: config_path.clone(),
                control_secret: secret.clone(),
                core_secret: core_secret.clone(),
                core_socket: core_channel_path.display().to_string(),
                debug,
            }),
            DesiredState::Enabled,
        )),
        config.router.enabled || debug,
    );

    // im：`[watchdog.im] enabled` 缺省跟随飞书启用判定（feishu-option spec）；
    // 显式给出时以显式值为准。飞书部署由此自动获得 im 服务。
    let im_enabled = config.im.enabled.unwrap_or(im_enabled_default);
    services.register(
        spec_with_fail_fast(ServiceSpec::new(
            ServiceName::Im,
            Arc::new(ImSpawner {
                config_path: config_path.clone(),
                control_secret: secret.clone(),
                core_secret: core_secret.clone(),
            }),
            DesiredState::Enabled,
        )),
        im_enabled,
    );

    // 监督 task 各自永续运行；watchdog 主 task 停泊在关闭信号上。
    // kill_on_drop 只在进程内 Drop 时生效——收到信号时默认动作是立即
    // 终止，Drop 根本不会运行，子进程会被孤儿化；孤儿 core 仍持有飞书
    // WS 长连接，会与新实例竞争事件分发（sebas-a87）。因此必须显式
    // 捕获信号、shutdown_all 之后再退出。
    let sigterm = async {
        #[cfg(unix)]
        {
            let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
            sig.recv().await;
        }
        #[cfg(not(unix))]
        {
            std::future::pending::<()>().await;
        }
    };
    // 终态启动失败等待：watch 通道值变为 Some(event) 即返回。
    let startup_failure = async {
        loop {
            if fail_rx.changed().await.is_err() {
                // 全部发送端已丢弃（不可能：主循环持有 fail_tx 的克隆）；
                // 停泊以免 select 空转。
                std::future::pending::<()>().await;
            }
            if let Some(event) = fail_rx.borrow().clone() {
                break event;
            }
        }
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("watchdog got SIGINT, stopping all children");
        }
        _ = sigterm => {
            tracing::info!("watchdog got SIGTERM, stopping all children");
        }
        event = startup_failure => {
            let cause = format!(
                "managed service '{}' failed to start after {} consecutive startup failures: {}",
                event.service.as_str(),
                event.info.count,
                event.info.last_stderr
            );
            error!("{cause}; shutting down, exiting with 75");
            services.shutdown_all().await;
            return Err(SebasError::Upgrade(cause));
        }
    }
    services.shutdown_all().await;
    tracing::info!("watchdog exited");
    Ok(())
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
    use crate::config::{WatchdogRouterConfig, WatchdogStorageConfig, WatchdogUpgradeConfig};
    use std::fs;

    fn test_config(tmp_data_dir: &std::path::Path) -> WatchdogConfig {
        WatchdogConfig {
            core: Default::default(),
            upgrade: WatchdogUpgradeConfig::default(),
            storage: WatchdogStorageConfig {
                data_dir: tmp_data_dir.display().to_string(),
                keep_versions: 1,
            },
            webui: Default::default(),
            router: WatchdogRouterConfig::default(),
            im: Default::default(),
            max_spawn_failures: crate::watchdog::supervisor::DEFAULT_MAX_SPAWN_FAILURES,
        }
    }

    // 断言 unix symlink 语义（current 软链）；Windows 的 copy 回退语义
    // 不同，先门控到 unix，见 upgrade.rs update_symlink。
    #[cfg(unix)]
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
        let cfg = test_config(&tmp);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            rollback_to_previous(&cfg)
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

        let cfg = test_config(&tmp);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(rollback_to_previous(&cfg));
        assert!(err.is_err(), "no backup should fail rollback");

        let _ = fs::remove_dir_all(&tmp);
    }
}
