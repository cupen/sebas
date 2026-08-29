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
    ProcessChild, ServiceName, ServiceSpawner, ServiceSpec, SpawnedInstance,
};
use crate::watchdog::updater::SubprocessUpdaterRunner;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{info, warn};

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
    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}

// ─── 各服务 spawner ────────────────────────────────────────

/// core 子进程：`current_exe() run --config <path>` + 管道 readiness 握手。
struct CoreSpawner {
    config_path: String,
    control_secret: String,
}

#[async_trait::async_trait]
impl ServiceSpawner for CoreSpawner {
    async fn spawn(&self) -> Result<SpawnedInstance> {
        let exe = std::env::current_exe()
            .map_err(|e| SebasError::Upgrade(format!("无法确定 sebas 子进程路径: {e}")))?;
        info!(
            "启动 sebas core 子进程: {} run --config {}",
            exe.display(),
            self.config_path
        );

        let mut child = Command::new(&exe)
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
}

#[async_trait::async_trait]
impl ServiceSpawner for WebUiSpawner {
    async fn spawn(&self) -> Result<SpawnedInstance> {
        spawn_aux_process(&self.config_path, &self.control_secret, &["webui"], "webui").await
    }
}

/// gateway 子进程：`current_exe() gateway --config <path> [--debug]`。
struct GatewaySpawner {
    config_path: String,
    control_secret: String,
    debug: bool,
}

#[async_trait::async_trait]
impl ServiceSpawner for GatewaySpawner {
    async fn spawn(&self) -> Result<SpawnedInstance> {
        let mut args = vec!["gateway"];
        if self.debug {
            args.push("--debug");
        }
        spawn_aux_process(&self.config_path, &self.control_secret, &args, "gateway").await
    }
}

async fn spawn_aux_process(
    config_path: &str,
    control_secret: &str,
    args: &[&str],
    label: &str,
) -> Result<SpawnedInstance> {
    let exe = std::env::current_exe()
        .map_err(|e| SebasError::Upgrade(format!("无法确定 {label} 子进程路径: {e}")))?;
    let mut cmd = Command::new(&exe);
    for a in args {
        cmd.arg(a);
    }
    cmd.arg("--config")
        .arg(config_path)
        .env("SEBAS_CONTROL_SECRET", control_secret)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true);

    let child = cmd
        .spawn()
        .map_err(|e| SebasError::Upgrade(format!("启动 {label} 子进程失败: {e}")))?;
    info!("{label} 子进程已启动 (pid={})", child.id().unwrap_or(0));
    Ok(SpawnedInstance {
        child: Box::new(ProcessChild(child)),
        readiness: None,
    })
}

/// core 专属：升级后新二进制未就绪时的自动回滚（spec「New-binary
/// auto-rollback」）。失败只记日志，监督循环继续。
async fn rollback_to_previous(config: &WatchdogConfig) -> Result<()> {
    let data_dir = upgrade::data_dir(config);
    if !data_dir.join("rollback").join("sebas").exists() {
        return Err(SebasError::Upgrade(
            "没有可回滚的版本（rollback/sebas 不存在）".into(),
        ));
    }
    info!("开始回滚，data_dir={}", data_dir.display());
    upgrade::try_lock(&data_dir)?;
    let result = upgrade::rollback(&data_dir);
    upgrade::unlock(&data_dir);
    result?;
    info!("回滚完成，current 已切回上一版本");
    Ok(())
}

/// 装配监督 task 用的回滚钩子（捕获 config 副本）。
fn rollback_hook(config: WatchdogConfig) -> super::watchdog::supervisor::UnreadyAfterUpgradeHook {
    Arc::new(move || {
        let cfg = config.clone();
        Box::pin(async move {
            if let Err(e) = rollback_to_previous(&cfg).await {
                warn!("自动回滚失败: {e}（watchdog 保持运行）");
            } else {
                info!("自动回滚成功，使用上一版本继续运行");
            }
        })
    })
}

// ─── 装配 ──────────────────────────────────────────────────

/// 运行 watchdog 模式：ServiceManager + control RPC + 各服务监督 task。
pub async fn run_watchdog(config: WatchdogConfig, config_path: String, debug: bool) -> Result<()> {
    init_watchdog_tracing();
    let dbg = debug;
    tracing::info!(debug_enabled = dbg, "watchdog 启动");
    let control = Arc::new(Mutex::new(ControlService::new()));
    let services = ServiceManager::new(services_persist_path());
    let executor = ControlExecutor::new(
        control.clone(),
        Arc::new(SubprocessUpdaterRunner),
        config.clone(),
        config_path.clone(),
        services.clone(),
    );

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

    // core：readiness 门 + 新二进制未就绪自动回滚。
    // 默认停用（feishu 可选，sebas-2ty）：`sebas watchdog` 默认只启动 WebUI，
    // core（飞书 bot）由 WebUI 服务页启用，或配置 [watchdog.core] enabled = true。
    // persist 文件（services.json）里的选择优先于这里的 config 初值。
    let mut core_spec = ServiceSpec::new(
        ServiceName::Core,
        Arc::new(CoreSpawner {
            config_path: config_path.clone(),
            control_secret: secret.clone(),
        }),
        DesiredState::Enabled,
    );
    core_spec.on_unready_after_upgrade = Some(rollback_hook(config.clone()));
    services.register(core_spec, config.core.enabled);

    // webui：config 开关（默认开）。始终注册进 ServiceManager：即使初值停用，
    // 服务页也能看到并重新启用。
    services.register(
        ServiceSpec::new(
            ServiceName::WebUi,
            Arc::new(WebUiSpawner {
                config_path: config_path.clone(),
                control_secret: secret.clone(),
            }),
            DesiredState::Enabled,
        ),
        config.webui.enabled,
    );

    // gateway：config 开关（默认关）；`--debug` 强制启用 debug 形态
    // （内置 test provider，不转发上游）。同样始终注册以便后续启停。
    services.register(
        ServiceSpec::new(
            ServiceName::Gateway,
            Arc::new(GatewaySpawner {
                config_path: config_path.clone(),
                control_secret: secret.clone(),
                debug,
            }),
            DesiredState::Enabled,
        ),
        config.gateway.enabled || debug,
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
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("watchdog 收到 SIGINT，关闭全部子进程");
        }
        _ = sigterm => {
            tracing::info!("watchdog 收到 SIGTERM，关闭全部子进程");
        }
    }
    services.shutdown_all().await;
    tracing::info!("watchdog 退出");
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
    use crate::config::{WatchdogGatewayConfig, WatchdogStorageConfig, WatchdogUpgradeConfig};
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
            gateway: WatchdogGatewayConfig::default(),
        }
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
