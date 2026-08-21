use crate::config::WatchdogConfig;
use crate::error::{Result, SebasError};
use crate::upgrade;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// updater 超时后 SIGTERM 到 SIGKILL 之间的宽限期（秒）。
const UPDATER_KILL_GRACE_SECS: u64 = 5;

#[derive(Debug, Clone)]
pub struct UpdatePlan {
    pub config_path: String,
    pub dev: bool,
    pub dry_run: bool,
    pub rollback: bool,
    pub project_dir: Option<PathBuf>,
}

#[async_trait::async_trait]
pub trait UpdaterRunner: Send + Sync {
    async fn run(&self, plan: &UpdatePlan, watchdog: &WatchdogConfig) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct SubprocessUpdaterRunner;

#[async_trait::async_trait]
impl UpdaterRunner for SubprocessUpdaterRunner {
    async fn run(&self, plan: &UpdatePlan, watchdog: &WatchdogConfig) -> Result<()> {
        let exe = std::env::current_exe()
            .map_err(|e| SebasError::Upgrade(format!("获取 updater 路径失败: {e}")))?;
        let mut cmd = Command::new(exe);
        cmd.arg("update").arg("--config").arg(&plan.config_path);
        if plan.dev {
            cmd.arg("--dev");
        }
        if plan.dry_run {
            cmd.arg("--dry-run");
        }
        if plan.rollback {
            cmd.arg("--rollback");
        }
        if let Some(project_dir) = &plan.project_dir {
            cmd.arg("--project-dir").arg(project_dir);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        // dev 走 cargo 编译，耗时可达数分钟；release 只下载安装。两者用不同上限，
        // 绝不能复用 retry_delay_secs（那是重试间隔，5s 会让 dev 编译必然被误杀）。
        let timeout = watchdog.upgrade.updater_timeout(plan.dev);

        let mut child = cmd
            .spawn()
            .map_err(|e| SebasError::Upgrade(format!("启动 updater 失败: {e}")))?;

        let status = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(result) => {
                result.map_err(|e| SebasError::Upgrade(format!("等待 updater 失败: {e}")))?
            }
            Err(_) => {
                // 超时：先 SIGTERM 给机会清理（updater 可能持有 upgrade.lock），
                // 宽限期后再 SIGKILL。
                terminate_then_kill(&mut child).await;
                return Err(SebasError::Upgrade(format!(
                    "updater 执行超时（{}s）",
                    timeout.as_secs()
                )));
            }
        };

        if !status.success() {
            return Err(SebasError::Upgrade(format!("updater 退出码: {status}")));
        }
        Ok(())
    }
}

/// 超时后的两阶段停止：SIGTERM → 宽限 → SIGKILL。
/// updater 可能持有 `upgrade.lock`，直接 SIGKILL 会留下陈旧锁文件。
async fn terminate_then_kill(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    let _ = child.start_kill();

    match tokio::time::timeout(Duration::from_secs(UPDATER_KILL_GRACE_SECS), child.wait()).await {
        Ok(_) => {}
        Err(_) => {
            let _ = child.kill().await;
        }
    }
}

pub async fn run_one_shot(plan: UpdatePlan) -> Result<()> {
    let raw = std::fs::read_to_string(&plan.config_path).unwrap_or_default();
    let cfg = crate::config::Config::parse(&raw)?;
    run_one_shot_with_config(plan, &cfg.watchdog).await
}

pub async fn run_one_shot_with_config(plan: UpdatePlan, watchdog: &WatchdogConfig) -> Result<()> {
    let data_dir = upgrade::data_dir(watchdog);
    if plan.rollback {
        if plan.dry_run {
            println!("would rollback using data_dir={}", data_dir.display());
            return Ok(());
        }
        upgrade::try_lock(&data_dir)?;
        let result = upgrade::rollback(&data_dir);
        upgrade::unlock(&data_dir);
        result?;
        println!("rollback installed; restart required");
        return Ok(());
    }

    upgrade::try_lock(&data_dir)?;
    let result = if plan.dev {
        update_dev(&data_dir, plan.project_dir.as_ref(), plan.dry_run).await
    } else {
        update_release(watchdog, &data_dir, plan.dry_run).await
    };
    upgrade::unlock(&data_dir);
    result
}

async fn update_release(watchdog: &WatchdogConfig, data_dir: &Path, dry_run: bool) -> Result<()> {
    let repo = &watchdog.upgrade.github_repo;
    let current = upgrade::current_version_raw();
    println!("checking latest release from {repo} (current {current})");
    let Some(release) = upgrade::check_latest(repo, &current).await? else {
        println!("already up to date");
        return Ok(());
    };

    println!("latest release: {}", release.version);
    if dry_run {
        println!("would download {}", release.download_url);
        return Ok(());
    }

    let tmp_dir = data_dir.join("downloads");
    let tmp = tmp_dir.join(format!("sebas-{}", release.version));
    upgrade::download_release(&release, &tmp, &tmp_dir).await?;
    upgrade::install_version(&tmp, &release.version, data_dir)?;
    let _ = std::fs::remove_file(&tmp);
    println!("installed {}; restart required", release.version);
    Ok(())
}

async fn update_dev(data_dir: &Path, project_dir: Option<&PathBuf>, dry_run: bool) -> Result<()> {
    let project_dir = project_dir.cloned().unwrap_or(
        std::env::current_dir()
            .map_err(|e| SebasError::Upgrade(format!("获取当前目录失败: {e}")))?,
    );
    println!("building dev version from {}", project_dir.display());
    if dry_run {
        let cargo_toml = project_dir.join("Cargo.toml");
        if !cargo_toml.exists() {
            return Err(SebasError::Upgrade(format!(
                "不是 Rust 项目目录: {}",
                project_dir.display()
            )));
        }
        println!("would run cargo build --release");
        return Ok(());
    }

    let binary = upgrade::compile_dev(&project_dir).await?;
    let version = format!("dev-{}", upgrade::current_version_raw());
    upgrade::install_version(&binary, &version, data_dir)?;
    println!("installed {version}; restart required");
    Ok(())
}

// ─── Version / Readiness Policy (spec §14) ────────────────

/// Signal emitted after a completed update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateSignal {
    /// Core restart is sufficient.
    RestartCore,
    /// The update affects watchdog/control-plane semantics; the watchdog
    /// process itself should be restarted (e.g. via systemd).
    WatchdogServiceRestartRequired,
}

