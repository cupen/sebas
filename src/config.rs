use crate::error::{Result, SebasError};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub feishu: FeishuConfig,
    #[serde(default)]
    pub acp: AcpConfig,
    #[serde(default)]
    pub router: RouterConfig,
    #[serde(default)]
    pub card: feishu::cards::CardConfig,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub watchdog: WatchdogConfig,
}

/// Wrapper for all ACP agent configs. TOML section `[acp.<agent>]` nests here.
/// Currently only `claude` is supported; future agents add new fields.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AcpConfig {
    #[serde(default)]
    pub claude: AcpClaudeConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeishuConfig {
    // serde-default（空串）让 TOML 缺字段时解析不报错，把「必填」判定留给
    // validate() —— 这样 env 覆盖（SEBAS_FEISHU_APP_ID/SECRET）才有机会
    // 在 validate 前补齐字段（spec §6.3 env > TOML）。
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub app_secret: String,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default = "default_chat_types")]
    pub allowed_chat_types: Vec<String>,
    /// Optional message sent to each restored chat on daemon startup.
    /// Empty (default) = nothing sent.
    #[serde(default)]
    pub hello_msg: String,
    /// 机器人名称（用于群聊 @ 检测）。群聊中只有 @ 了机器人的消息才会被处理。
    /// 空字符串 = 不检查 @（处理所有消息）。
    #[serde(default)]
    pub bot_name: String,
}

impl Default for FeishuConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_secret: String::new(),
            owner_id: String::new(),
            allowed_chat_types: default_chat_types(),
            hello_msg: String::new(),
            bot_name: String::new(),
        }
    }
}

fn default_chat_types() -> Vec<String> {
    vec!["private".into(), "group".into()]
}

#[derive(Debug, Clone, Deserialize)]
pub struct AcpClaudeConfig {
    #[serde(default = "default_claude_path")]
    pub path: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_sessions_dir")]
    pub sessions_dir: String,
    #[serde(default)]
    pub work_dir: Option<String>,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_secs: u64,
    #[serde(default = "default_idle_kill")]
    pub idle_kill_secs: u64,
}

impl Default for AcpClaudeConfig {
    fn default() -> Self {
        Self {
            path: default_claude_path(),
            args: vec![],
            sessions_dir: default_sessions_dir(),
            work_dir: None,
            startup_timeout_secs: default_startup_timeout(),
            idle_kill_secs: default_idle_kill(),
        }
    }
}

fn default_claude_path() -> String {
    "claude".into()
}
fn default_sessions_dir() -> String {
    "~/.claude/sessions".into()
}
fn default_startup_timeout() -> u64 {
    30
}
fn default_idle_kill() -> u64 {
    172800
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouterConfig {
    #[serde(default = "default_state_file")]
    pub state_file: String,
    #[serde(default = "default_channel_buffer")]
    pub channel_buffer: usize,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_sessions: usize,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            state_file: default_state_file(),
            channel_buffer: default_channel_buffer(),
            max_concurrent_sessions: default_max_concurrent(),
        }
    }
}

fn default_state_file() -> String {
    "~/.config/sebas/sessions.json".into()
}
fn default_channel_buffer() -> usize {
    256
}
fn default_max_concurrent() -> usize {
    32
}

#[derive(Debug, Clone, Deserialize)]
pub struct MediaConfig {
    #[serde(default = "default_download_dir")]
    pub download_dir: String,
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir(),
            max_file_size: default_max_file_size(),
        }
    }
}

fn default_download_dir() -> String {
    "~/.cache/sebas/downloads".into()
}
fn default_max_file_size() -> u64 {
    52_428_800
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub file: Option<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            file: None,
        }
    }
}

fn default_log_level() -> String {
    "info".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchdogConfig {
    #[serde(default)]
    pub upgrade: WatchdogUpgradeConfig,
    #[serde(default)]
    pub storage: WatchdogStorageConfig,
    #[serde(default)]
    pub webui: WatchdogWebUiConfig,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            upgrade: WatchdogUpgradeConfig::default(),
            storage: WatchdogStorageConfig::default(),
            webui: WatchdogWebUiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WatchdogWebUiConfig {
    /// Let watchdog own the WebUI lifecycle.
    #[serde(default)]
    pub enabled: bool,
    /// Bind address. Phase 2.3 tightens non-loopback security.
    #[serde(default = "default_webui_host")]
    pub host: String,
    /// Bind port for watchdog-owned WebUI.
    #[serde(default = "default_webui_port")]
    pub port: u16,
}

impl Default for WatchdogWebUiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_webui_host(),
            port: default_webui_port(),
        }
    }
}

