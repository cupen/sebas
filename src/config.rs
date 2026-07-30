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
    pub app_id: String,
    pub app_secret: String,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default = "default_chat_types")]
    pub allowed_chat_types: Vec<String>,
    /// Optional message sent to each restored chat on daemon startup.
    /// Empty (default) = nothing sent.
    #[serde(default)]
    pub hello_msg: String,
}

impl Default for FeishuConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_secret: String::new(),
            owner_id: String::new(),
            allowed_chat_types: default_chat_types(),
            hello_msg: String::new(),
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

impl Config {
    pub fn parse(s: &str) -> Result<Self> {
        let cfg: Config =
            toml::from_str(s).map_err(|e| SebasError::Config(format!("toml parse: {e}")))?;
        cfg.validate()?;
        Ok(cfg.with_expanded_paths())
    }

    fn validate(&self) -> Result<()> {
        if self.feishu.app_id.is_empty() {
            return Err(SebasError::Config("feishu.app_id is required".into()));
        }
        if self.feishu.app_secret.is_empty() {
            return Err(SebasError::Config("feishu.app_secret is required".into()));
        }
        // owner_id is optional; empty means skip owner filtering (single-user bots).
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

pub fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into();
    }
    p.to_string()
}