/// Human-readable message for an [`UpdateSignal`].
pub fn update_signal_message(signal: UpdateSignal) -> &'static str {
    match signal {
        UpdateSignal::RestartCore => "update completed; restarting core",
        UpdateSignal::WatchdogServiceRestartRequired => {
            "update completed; watchdog service restart required"
        }
    }
}

/// Whether an update touches watchdog/control-plane code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneImpact {
    CoreOnly,
    AffectsControlPlane,
}

/// Compute the appropriate [`UpdateSignal`] based on the update's impact.
pub fn classify_update_impact(impact: ControlPlaneImpact) -> UpdateSignal {
    match impact {
        ControlPlaneImpact::CoreOnly => UpdateSignal::RestartCore,
        ControlPlaneImpact::AffectsControlPlane => UpdateSignal::WatchdogServiceRestartRequired,
    }
}

/// Outcome when a core child exits without reporting readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessFailureAction {
    /// Normal crash: apply crash-count policy.
    CrashRetry,
    /// The child was a new binary after an update and failed to become ready.
    /// Rollback or manual intervention is required.
    NewBinaryNotReady,
}

/// Classify a core child exit based on whether an update was just performed
/// and whether the child ever reported readiness.
pub fn classify_readiness_failure(
    just_performed_update: bool,
    received_ready: bool,
    child_exited: bool,
) -> ReadinessFailureAction {
    if just_performed_update && !received_ready && child_exited {
        ReadinessFailureAction::NewBinaryNotReady
    } else {
        ReadinessFailureAction::CrashRetry
    }
}

/// Recommended recovery action when an update cannot be completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    Rollback,
    ManualIntervention,
}