fn default_webui_host() -> String {
    "127.0.0.1".into()
}

fn default_webui_port() -> u16 {
    9797
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchdogUpgradeConfig {
    /// GitHub 仓库（owner/repo）
    #[serde(default = "default_github_repo")]
    pub github_repo: String,
    /// 启动时自动检查更新
    #[serde(default = "default_true")]
    pub check_on_start: bool,
    /// 最大重试次数
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// 重试间隔（秒）。仅用于失败重试的等待，**不是**子进程超时。
    #[serde(default = "default_retry_delay")]
    pub retry_delay_secs: u64,
    /// release 更新（下载 + 校验 + 安装）的 updater 子进程超时（秒）。
    #[serde(default = "default_updater_timeout")]
    pub updater_timeout_secs: u64,
    /// dev 更新（cargo build --release）的 updater 子进程超时（秒）。
    /// 编译整个 workspace 可能耗时数分钟，故默认远大于 release 路径。
    #[serde(default = "default_dev_build_timeout")]
    pub dev_build_timeout_secs: u64,
}

impl Default for WatchdogUpgradeConfig {
    fn default() -> Self {
        Self {
            github_repo: default_github_repo(),
            check_on_start: default_true(),
            max_retries: default_max_retries(),
            retry_delay_secs: default_retry_delay(),
            updater_timeout_secs: default_updater_timeout(),
            dev_build_timeout_secs: default_dev_build_timeout(),
        }
    }
}

impl WatchdogUpgradeConfig {
    /// updater 子进程超时：dev 走编译路径，给足编译时间；release 只下载安装。
    pub fn updater_timeout(&self, dev: bool) -> std::time::Duration {
        let secs = if dev {
            self.dev_build_timeout_secs
        } else {
            self.updater_timeout_secs
        };
        std::time::Duration::from_secs(secs.max(1))
    }
}

fn default_github_repo() -> String {
    "cupen/sebas".into()
}
fn default_true() -> bool {
    true
}
fn default_max_retries() -> u32 {
    3
}
fn default_retry_delay() -> u64 {
    5
}
fn default_updater_timeout() -> u64 {
    600
}
fn default_dev_build_timeout() -> u64 {
    1800
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchdogStorageConfig {
    /// 数据存放目录（二进制、版本备份）
    #[serde(default)]
    pub data_dir: String,
    /// 保留的旧版本数量
    #[serde(default = "default_keep_versions")]
    pub keep_versions: u32,
}

impl Default for WatchdogStorageConfig {
    fn default() -> Self {
        Self {
            data_dir: String::new(),
            keep_versions: default_keep_versions(),
        }
    }
}

fn default_keep_versions() -> u32 {
    1
}

impl Config {
    /// Parse TOML, apply env overrides, validate required fields, expand
    /// `~` paths. Priority per spec §6.3: CLI flags > env vars > TOML >
    /// defaults (CLI flags are applied by the caller before/after this).
    pub fn parse(s: &str) -> Result<Self> {
        let mut cfg: Config =
            toml::from_str(s).map_err(|e| SebasError::Config(format!("toml parse: {e}")))?;
        cfg.apply_env_overrides();
        cfg.validate()?;
        Ok(cfg.with_expanded_paths())
    }

    /// env vars override TOML for the sensitive/ops fields (spec §6.3).
    /// Empty values are ignored so `SEBAS_X=` never blanks a configured
    /// credential. Runs BEFORE `validate` so env can satisfy required
    /// fields on a host without a config file.
    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("SEBAS_FEISHU_APP_ID")
            && !v.is_empty()
        {
            self.feishu.app_id = v;
        }
        if let Ok(v) = std::env::var("SEBAS_FEISHU_APP_SECRET")
            && !v.is_empty()
        {
            self.feishu.app_secret = v;
        }
        if let Ok(v) = std::env::var("SEBAS_LOG_LEVEL")
            && !v.is_empty()
        {
            self.log.level = v;
        }
    }

    fn validate(&self) -> Result<()> {
        if self.feishu.app_id.is_empty() {
            return Err(SebasError::Config("feishu.app_id is required".into()));
        }
        if self.feishu.app_secret.is_empty() {
            return Err(SebasError::Config("feishu.app_secret is required".into()));
        }
        // owner_id 决策（sebas-nya）：维持**可选**，偏离 spec §6.1 的必填。
        // 依据：spec §6 同时写明「只有 3 个必填字段」只是设计原则，而实际
        // 部署（config/config.toml）以 owner_id = "" 运行单用户机器人；
        // 空值语义 = 跳过 owner 过滤。风险（任何飞书用户都可驱动 bot）在
        // run::run 启动时以 warn 提示，并在 config.toml.example 文档化。
        Ok(())
    }

    /// Environmental startup checks (spec §6.4) that need a real
    /// filesystem and PATH — kept OUT of `parse` so unit tests can
    /// validate pure config on hosts without a claude binary. `run::run`
    /// calls this before touching the network or spawning anything.
    ///
    /// 1. 目录可写性：state_file 父目录、media.download_dir、log.file 父
    ///    目录（缺失则创建；创建/探测失败 → 友好 Config 错误，不 panic）。
    /// 2. ACP 子进程二进制可达：绝对路径查存在+可执行位；裸名字扫 PATH。
    pub fn validate_runtime(&self) -> Result<()> {
        if let Some(parent) = std::path::Path::new(&self.router.state_file).parent() {
            check_dir_writable(parent, "router.state_file 父目录")?;
        }
        check_dir_writable(
            std::path::Path::new(&self.media.download_dir),
            "media.download_dir",
        )?;
        if let Some(f) = &self.log.file
            && let Some(parent) = std::path::Path::new(f).parent()
        {
            check_dir_writable(parent, "log.file 父目录")?;
        }
        check_binary_reachable(&self.acp.claude.path)?;
        Ok(())
    }

    fn with_expanded_paths(mut self) -> Self {
        self.router.state_file = expand_tilde(&self.router.state_file);
        self.acp.claude.sessions_dir = expand_tilde(&self.acp.claude.sessions_dir);
        self.media.download_dir = expand_tilde(&self.media.download_dir);
        if let Some(ref wd) = self.acp.claude.work_dir {
            self.acp.claude.work_dir = Some(expand_tilde(wd));
        }
        if let Some(ref f) = self.log.file {
            self.log.file = Some(expand_tilde(f));
        }
        self
    }
}

/// spec §6.4.3: the directory must exist (create it if missing) and accept
/// a probe file. The probe is created and removed immediately — it proves
/// writability for the state file / downloads / log file we create later.
fn check_dir_writable(dir: &std::path::Path, what: &str) -> Result<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| SebasError::Config(format!("{what} {} 创建失败: {e}", dir.display())))?;
    let probe = dir.join(format!(".sebas-write-probe-{}", std::process::id()));
    let probe_result = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&probe)
        .map(|_| ());
    let _ = std::fs::remove_file(&probe);
    probe_result.map_err(|e| SebasError::Config(format!("{what} {} 不可写: {e}", dir.display())))
}

/// spec §6.4.4: the ACP child binary must be reachable — an absolute (or
/// relative-with-separator) path is checked directly, a bare name is
/// resolved against PATH. Either way the file must exist and be executable.
fn check_binary_reachable(path: &str) -> Result<()> {
    let is_executable = |p: &std::path::Path| -> bool {
        if !p.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            true
        }
    };

    let found = if path.contains('/') {
        is_executable(std::path::Path::new(path))
    } else {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(path))))
            .unwrap_or(false)
    };
    if !found {
        return Err(SebasError::Config(format!(
            "找不到 ACP agent 二进制 '{path}'。请安装 claude（并确认它以 ACP 模式运行所需的包装），\
             或在 [acp.claude] path 配置可执行文件的绝对路径。"
        )));
    }
    Ok(())
}

pub fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into();
    }
    p.to_string()
}
