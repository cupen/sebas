use crate::config::WatchdogConfig;
use crate::error::{Result, SebasError};
use crate::upgrade;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

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

        let status = tokio::time::timeout(
            Duration::from_secs(watchdog.upgrade.retry_delay_secs.max(1)),
            cmd.status(),
        )
        .await
        .map_err(|_| SebasError::Upgrade("updater 执行超时".into()))?
        .map_err(|e| SebasError::Upgrade(format!("启动 updater 失败: {e}")))?;

        if !status.success() {
            return Err(SebasError::Upgrade(format!("updater 退出码: {status}")));
        }
        Ok(())
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