pub fn recommended_recovery(has_rollback_backup: bool) -> RecoveryAction {
    if has_rollback_backup {
        RecoveryAction::Rollback
    } else {
        RecoveryAction::ManualIntervention
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WatchdogUpgradeConfig;

    #[test]
    fn dev_and_release_get_distinct_timeouts() {
        let cfg = WatchdogUpgradeConfig::default();
        let dev = cfg.updater_timeout(true);
        let release = cfg.updater_timeout(false);

        assert_ne!(dev, release, "dev 与 release 必须使用不同超时");
        assert!(
            dev > release,
            "dev 需要编译，超时应大于 release: dev={dev:?} release={release:?}"
        );
    }

    #[test]
    fn dev_timeout_allows_a_full_release_build() {
        // 本仓库 `cargo build --release` 实测约 95s。默认 dev 超时必须留足余量，
        // 否则 /upgrade dev 会被误判超时（这正是该 bug 的原始症状）。
        let cfg = WatchdogUpgradeConfig::default();
        assert!(
            cfg.updater_timeout(true) >= Duration::from_secs(600),
            "dev 编译超时过短，会误杀正常编译"
        );
    }

    #[test]
    fn retry_delay_is_not_used_as_timeout() {
        // retry_delay_secs 语义是重试间隔。即便它被设成极小值，
        // updater 超时也不能跟着变小。
        let cfg = WatchdogUpgradeConfig {
            retry_delay_secs: 1,
            ..WatchdogUpgradeConfig::default()
        };
        assert!(cfg.updater_timeout(true) > Duration::from_secs(1));
        assert!(cfg.updater_timeout(false) > Duration::from_secs(1));
    }

    #[test]
    fn timeout_config_is_overridable_and_never_zero() {
        let cfg = WatchdogUpgradeConfig {
            updater_timeout_secs: 42,
            dev_build_timeout_secs: 99,
            ..WatchdogUpgradeConfig::default()
        };
        assert_eq!(cfg.updater_timeout(false), Duration::from_secs(42));
        assert_eq!(cfg.updater_timeout(true), Duration::from_secs(99));

        // 0 会让 timeout 立即触发，必须被抬到至少 1s。
        let zero = WatchdogUpgradeConfig {
            updater_timeout_secs: 0,
            dev_build_timeout_secs: 0,
            ..WatchdogUpgradeConfig::default()
        };
        assert_eq!(zero.updater_timeout(false), Duration::from_secs(1));
        assert_eq!(zero.updater_timeout(true), Duration::from_secs(1));
    }

    #[test]
    fn timeouts_parse_from_toml() {
        let raw = r#"
[feishu]
app_id = "a"
app_secret = "b"

[watchdog.upgrade]
updater_timeout_secs = 123
dev_build_timeout_secs = 4567
"#;
        let cfg = crate::config::Config::parse(raw).expect("config must parse");
        assert_eq!(cfg.watchdog.upgrade.updater_timeout_secs, 123);
        assert_eq!(cfg.watchdog.upgrade.dev_build_timeout_secs, 4567);
        assert_eq!(
            cfg.watchdog.upgrade.updater_timeout(true),
            Duration::from_secs(4567)
        );
    }

    #[test]
    fn core_only_update_does_not_require_watchdog_restart() {
        assert_eq!(
            classify_update_impact(ControlPlaneImpact::CoreOnly),
            UpdateSignal::RestartCore
        );
    }

    #[test]
    fn control_plane_update_requires_watchdog_restart() {
        assert_eq!(
            classify_update_impact(ControlPlaneImpact::AffectsControlPlane),
            UpdateSignal::WatchdogServiceRestartRequired
        );
    }

    #[test]
    fn update_signal_messages_are_distinct() {
        let core_msg = update_signal_message(UpdateSignal::RestartCore);
        let cp_msg = update_signal_message(UpdateSignal::WatchdogServiceRestartRequired);
        assert_ne!(core_msg, cp_msg);
        assert!(cp_msg.contains("watchdog service restart required"));
    }

    #[test]
    fn normal_child_exit_is_crash_retry() {
        assert_eq!(
            classify_readiness_failure(false, true, true),
            ReadinessFailureAction::CrashRetry
        );
        assert_eq!(
            classify_readiness_failure(false, false, true),
            ReadinessFailureAction::CrashRetry
        );
    }

    #[test]
    fn new_binary_not_ready_is_detected() {
        assert_eq!(
            classify_readiness_failure(true, false, true),
            ReadinessFailureAction::NewBinaryNotReady
        );
    }

    #[test]
    fn ready_child_exit_not_readiness_failure() {
        assert_eq!(
            classify_readiness_failure(true, true, true),
            ReadinessFailureAction::CrashRetry
        );
    }

    #[test]
    fn recovery_with_backup_recommends_rollback() {
        assert_eq!(recommended_recovery(true), RecoveryAction::Rollback);
    }

    #[test]
    fn recovery_without_backup_recommends_manual_intervention() {
        assert_eq!(
            recommended_recovery(false),
            RecoveryAction::ManualIntervention
        );
    }
}
