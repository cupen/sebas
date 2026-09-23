use crate::error::{Result, SebasError};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub feishu: FeishuConfig,
    #[serde(default)]
    pub acp: AcpConfig,
    #[serde(default)]
    pub dispatch: DispatchConfig,
    #[serde(default)]
    pub card: sebas_feishu::cards::CardConfig,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub log: LogConfig,
    /// 受管服务的三节配置（simplify-service-config）：`[service.core]` /
    /// `[service.webui]` / `[service.router]`。旧的 `[watchdog.*]` 同名节
    /// 警告忽略（见 `warn_deprecated_watchdog_keys`）。
    #[serde(default)]
    pub service: ServiceConfig,
    /// watchdog 自身的运维配置：`[watchdog.im]` / `[watchdog.upgrade]` /
    /// `[watchdog.storage]` 与裸 `max_spawn_failures`（前缀保留 watchdog）。
    #[serde(default)]
    pub watchdog: WatchdogConfig,
    #[serde(default)]
    pub node_link: NodeLinkConfig,
    /// 工作区根目录（add-workspace-root）：机器级项目 containment 边界。
    /// 缺省段 = 装配点回退进程 cwd 并告警；`SEBAS_WORKSPACE_ROOT` env 优先。
    /// 顶层无 `deny_unknown_fields`，旧二进制忽略新键、新二进制忽略旧键，
    /// 双向滚动升级都不碎。
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// 操作者级 skill 仓（add-agent-skills D5）：一行可选覆盖，无 enabled /
    /// per-backend 开关——目录缺失当空仓处理，投影面向「所有有落点的 backend」。
    #[serde(default)]
    pub skills: SkillsConfig,
}

/// 顶层 `[skills]` 段（add-agent-skills D5）：只有仓路径一个可选键。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillsConfig {
    /// skill 仓目录覆盖；缺省 `~/.agents/skills`。支持 `~` 展开（with_expanded_paths
    /// 管线）；空白视同未配置（全仓空值语义一致）。
    #[serde(default)]
    pub dir: Option<String>,
}

/// skill 仓缺省路径（proposal/design D5）。
pub const DEFAULT_SKILLS_DIR: &str = "~/.agents/skills";

/// 顶层 `[workspace]` 段（add-workspace-root D1）：workspace root 是机器级
/// 概念——webui 只是执法者之一，节点没有 `[watchdog.webui]` 节，放顶层使
/// 「这台机器的项目根」只配一次。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkspaceConfig {
    /// 项目根目录。支持 `~` 展开（with_expanded_paths 管线）；按字面量保存、
    /// 不 canonicalize——范围判定（`within_workspace_root`）两侧同规范。
    #[serde(default)]
    pub root: Option<String>,
}

/// Wrapper for all ACP agent configs. TOML section `[acp.<agent>]` nests here.
/// Multi-agent: `default` names the kind used when a session does not request
/// one; `agents` maps an open kind slug to its driver-backed config.
/// `deny_unknown_fields` rejects legacy blocks like `[acp.claude]` at parse
/// time instead of silently dropping them.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcpConfig {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub agents: std::collections::HashMap<String, AgentConfig>,
}

/// One configured agent, tagged by driver. `Claude` drives the dedicated
/// Claude Code path; `Acp` drives any native-ACP agent via a launch command.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "driver", rename_all = "snake_case")]
pub enum AgentConfig {
    Claude(AcpClaudeConfig),
    Acp {
        command: Vec<String>,
        #[serde(default = "default_startup_timeout")]
        startup_timeout_secs: u64,
        #[serde(default = "default_idle_kill")]
        idle_kill_secs: u64,
        /// 产品展示名（可选，仅用于 UI 呈现；缺省 = agent 键名本身）。
        #[serde(default)]
        display: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeishuConfig {
    /// 显式启用开关（feishu 可选，make-feishu-optional-webui-primary）：
    /// - `Some(true)`：强制接入（凭据不完整会在 validate 报错）。
    /// - `Some(false)`：强制不接入（即使凭据齐全也不起飞书）。
    /// - `None`（缺省）：回退历史隐式判定 = `app_id` 与 `app_secret` 双非空。
    #[serde(default)]
    pub enabled: Option<bool>,
    // serde-default（空串）让 TOML 缺字段时解析不报错，把「必填」判定留给
    // validate() —— 这样 env 覆盖（SEBAS_FEISHU_APP_ID/SECRET）才有机会
    // 在 validate 前补齐字段（openspec/specs/cli-service/spec.md env > TOML）。
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
    /// 飞书 **新**会话的默认执行体（make-feishu-optional-webui-primary）：
    /// `true` = 走原生 sebas-agent 内核（`agent-*` 会话，输出在 webui 看，
    /// 不出飞书卡片）；`false`（默认）= 走 acp 桥（Claude Code，飞书卡片
    /// 渲染照旧）。既有原生会话不受影响。
    #[serde(default)]
    pub native_default: bool,
}

impl FeishuConfig {
    /// feishu 是否启用：显式 `enabled` 优先；缺省回退历史隐式判定
    /// （app_id 与 app_secret 双非空 = 接入，sebas-2ty）。
    pub fn is_enabled(&self) -> bool {
        self.enabled
            .unwrap_or(!self.app_id.is_empty() && !self.app_secret.is_empty())
    }

    /// feishu 是否启用：app_id 与 app_secret 同时非空。
    /// 两者同时为空 = 不接入飞书（feishu 可选，sebas-2ty）。
    /// 保持历史命名，内部委托给显式开关优先的 `is_enabled`。
    pub fn enabled(&self) -> bool {
        self.is_enabled()
    }
}

impl Default for FeishuConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            app_id: String::new(),
            app_secret: String::new(),
            owner_id: String::new(),
            allowed_chat_types: default_chat_types(),
            hello_msg: String::new(),
            bot_name: String::new(),
            native_default: false,
        }
    }
}

/// 飞书真实 wire 值只有 "p2p"（私聊）和 "group"（群聊）；"private" 是
/// 字段缺失时的本地缺省幻影值（events.rs），过滤侧另做 private↔p2p
/// 别名归一化兜底。sebas-5y5：旧默认 ["private","group"] 按字面匹配
/// 会把所有真实私聊消息静默丢弃。
fn default_chat_types() -> Vec<String> {
    vec!["p2p".into(), "group".into()]
}

#[derive(Debug, Clone, Deserialize)]
pub struct AcpClaudeConfig {
    #[serde(default = "default_claude_path")]
    pub path: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// 产品展示名（workbench-agent-wire-fix 3.1，可选）：仅用于 UI 呈现，
    /// 不是 wire 标识；缺省由目录层回退 agent id 本身
    /// （fix-webui-qa-defects 7.1：不再按 driver 推导 "Claude Code"）。
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default = "default_sessions_dir")]
    pub sessions_dir: String,
    #[serde(default)]
    pub work_dir: Option<String>,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_secs: u64,
    #[serde(default = "default_idle_kill")]
    pub idle_kill_secs: u64,
    /// （workbench-composer-input-polish 2.1）模型别名表覆盖：`None`（键缺
    /// 省）与空表都回退内置别名表（default/opus/sonnet/haiku）；非空列表
    /// 整体替换内置表。全名模型 id（如 `sonnet[1m]`）亦可写进来。
    #[serde(default)]
    pub models: Option<Vec<String>>,
}

impl Default for AcpClaudeConfig {
    fn default() -> Self {
        Self {
            path: default_claude_path(),
            args: vec![],
            display: None,
            sessions_dir: default_sessions_dir(),
            work_dir: None,
            startup_timeout_secs: default_startup_timeout(),
            idle_kill_secs: default_idle_kill(),
            models: None,
        }
    }
}

impl AcpClaudeConfig {
    /// （workbench-composer-input-polish 2.1）本 agent 实例生效的模型别名
    /// 表，单一归一出处：键缺省（`None`）与显式空表都回退内置——空表没有
    /// 可选词汇，等效「未覆盖」（任务 2.1「空表回退内置」）。
    pub fn resolved_models(&self) -> Vec<String> {
        match self.models.as_deref() {
            Some(list) if !list.is_empty() => list.to_vec(),
            _ => sebas_acp::claude::builtin_claude_models(),
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

impl AcpConfig {
    /// The kind used when a session does not request one. Falls back to
    /// `"claude"` (the historical single-agent default).
    pub fn default_kind(&self) -> &str {
        self.default.as_deref().unwrap_or("claude")
    }

    /// The executable (argv[0]) of the default agent, for reachability checks.
    /// Falls back to `"claude"` when no agent is configured (matches the
    /// historical behavior of always probing the claude binary).
    fn default_kind_binary(&self) -> String {
        self.command_for(self.default_kind())
            .and_then(|mut v| {
                if v.is_empty() {
                    None
                } else {
                    Some(v.remove(0))
                }
            })
            .unwrap_or_else(|| "claude".to_string())
    }

    /// The full argv (executable + args) for an agent kind, if configured.
    pub fn command_for(&self, kind: &str) -> Option<Vec<String>> {
        self.agents.get(kind).map(|a| match a {
            AgentConfig::Claude(c) => {
                let mut v = vec![c.path.clone()];
                v.extend(c.args.clone());
                v
            }
            AgentConfig::Acp { command, .. } => command.clone(),
        })
    }

    /// 静态 launch 策略标签（配置层概念，不上 wire；workbench-agent-wire-fix
    /// D3/A）：`"claude"` 或 `"acp"`。未知 kind 返回空串。
    pub fn driver_tag_of(&self, kind: &str) -> String {
        match self.agents.get(kind) {
            Some(AgentConfig::Claude(_)) => "claude".to_string(),
            Some(AgentConfig::Acp { .. }) => "acp".to_string(),
            None => String::new(),
        }
    }

    /// 产品展示名（可选 display 字段；缺省 None 由 catalog 层按 driver 推导）。
    pub fn display_for(&self, kind: &str) -> Option<String> {
        match self.agents.get(kind) {
            Some(AgentConfig::Claude(c)) => c.display.clone(),
            Some(AgentConfig::Acp { display, .. }) => display.clone(),
            None => None,
        }
    }

    /// The configured work directory for an agent kind (Claude only for now).
    pub fn work_dir_for(&self, kind: &str) -> Option<String> {
        match self.agents.get(kind) {
            Some(AgentConfig::Claude(c)) => c.work_dir.clone(),
            _ => None,
        }
    }

    /// The startup timeout for an agent kind.
    pub fn startup_timeout_for(&self, kind: &str) -> std::time::Duration {
        let secs = self
            .agents
            .get(kind)
            .map(|a| match a {
                AgentConfig::Claude(c) => c.startup_timeout_secs,
                AgentConfig::Acp {
                    startup_timeout_secs,
                    ..
                } => *startup_timeout_secs,
            })
            .unwrap_or_else(default_startup_timeout);
        std::time::Duration::from_secs(secs.max(1))
    }

    /// The idle-kill timeout for an agent kind (0 = never expire).
    pub fn idle_kill_for(&self, kind: &str) -> u64 {
        self.agents
            .get(kind)
            .map(|a| match a {
                AgentConfig::Claude(c) => c.idle_kill_secs,
                AgentConfig::Acp { idle_kill_secs, .. } => *idle_kill_secs,
            })
            .unwrap_or_else(default_idle_kill)
    }

    /// When `default` is absent and exactly one agent is configured, that
    /// agent becomes the implicit default (lets a bare `acp` hint resolve
    /// to the only configured kind). Idempotent; called once in `parse`.
    fn apply_implicit_default(&mut self) {
        if self.default.is_none() && self.agents.len() == 1 {
            let only = self.agents.keys().next().cloned().unwrap_or_default();
            if !only.is_empty() {
                self.default = Some(only);
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DispatchConfig {
    #[serde(default = "default_state_file")]
    pub state_file: String,
    #[serde(default = "default_channel_buffer")]
    pub channel_buffer: usize,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_sessions: usize,
    /// （fix-pending-queue-liveness 1.2）回合停滞看门狗阈值（秒）：WORKING
    /// 会话在无泊车审批、且持续该时长**没有任何事件**到达时，引擎强制把回合
    /// 收尾到终态、drain 待执行队列，并发出 warn 通知。`0` = 关闭看门狗。
    /// 默认 600（10 分钟）：claude 驱动自带的 hang 升级链（5m 静默 →
    /// interrupt ×3 → SIGTERM）先于它触发，看门狗只兜驱动判不了的场景。
    #[serde(default = "default_turn_stall_timeout")]
    pub turn_stall_timeout: u64,
}

impl Default for DispatchConfig {
    fn default() -> Self {
        Self {
            state_file: default_state_file(),
            channel_buffer: default_channel_buffer(),
            max_concurrent_sessions: default_max_concurrent(),
            turn_stall_timeout: default_turn_stall_timeout(),
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
fn default_turn_stall_timeout() -> u64 {
    600
}

fn default_node_link_listen() -> String {
    "127.0.0.1:9878".into()
}
fn default_node_link_token_ttl_secs() -> u64 {
    900
}

/// 执行节点入站链路（add-remote-execution-node）。
///
/// 默认**关**：这是一个新的网络入站面，必须显式打开。默认只监听回环——
/// 把它暴露到别的网络接口是部署决策（TLS 由部署方终止）。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NodeLinkConfig {
    /// 是否开放节点入站端点。
    #[serde(default)]
    pub enabled: bool,
    /// 监听地址（须为 `IP:PORT`；不接受域名，避免启动期解析歧义）。
    #[serde(default = "default_node_link_listen")]
    pub listen: String,
    /// 节点注册表文件；缺省为状态目录下的 `nodes.json`（single-state-dir
    /// 4.2，从 config 文件同目录迁来——唯一默认位置变化的落点）。
    #[serde(default)]
    pub registry_file: Option<String>,
    /// 首次启动（无节点且无待用 token）自动签发的配对 token 的有效期（秒）。
    /// bootstrap token 只用于把第一台节点接进来，之后管理入口接手。
    #[serde(default = "default_node_link_token_ttl_secs")]
    pub bootstrap_token_ttl_secs: u64,
}

impl Default for NodeLinkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: default_node_link_listen(),
            registry_file: None,
            bootstrap_token_ttl_secs: default_node_link_token_ttl_secs(),
        }
    }
}

/// 注册表路径推导（single-state-dir 4.2）：显式配置键优先（优先级不变，
/// tilde 展开），否则落在**状态目录**下的 `nodes.json`（此前默认是 config
/// 文件同目录——那是唯一发生迁移的落点，design D7）。
pub fn node_link_registry_path(
    registry_file: Option<&str>,
    _config_path: &std::path::Path,
) -> std::path::PathBuf {
    match registry_file {
        Some(p) if !p.trim().is_empty() => std::path::PathBuf::from(expand_tilde(p)),
        _ => sebas_domain::state_paths::StatePath::NodeRegistry.resolve(),
    }
}

impl NodeLinkConfig {
    /// 以已知 config 文件路径解析注册表位置。
    pub fn registry_path(&self, config_path: &std::path::Path) -> std::path::PathBuf {
        node_link_registry_path(self.registry_file.as_deref(), config_path)
    }
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

/// 受管服务三节的配置载体（simplify-service-config）：core / webui /
/// router 的配置节更名 `[watchdog.*]` → `[service.*]`；im/upgrade/storage
/// 保留 `[watchdog.*]` 前缀，仍在 [`WatchdogConfig`]。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServiceConfig {
    #[serde(default)]
    pub core: ServiceCoreConfig,
    #[serde(default)]
    pub webui: ServiceWebUiConfig,
    #[serde(default)]
    pub router: ServiceRouterConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchdogConfig {
    #[serde(default)]
    pub upgrade: WatchdogUpgradeConfig,
    #[serde(default)]
    pub storage: WatchdogStorageConfig,
    #[serde(default)]
    pub im: WatchdogImConfig,
    /// 受管服务连续 spawn 失败上限（fail-fast-on-startup-errors D1）：同一
    /// 服务连续 spawn 失败（或 ready 前 early-fatal 退出）达到该值 → 服务进入
    /// `failed-startup` 终态、watchdog 以 EX_TEMPFAIL (75) 退出，不再无限重试。
    /// 默认 3；`WatchdogConfig::default()` 与 TOML 缺省共用同一来源。
    #[serde(default = "default_max_spawn_failures")]
    pub max_spawn_failures: u32,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            upgrade: Default::default(),
            storage: Default::default(),
            im: Default::default(),
            max_spawn_failures: default_max_spawn_failures(),
        }
    }
}

fn default_max_spawn_failures() -> u32 {
    3
}

/// `sebas run` 模式下 core 子进程（会话核心 + ACP）的路径配置。
/// core 恒启动（enable-core-by-default）：无 `enabled` 开关，watchdog
/// 无条件拉起并监督；旧的 `enabled` 键被忽略并告警（见
/// `warn_deprecated_watchdog_keys`）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServiceCoreConfig {
    /// core session channel 的 Unix socket 路径（openspec/changes/
    /// add-core-session-channel）。空/缺省 → `$XDG_RUNTIME_DIR/sebas/core.sock`
    /// （或 per-uid 临时目录回退）。
    #[serde(default)]
    pub channel_path: Option<String>,
    /// core session channel 的握手 secret 文件路径（harden-core-channel-deployment
    /// D1）。空/缺省 → `<config 文件所在目录>/core.secret`。core 自动武装时把
    /// 本次启动的 secret 原子写入该文件（0600），无 env 注入的通道客户端
    /// （standalone webui / im / router 订阅）按 env → 文件顺序发现密钥。
    #[serde(default)]
    pub secret_file: Option<String>,
}

/// 解析 core session channel 的 secret 文件路径（D1 纯函数）：
/// `[service.core] secret_file` 显式键优先（`~` 由 with_expanded_paths 展开）；
/// 缺省推导为 `<config 文件所在目录>/core.secret`。双进程（core 与客户端）
/// 读同一份 `-c` config，路径天然一致；沙箱用自己的 config，隔离天然成立。
pub fn core_secret_file_path(
    secret_file: Option<&str>,
    config_path: &std::path::Path,
) -> std::path::PathBuf {
    match secret_file {
        Some(p) if !p.trim().is_empty() => std::path::PathBuf::from(expand_tilde(p)),
        _ => {
            let dir = config_path
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            dir.join("core.secret")
        }
    }
}

impl ServiceCoreConfig {
    /// 以已知 config 文件路径解析 secret 文件位置（`core_secret_file_path`
    /// 的方法形态，调用方不必拆字段）。
    pub fn secret_file_path(&self, config_path: &std::path::Path) -> std::path::PathBuf {
        core_secret_file_path(self.secret_file.as_deref(), config_path)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ServiceWebUiConfig {
    /// Let watchdog own the WebUI lifecycle. 默认开：watchdog 唯一默认启动
    /// 的服务，其余（core/router）由 WebUI 服务页按需启停。
    #[serde(default = "default_webui_enabled")]
    pub enabled: bool,
    /// Bind address. Phase 2.3 tightens non-loopback security.
    #[serde(default = "default_webui_host")]
    pub host: String,
    /// Bind port for watchdog-owned WebUI.
    #[serde(default = "default_webui_port")]
    pub port: u16,
    /// 登录鉴权总开关（add-webui-auth-switch）。默认 true：凭据存在即强制
    /// 登录。测试/联调环境可设 false 全路由免登录；关闭时非 loopback bind
    /// 一律拒绝（见 webui_cmd 的安全门）。
    #[serde(default = "default_webui_auth")]
    pub auth: bool,
    /// 归档保留期（天）。默认 30；过期归档在 webui 启动时及每次列表请求时
    /// 删除（add-project-session-actions）。配置项归属 `[service.webui]`
    /// （webui 配置的现唯一归属地，无独立 `[webui]` 顶层节）。
    #[serde(default = "default_archive_retention_days")]
    pub archive_retention_days: u64,
    // 兼容性（add-workspace-root）：未标 `deny_unknown_fields`，旧键
    // `allowed_roots` 出现在配置里被静默忽略、解析不报错——白名单机制已由
    // 单一 workspace root 接管（顶层 `[workspace] root` / `SEBAS_WORKSPACE_ROOT`）。
}

impl Default for ServiceWebUiConfig {
    fn default() -> Self {
        Self {
            enabled: default_webui_enabled(),
            host: default_webui_host(),
            port: default_webui_port(),
            auth: default_webui_auth(),
            archive_retention_days: default_archive_retention_days(),
        }
    }
}

fn default_archive_retention_days() -> u64 {
    30
}

/// 解析生效的 workspace root（add-workspace-root，spec「解析顺序」）：
/// `SEBAS_WORKSPACE_ROOT` 环境变量 > 配置项 `[workspace] root` > 回退进程
/// cwd。返回 `(根, 是否回退)`；回退时**装配点**应打一条启动告警（D5：判定
/// 函数本身不打日志——它被高频调用，告警是启动期事实）。env / config 取值
/// 按字面量保存（`~` 展开只走 config 管线，见 `with_expanded_paths`），是否
/// canonicalize 由范围判定函数负责——`within_workspace_root` 两侧同规范。
/// 空值语义与全仓一致：空白/纯空白字符串视同未配置。
pub fn resolve_workspace_root(
    env: Option<&str>,
    config_root: Option<&str>,
    cwd: &std::path::Path,
) -> (std::path::PathBuf, bool) {
    let explicit = env
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            config_root
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from)
        });
    match explicit {
        Some(root) => (root, false),
        None => (cwd.to_path_buf(), true),
    }
}

/// add-system-dir-denylist 3.1：workspace root 解析为系统目录时打启动 warn
/// （不阻断——名单对 root 只提示不执法，注册层才是执法线）。判定与注册
/// 执法同源（sebas_webui::fs::is_system_dir，双侧 canonicalize 精确匹配），
/// 抽成小函数供两处装配点（run.rs core --webui / webui_cmd.rs 独立 webui）
/// 共用与直测。
pub(crate) fn warn_if_workspace_root_is_system_dir(root: &std::path::Path) {
    if sebas_webui::fs::is_system_dir(root) {
        let msg = format!(
            "workspace root 解析为系统目录 {}；系统目录不可注册为项目，且此配置下注册围栏形同虚设——建议改指具体工作区（[workspace] root 或 SEBAS_WORKSPACE_ROOT）",
            root.display()
        );
        tracing::warn!("{msg}");
        eprintln!("warning: {msg}");
    }
}

fn default_webui_enabled() -> bool {
    true
}

fn default_webui_auth() -> bool {
    true
}

fn default_webui_host() -> String {
    "127.0.0.1".into()
}

fn default_webui_port() -> u16 {
    9797
}

/// watchdog 模式下 router 子进程的开关（默认关：未 opt-in 不 spawn）。
/// router 只以独立进程存在（unify-router-process-shape）：手工
/// `sebas router` 或本开关开启后的 watchdog 受管子进程，同一入口；
/// 内嵌形态（`core --router`）已删除（sebas-08c）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServiceRouterConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// `[watchdog.im]`：独立 IM 服务（`sebas im`）的托管开关。缺省跟随飞书
/// 启用判定（`[feishu] enabled` 或隐式回退）；显式给出时以显式值为准
/// （extract-im-service，spec feishu-option / watchdog）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WatchdogImConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchdogUpgradeConfig {
    /// GitHub 仓库（owner/repo）
    #[serde(default = "default_github_repo")]
    pub github_repo: String,
    /// release 更新（下载 + 校验 + 安装）的 updater 子进程超时（秒）。
    #[serde(default = "default_updater_timeout")]
    pub updater_timeout_secs: u64,
    /// dev 更新（cargo build --release）的 updater 子进程超时（秒）。
    /// 编译整个 workspace 可耗时数分钟，故默认远大于 release 路径。
    #[serde(default = "default_dev_build_timeout")]
    pub dev_build_timeout_secs: u64,
}

impl Default for WatchdogUpgradeConfig {
    fn default() -> Self {
        Self {
            github_repo: default_github_repo(),
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

/// 已废弃且无消费者的 `[watchdog.upgrade]` 键。解析时扫描原始 TOML，
/// 命中则 warn 一行提示（不报错，旧配置照常启动）。
const DEPRECATED_WATCHDOG_UPGRADE_KEYS: &[&str] =
    &["check_on_start", "max_retries", "retry_delay_secs"];

/// 扫描原始 TOML，返回 `[watchdog.upgrade]` 段中出现的废弃键。
fn deprecated_watchdog_upgrade_hits(raw: &str) -> Vec<&'static str> {
    let Ok(value) = raw.parse::<toml::Table>() else {
        return Vec::new();
    };
    let Some(watchdog) = value.get("watchdog").and_then(|v| v.as_table()) else {
        return Vec::new();
    };
    let Some(upgrade) = watchdog.get("upgrade").and_then(|v| v.as_table()) else {
        return Vec::new();
    };
    DEPRECATED_WATCHDOG_UPGRADE_KEYS
        .iter()
        .copied()
        .filter(|k| upgrade.contains_key(*k))
        .collect()
}

/// simplify-service-config：`[watchdog.{core,webui,router}]` 三节已更名
/// `[service.*]`。返回命中的旧节名（原始 TOML 扫描；serde 侧这些表已不在
/// `WatchdogConfig`，未知子表被静默忽略——可见性全靠本扫描）。
fn deprecated_watchdog_service_tables(raw: &str) -> Vec<&'static str> {
    let Ok(value) = raw.parse::<toml::Table>() else {
        return Vec::new();
    };
    let Some(watchdog) = value.get("watchdog").and_then(|v| v.as_table()) else {
        return Vec::new();
    };
    ["core", "webui", "router"]
        .iter()
        .copied()
        .filter(|k| watchdog.contains_key(*k))
        .collect()
}

fn warn_deprecated_watchdog_keys(raw: &str) {
    let hit = deprecated_watchdog_upgrade_hits(raw);
    if !hit.is_empty() {
        tracing::warn!(
            "config [watchdog.upgrade] has deprecated fields ({}): they no longer work, remove them from the config",
            hit.join(", ")
        );
    }
    let renamed = deprecated_watchdog_service_tables(raw);
    if !renamed.is_empty() {
        // parse 发生在 tracing 初始化之前（watchdog 在 parse 后才 init），
        // tracing::warn 会被静默丢弃——deprecation 提示同时走 stderr，确保可见。
        let msg = format!(
            "config deprecated section(s) [watchdog.{}] ignored: renamed to [service.{}] - move the section",
            renamed.join(", "),
            renamed.join(", ")
        );
        tracing::warn!("{msg}");
        eprintln!("warning: {msg}");
    }
}

/// [service.webui] 未知键点名（webui auth e2e 复盘）：该节承载 auth 开关，
/// serde 刻意不拒绝未知键（旧二进制读新配置的前向兼容），代价是键名打错
/// 或写错节都静默落回默认——`auth` 误配的症状（首启设置页翻登录页）与
/// 「开关开着」完全同貌，无从排查。启动期把被忽略的键点名，误配当场可见。
fn warn_unknown_webui_keys(raw: &str) {
    let unknown = unknown_webui_keys(raw);
    if !unknown.is_empty() {
        // 同 warn_deprecated_watchdog_keys：tracing 可能尚未初始化，stderr 兜底。
        let msg = format!(
            "config [service.webui] has unknown field(s) ({}) ignored: typo or removed key - a miswritten `auth` key silently means auth stays on",
            unknown.join(", ")
        );
        tracing::warn!("{msg}");
        eprintln!("warning: {msg}");
    }
}

/// 纯收集（测试锚点）：raw 中 `[service.webui]` 表内不在已知字段集的键。
fn unknown_webui_keys(raw: &str) -> Vec<String> {
    const KNOWN: [&str; 5] = ["enabled", "host", "port", "auth", "archive_retention_days"];
    let Ok(v) = toml::from_str::<toml::Value>(raw) else {
        return Vec::new(); // 解析失败由主 parse 报错，这里不抢戏
    };
    v.get("service")
        .and_then(|s| s.get("webui"))
        .and_then(|w| w.as_table())
        .map(|table| {
            table
                .keys()
                .filter(|k| !KNOWN.contains(&k.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn default_github_repo() -> String {
    "cupen/sebas".into()
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
    /// `~` paths. Priority per openspec/specs/cli-service/spec.md: CLI flags > env vars > TOML >
    /// defaults (CLI flags are applied by the caller before/after this).
    pub fn parse(s: &str) -> Result<Self> {
        warn_deprecated_watchdog_keys(s);
        warn_unknown_webui_keys(s);
        let mut cfg: Config =
            toml::from_str(s).map_err(|e| SebasError::Config(format!("toml parse: {e}")))?;
        cfg.acp.apply_implicit_default();
        cfg.apply_env_overrides();
        cfg.validate()?;
        Ok(cfg.with_expanded_paths())
    }

    /// env vars override TOML for the sensitive/ops fields (openspec/specs/cli-service/spec.md).
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
        if let Ok(v) = std::env::var("SEBAS_NODE_LINK_LISTEN")
            && !v.is_empty()
        {
            self.node_link.listen = v;
        }
    }

    fn validate(&self) -> Result<()> {
        // 节点链路：开着就必须是一个可 bind 的 IP:PORT —— 启动期就报错，
        // 而不是等到 ready 之后由 arm 失败（那时已经在服务中了）。
        if self.node_link.enabled
            && self
                .node_link
                .listen
                .parse::<std::net::SocketAddr>()
                .is_err()
        {
            return Err(crate::error::SebasError::Config(format!(
                "[node_link] listen {:?} 不是 IP:PORT（不接受域名）",
                self.node_link.listen
            )));
        }
        // feishu 是可选项（sebas-2ty）：app_id/app_secret 同时为空 = 不接入
        // 飞书（`sebas core` 以无飞书模式运行；watchdog 下 core 服务默认不
        // 启动）。只配其一属于半配置，明确报错而不是静默半启用。
        if self.feishu.app_id.is_empty() != self.feishu.app_secret.is_empty() {
            return Err(SebasError::Config(
                "feishu.app_id 与 feishu.app_secret 必须同时配置；同时留空 = 不启用飞书".into(),
            ));
        }
        // 显式开关（make-feishu-optional-webui-primary）：enabled = true 但凭据
        // 不完整 = 半配置意图，拒绝启动；enabled = false 而凭据齐全 = 以显式
        // 值为准（不接入），交给 run.rs 记日志提示，这里不算错误。
        if self.feishu.enabled == Some(true)
            && (self.feishu.app_id.is_empty() || self.feishu.app_secret.is_empty())
        {
            return Err(SebasError::Config(
                "feishu.enabled = true 但 app_id/app_secret 未完整配置；\
                 请同时填写凭据，或设 enabled = false（显式停用）"
                    .into(),
            ));
        }
        // owner_id 决策（sebas-nya）：维持**可选**，偏离 openspec/specs/cli-service/spec.md 的必填。
        // 依据：openspec/specs/cli-service/spec.md 同时写明「只有 3 个必填字段」只是设计原则，而实际
        // 部署（config/config.toml）以 owner_id = "" 运行单用户机器人；
        // 空值语义 = 跳过 owner 过滤。风险（任何飞书用户都可驱动 bot）在
        // run::run 启动时以 warn 提示，并在 config.toml.example 文档化。
        self.validate_agent_args()?;
        Ok(())
    }

    /// claude-driver agent 的 `args` argv 保真（fix-webui-qa-defects 6.1，
    /// design D4）：内部 flag-map 无法表达位置参数，运行期只会 warn 后丢弃
    /// ——「配置写了、子进程没收到」的静默失效。解析期直接拒绝：点名参数
    /// 并给出键值形式示例。判定与 driver 的 `args_to_extra_args` 配对规则
    /// 同构（`--flag` 后紧跟的非 `--` 令牌是它的值；其余无 `--` 前缀令牌
    /// 一律按位置参数拒绝——含单 `-` 形态，driver 的键化同样表达不了）。
    fn validate_agent_args(&self) -> Result<()> {
        for (name, agent) in &self.acp.agents {
            let AgentConfig::Claude(cfg) = agent else {
                continue;
            };
            let args = &cfg.args;
            let mut i = 0;
            while i < args.len() {
                if args[i].starts_with("--") {
                    // 键值配对：跳过 `--flag` 与它消费的值（非 `--` 前缀）。
                    if matches!(args.get(i + 1), Some(v) if !v.starts_with("--")) {
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                return Err(SebasError::Config(format!(
                    "[acp.agents.{name}] args 含位置参数 {:?}：\
                     位置参数不会到达子进程 argv（会被静默丢弃），\
                     请改用键值形式（如 args = [\"--scenario\", \"thinking\"]）",
                    args[i]
                )));
            }
        }
        Ok(())
    }

    /// Environmental startup checks (openspec/specs/cli-service/spec.md) that need a real
    /// filesystem and PATH — kept OUT of `parse` so unit tests can
    /// validate pure config on hosts without a claude binary. `run::run`
    /// calls this before touching the network or spawning anything.
    ///
    /// 1. 目录可写性：state_file 父目录、media.download_dir、log.file 父
    ///    目录（缺失则创建；创建/探测失败 → 友好 Config 错误，不 panic）。
    /// 2. ACP 子进程二进制可达：绝对路径查存在+可执行位；裸名字扫 PATH。
    pub fn validate_runtime(&self) -> Result<()> {
        if let Some(parent) = std::path::Path::new(&self.dispatch.state_file).parent() {
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
        check_binary_reachable(&self.acp.default_kind_binary())?;
        Ok(())
    }

    fn with_expanded_paths(mut self) -> Self {
        self.dispatch.state_file = expand_tilde(&self.dispatch.state_file);
        if let Some(ref f) = self.service.core.secret_file {
            self.service.core.secret_file = Some(expand_tilde(f));
        }
        for agent in self.acp.agents.values_mut() {
            if let AgentConfig::Claude(c) = agent {
                c.sessions_dir = expand_tilde(&c.sessions_dir);
                if let Some(ref wd) = c.work_dir {
                    c.work_dir = Some(expand_tilde(wd));
                }
            }
        }
        self.media.download_dir = expand_tilde(&self.media.download_dir);
        if let Some(ref d) = self.skills.dir {
            self.skills.dir = Some(expand_tilde(d));
        }
        if let Some(ref root) = self.workspace.root {
            self.workspace.root = Some(expand_tilde(root));
        }
        if let Some(ref f) = self.log.file {
            self.log.file = Some(expand_tilde(f));
        }
        self
    }

    /// skill 仓目录（已展开 `~`）：`[skills] dir` 非空优先，缺省回退
    /// `~/.agents/skills`。返回的是路径字符串；目录缺失不是错误（空仓语义，
    /// 由 `skills::scan_store` 按空列表处理）。
    pub fn skills_dir(&self) -> String {
        self.skills
            .dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(expand_tilde)
            .unwrap_or_else(|| expand_tilde(DEFAULT_SKILLS_DIR))
    }
}

/// openspec/specs/cli-service/spec.md: the directory must exist (create it if missing) and accept
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

/// openspec/specs/cli-service/spec.md: the ACP child binary must be reachable — an absolute (or
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
            "找不到 ACP agent 二进制 '{path}'。请安装该 CLI（claude 需以 ACP 模式运行的包装），\
             或在 [acp.agents.<name>] path/command 配置可执行文件的绝对路径。"
        )));
    }
    Ok(())
}

/// Appended to every default tracing filter: the third-party openlark WS
/// client logs its full connect URL (incl. access_key/ticket) at info.
pub(crate) const LOG_FILTER_QUIET: &str = ",openlark_client=warn";

/// Default filter when neither RUST_LOG nor [log] level is configured.
pub(crate) const DEFAULT_LOG_FILTER: &str = "info,openlark_client=warn";

// `expand_tilde` 唯一实现在 `sebas_domain::prim`（add-domain-layer 2.4）；
// 本模块经 `use` 保持既有调用点不变。
pub use sebas_domain::prim::expand_tilde;

#[cfg(test)]
mod tests {
    use super::*;

    // ── fix-webui-qa-defects 6.1（design D4）：claude args argv 保真 ────────

    #[test]
    fn positional_claude_arg_is_rejected_at_parse_time() {
        let err = Config::parse(
            "[acp.agents.claude]
driver = \"claude\"
args = [\"thinking\"]
",
        )
        .expect_err("位置参数必须在解析期拒绝");
        let msg = format!("{err}");
        assert!(msg.contains("thinking"), "错误点名参数: {msg}");
        assert!(
            msg.contains("--scenario") && msg.contains("thinking"),
            "错误给出键值形式示例: {msg}"
        );
        assert!(msg.contains("acp.agents.claude"), "错误点名配置键: {msg}");
    }

    #[test]
    fn keyed_claude_args_parse() {
        let cfg = Config::parse(
            "[acp.agents.claude]
driver = \"claude\"
args = [\"--scenario\", \"thinking\"]
",
        )
        .expect("键值形式的 args 必须通过解析");
        let AgentConfig::Claude(c) = cfg.acp.agents.get("claude").expect("agent present") else {
            panic!("claude agent");
        };
        assert_eq!(c.args, vec!["--scenario".to_string(), "thinking".to_string()]);
    }

    #[test]
    fn mixed_args_flag_value_and_trailing_flags_parse() {
        // `--flag value` 消费值对；值以 `--` 开头时不消费（与 driver 的
        // flag-map 配对规则同构，值归属下一个 flag）。
        let cfg = Config::parse(
            "[acp.agents.claude]
driver = \"claude\"
args = [\"--verbose\", \"--model\", \"opus\"]
",
        )
        .expect("混排 args 必须通过解析");
        let AgentConfig::Claude(c) = cfg.acp.agents.get("claude").unwrap() else {
            panic!("claude agent");
        };
        assert_eq!(c.args.len(), 3);
    }

    #[test]
    fn positional_after_a_value_pair_is_also_rejected() {
        let err = Config::parse(
            "[acp.agents.claude]
driver = \"claude\"
args = [\"--model\", \"opus\", \"thinking\"]
",
        )
        .expect_err("值对之后的位置参数同样拒绝");
        assert!(format!("{err}").contains("thinking"));
    }

    #[test]
    fn acp_driver_agent_args_are_not_checked() {
        // 通用 ACP driver 的 command 本就是 argv 数组，无 flag-map 问题。
        let cfg = Config::parse(
            "[acp.agents.open]
driver = \"acp\"
command = [\"opencode\", \"positional-ok\"]
",
        )
        .expect("acp driver 不受 args 检查约束");
        assert!(cfg.acp.agents.contains_key("open"));
    }

    #[test]
    fn legacy_positional_configs_now_fail_loudly_instead_of_silently_dropping() {
        // 释放说明点名（design Risks）：此前静默失效的配置现在启动报错。
        let err = Config::parse(
            "[acp.agents.claude]
driver = \"claude\"
args = [\"verbose\"]
",
        )
        .expect_err("无 -- 前缀的旧配置必须显式报错");
        assert!(format!("{err}").contains("verbose"));
    }

    #[test]
    fn skills_dir_default_and_override() {
        // add-agent-skills 3.2：缺省回退 ~/.agents/skills（已展开 ~），`[skills]
        // dir` 覆盖生效。Windows 反斜杠不能裸写进 TOML basic string，路径归一
        // 为正斜杠。
        let cfg = Config::parse("").expect("空配置应可解析");
        let default_dir = cfg.skills_dir();
        assert!(!default_dir.starts_with("~"), "~ 必须已展开: {default_dir}");
        assert!(
            default_dir.replace('\\', "/").ends_with(".agents/skills"),
            "缺省必须是 ~/.agents/skills: {default_dir}"
        );

        let tmp = tempfile::tempdir().unwrap();
        let override_dir = tmp.path().join("my-skills");
        // TOML 里写正斜杠（Windows 反斜杠不能裸写进 basic string）；配置按
        // 字面量保存，skills_dir() 原样返回正斜杠形式。
        let override_toml = override_dir.display().to_string().replace('\\', "/");
        let cfg = Config::parse(&format!("[skills]\ndir = \"{override_toml}\"\n"))
            .expect("[skills] dir 应可解析");
        assert_eq!(cfg.skills_dir(), override_toml, "显式覆盖生效");

        // 空白取值视同未配置（全仓空值语义一致），回退缺省。
        let cfg = Config::parse("[skills]\ndir = \"   \"\n").expect("空白 dir 应可解析");
        assert_eq!(cfg.skills_dir(), default_dir);
    }

    #[test]
    fn webui_enabled_by_default_and_router_disabled() {
        // watchdog 默认服务面（enable-core-by-default）：core 恒启动（无
        // enabled 开关），webui 默认开，router 默认关。
        let cfg = Config::parse("").expect("空配置应可解析（feishu 可选）");
        assert!(cfg.service.webui.enabled, "webui 应默认启用");
        assert!(!cfg.service.router.enabled, "router 应默认停用");
        assert!(!cfg.feishu.enabled(), "无凭证时 feishu 应视为未启用");
        assert!(
            cfg.workspace.root.is_none(),
            "workspace root 缺省 = None（装配点回退 cwd 并告警）"
        );
        assert!(
            cfg.service.core.secret_file.is_none(),
            "secret_file 缺省 = None（由 core_secret_file_path 推导）"
        );
    }

    #[test]
    fn deprecated_watchdog_service_tables_are_warned_and_ignored() {
        // enable-core-by-default + simplify-service-config：`[watchdog.core]`
        // 整节废弃（先 enabled 键、后整节更名），警告忽略、serde 静默跳过。
        let cfg =
            Config::parse("[watchdog.core]\nenabled = false\n").expect("旧节应被忽略而非报错");
        assert_eq!(
            deprecated_watchdog_service_tables("[watchdog.core]\nenabled = false\n"),
            vec!["core"]
        );
        assert!(
            deprecated_watchdog_service_tables("[service.core]\nchannel_path = \"/x\"\n")
                .is_empty()
        );
        // 旧键不再能关掉 core：结构里没有 enabled 字段可读，恒启动由 watchdog 保证。
        let _ = cfg;
    }

    #[test]
    fn secret_file_path_explicit_key_wins() {
        // harden-core-channel-deployment 1.1：`[service.core] secret_file`
        // 显式键优先于缺省推导。TOML 值用正斜杠——Windows 反斜杠路径在
        // basic string 里是 `\U` 转义起点，会解析失败。
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let explicit = dir.path().join("keys/channel.key");
        // TOML basic string 不能裸写反斜杠（`\U` 是 unicode 转义），Windows
        // 路径归一为正斜杠——std::path 在 Windows 上同样接受。
        let explicit_toml = explicit.display().to_string().replace('\\', "/");
        let cfg = Config::parse(&format!(
            "[service.core]\nsecret_file = \"{}\"\n",
            explicit_toml
        ))
        .expect("secret_file 键应可解析");
        assert_eq!(
            cfg.service.core.secret_file_path(&config_path),
            explicit,
            "显式键必须原样（已展开）生效"
        );
    }

    #[test]
    fn secret_file_path_defaults_to_config_dir() {
        // 缺省推导 = `<config 文件所在目录>/core.secret`——沙箱用自己目录下的
        // config，secret 文件随之隔离，绝不落进真实 ~/.sebas（D1）。
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("nested").join("config.toml");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let cfg = Config::parse("").expect("空配置应可解析");
        assert_eq!(
            cfg.service.core.secret_file_path(&config_path),
            dir.path().join("nested").join("core.secret"),
            "缺省 secret 文件必须与 config 文件同目录"
        );
        // 空/空白字符串视同未配置（与 channel_path 的空值语义一致）。
        let cfg_blank = Config::parse("[service.core]\nsecret_file = \"\"\n").unwrap();
        assert_eq!(
            cfg_blank.service.core.secret_file_path(&config_path),
            dir.path().join("nested").join("core.secret"),
            "空字符串 secret_file 回退缺省推导"
        );
    }

    #[test]
    fn workspace_root_section_parses() {
        // add-workspace-root 1.1：顶层 `[workspace] root` 解析 + `~` 展开
        // （with_expanded_paths 经由 Config::parse 的既有展开管线生效）。
        // 注意 Windows 反斜杠不能裸写进 TOML basic string，归一为正斜杠。
        let cfg = Config::parse(
            r#"
[workspace]
root = "~/projects"
"#,
        )
        .expect("[workspace] root 应可解析");
        let root = cfg.workspace.root.as_deref().expect("root 应在场");
        assert!(!root.starts_with("~"), "~ 必须已展开: {root}");
    }

    #[test]
    fn legacy_allowed_roots_key_is_ignored() {
        // add-workspace-root：白名单机制退役。旧配置带 `allowed_roots` 键
        // 必须静默忽略（解析不报错），范围约束改由 workspace root 接管。
        let cfg = Config::parse(
            r#"
[service.webui]
allowed_roots = ["~/work", "/srv/projects"]
"#,
        )
        .expect("含 allowed_roots 旧键的配置必须照常解析");
        assert!(
            cfg.workspace.root.is_none(),
            "旧键不得被解读为 workspace root"
        );
    }

    #[test]
    fn resolve_workspace_root_env_beats_config_beats_cwd() {
        // add-workspace-root 1.4：三态解析——env 优先 / 配置次之 / 双缺省
        // 回退 cwd 并置回退 flag（装配点据此打告警）。
        let cwd = std::path::Path::new("/process/cwd");

        let (root, fell_back) =
            resolve_workspace_root(Some("/env/root"), Some("/config/root"), cwd);
        assert_eq!(root, std::path::PathBuf::from("/env/root"), "env 优先");
        assert!(!fell_back, "env 生效时不算回退");

        let (root, fell_back) = resolve_workspace_root(None, Some("/config/root"), cwd);
        assert_eq!(root, std::path::PathBuf::from("/config/root"), "配置次之");
        assert!(!fell_back, "配置生效时不算回退");

        let (root, fell_back) = resolve_workspace_root(None, None, cwd);
        assert_eq!(root, cwd, "双缺省回退进程 cwd");
        assert!(fell_back, "回退必须置 flag 供装配点告警");

        // 空值语义：空白字符串视同未配置（与全仓 env/config 空值约定一致）。
        let (root, fell_back) = resolve_workspace_root(Some("  "), Some(""), cwd);
        assert_eq!(root, cwd, "空白取值视同未配置");
        assert!(fell_back);
    }

    #[test]
    fn warn_if_workspace_root_is_system_dir_flags_denylist_hits_only() {
        // add-system-dir-denylist 3.1：告警判定与注册执法同源。函数无返回值
        // （可观察行为是 stderr/tracing 告警），这里钉两态谓词 + 装配函数
        // 不 panic：unix 用 `/` 断言命中分支，tempdir 与不存在路径是放行分支。
        assert!(
            sebas_webui::fs::is_system_dir(std::path::Path::new("/")),
            "unix `/` must hit the denylist (warn branch)"
        );
        let t = tempfile::tempdir().unwrap();
        assert!(
            !sebas_webui::fs::is_system_dir(t.path()),
            "tempdir under /tmp must pass (silent branch)"
        );
        warn_if_workspace_root_is_system_dir(std::path::Path::new("/"));
        warn_if_workspace_root_is_system_dir(t.path());
        warn_if_workspace_root_is_system_dir(std::path::Path::new("/no/such/root"));
    }

    #[test]
    fn feishu_optional_but_not_half_configured() {
        // 同时留空 = 不启用飞书，合法。
        let both_empty = Config::parse("").expect("同时留空应可解析");
        assert!(!both_empty.feishu.enabled());

        // 只配其一 = 半配置，明确报错。
        let only_id = Config::parse(
            r#"
[feishu]
app_id = "cli_a1b2"
"#,
        );
        assert!(only_id.is_err(), "只配 app_id 必须报错");
        let only_secret = Config::parse(
            r#"
[feishu]
app_secret = "s"
"#,
        );
        assert!(only_secret.is_err(), "只配 app_secret 必须报错");

        // 同时配置 = 启用。
        let both = Config::parse(
            r#"
[feishu]
app_id = "cli_a1b2"
app_secret = "s"
"#,
        )
        .expect("完整 feishu 配置应可解析");
        assert!(both.feishu.enabled());
    }

    #[test]
    fn feishu_explicit_enabled_switch_four_states() {
        // 态 1：enabled = false + 凭据齐全 → 显式关闭优先（不接入）。
        let off_with_creds = Config::parse(
            r#"
[feishu]
enabled = false
app_id = "cli_a1b2"
app_secret = "s"
"#,
        )
        .expect("enabled=false + 凭据应可解析");
        assert!(
            !off_with_creds.feishu.is_enabled(),
            "显式 false 应优先于凭据"
        );

        // 态 2：enabled = true + 凭据缺失 → 配置错误拒绝启动。
        let on_no_creds = Config::parse(
            r#"
[feishu]
enabled = true
"#,
        );
        assert!(on_no_creds.is_err(), "enabled=true 但无凭据必须报错");
        assert!(
            on_no_creds
                .err()
                .unwrap()
                .to_string()
                .contains("enabled = true"),
            "报错应指明开关与凭据不匹配"
        );

        // 态 3：缺省 + 凭据空 → 不接入（历史行为）。
        let default_no_creds = Config::parse("").expect("空配置可解析");
        assert!(!default_no_creds.feishu.is_enabled(), "缺省+空凭据应不接入");

        // 态 4：缺省 + 凭据齐全 → 接入（历史隐式判定回退）。
        let default_with_creds = Config::parse(
            r#"
[feishu]
app_id = "cli_a1b2"
app_secret = "s"
"#,
        )
        .expect("缺省+凭据可解析");
        assert!(
            default_with_creds.feishu.is_enabled(),
            "缺省回退隐式判定应接入"
        );
    }

    #[test]
    fn watchdog_webui_explicit_disabled_wins() {
        let raw = r#"
[service.webui]
enabled = false
"#;
        let cfg = Config::parse(raw).expect("显式关闭应可解析");
        assert!(!cfg.service.webui.enabled, "显式 false 应优先于默认值");
    }

    #[test]
    fn watchdog_webui_auth_defaults_true() {
        let cfg = Config::parse("").expect("空配置应可解析");
        assert!(
            cfg.service.webui.auth,
            "鉴权开关缺省必须为 true（生产安全底线）"
        );
    }

    #[test]
    fn watchdog_webui_auth_explicit_false_wins() {
        let raw = r#"
[service.webui]
auth = false
"#;
        let cfg = Config::parse(raw).expect("显式关鉴权应可解析");
        assert!(!cfg.service.webui.auth, "显式 false 应优先于默认值");
    }

    #[test]
    fn unknown_webui_keys_are_named_for_the_operator() {
        // 键名打错（auht）：parse 仍成功（前向兼容刻意宽容），但键被点名。
        let raw = "[service.webui]\nauht = false\n";
        assert!(Config::parse(raw).is_ok());
        assert_eq!(unknown_webui_keys(raw), vec!["auht".to_string()]);
        // 已退役的 allowed_roots 同样被点名（静默忽略 → 启动期可见）。
        let legacy = "[service.webui]\nallowed_roots = [\"/tmp\"]\n";
        assert_eq!(
            unknown_webui_keys(legacy),
            vec!["allowed_roots".to_string()]
        );
        // 干净配置零误报：全字段 + 缺节都不在名单上。
        let clean = "[service.webui]\nenabled = true\nhost = \"127.0.0.1\"\nport = 9797\nauth = false\narchive_retention_days = 30\n";
        assert!(unknown_webui_keys(clean).is_empty());
        assert!(unknown_webui_keys("[feishu]\nenabled = false\n").is_empty());
    }

    #[test]
    fn deprecated_watchdog_upgrade_fields_parse_but_are_flagged() {
        let raw = r#"
[feishu]
app_id = "a"
app_secret = "b"

[watchdog.upgrade]
check_on_start = true
max_retries = 5
retry_delay_secs = 2
updater_timeout_secs = 123
"#;
        // 旧配置不报错：parse 成功，有效字段照常读取。
        let cfg = Config::parse(raw).expect("含废弃字段的配置必须能解析");
        assert_eq!(cfg.watchdog.upgrade.updater_timeout_secs, 123);
        // 废弃键被全部识别（parse 内部会对它们 warn 一行）。
        let mut hits = deprecated_watchdog_upgrade_hits(raw);
        hits.sort_unstable();
        assert_eq!(
            hits,
            vec!["check_on_start", "max_retries", "retry_delay_secs"]
        );
    }

    #[test]
    fn clean_config_has_no_deprecated_hits() {
        let raw = r#"
[feishu]
app_id = "a"
app_secret = "b"

[watchdog.upgrade]
updater_timeout_secs = 42
"#;
        assert!(deprecated_watchdog_upgrade_hits(raw).is_empty());
    }

    #[test]
    fn watchdog_webui_default_enabled() {
        // 无 watchdog.webui 段 → 默认启用
        let raw = r#"
[feishu]
app_id = "a"
app_secret = "b"
"#;
        let cfg = Config::parse(raw).expect("config parses");
        assert!(cfg.service.webui.enabled, "默认 webui 应启用");
        assert_eq!(cfg.service.webui.host, "127.0.0.1");
        assert_eq!(cfg.service.webui.port, 9797);
    }

    #[test]
    fn watchdog_webui_explicit_disabled() {
        let raw = r#"
[feishu]
app_id = "a"
app_secret = "b"

[service.webui]
enabled = false
"#;
        let cfg = Config::parse(raw).expect("config parses");
        assert!(!cfg.service.webui.enabled, "显式 false 应关闭");
    }

    #[test]
    fn watchdog_webui_custom_port() {
        let raw = r#"
[feishu]
app_id = "a"
app_secret = "b"

[service.webui]
port = 9798
"#;
        let cfg = Config::parse(raw).expect("config parses");
        assert!(cfg.service.webui.enabled, "未显式 disabled 应启用");
        assert_eq!(cfg.service.webui.port, 9798);
    }

    #[test]
    fn node_link_defaults_are_off_and_loopback() {
        let cfg = Config::parse("").expect("空配置应解析");
        assert!(!cfg.node_link.enabled, "新的网络入站面必须显式打开");
        assert_eq!(cfg.node_link.listen, "127.0.0.1:9878");
        assert_eq!(cfg.node_link.bootstrap_token_ttl_secs, 900);
        assert!(cfg.node_link.registry_file.is_none());
    }

    #[test]
    fn node_link_section_parses() {
        let raw = r#"
[node_link]
enabled = true
listen = "0.0.0.0:9999"
registry_file = "/var/lib/sebas/nodes.json"
bootstrap_token_ttl_secs = 120
"#;
        let cfg = Config::parse(raw).expect("config parses");
        assert!(cfg.node_link.enabled);
        assert_eq!(cfg.node_link.listen, "0.0.0.0:9999");
        assert_eq!(cfg.node_link.bootstrap_token_ttl_secs, 120);
        assert_eq!(
            cfg.node_link
                .registry_path(std::path::Path::new("/etc/sebas/config.toml")),
            std::path::PathBuf::from("/var/lib/sebas/nodes.json")
        );
    }

    /// single-state-dir 4.2：无配置键时 nodes.json 从**状态目录**派生
    /// （「配置键 > 目录派生 > 默认」中的后两级——目录派生就是状态目录
    /// 解析，未设变量时落在默认 ~/.sebas）。这是唯一发生迁移的落点
    /// （此前默认是 config 文件同目录）。
    #[test]
    fn node_link_registry_defaults_inside_the_state_dir() {
        let pin = tempfile::tempdir().unwrap();
        let saved = std::env::var_os("SEBAS_STATE_DIR");
        unsafe { std::env::set_var("SEBAS_STATE_DIR", pin.path()) };
        let cfg = Config::parse("[node_link]\nenabled = true\n").expect("config parses");
        let derived = cfg.node_link.registry_path(std::path::Path::new("/etc/sebas/config.toml"));
        match &saved {
            Some(v) => unsafe { std::env::set_var("SEBAS_STATE_DIR", v) },
            None => unsafe { std::env::remove_var("SEBAS_STATE_DIR") },
        }
        assert_eq!(
            derived,
            pin.path().join("nodes.json"),
            "缺省落在状态目录（映射表派生）"
        );

        // 未设任何变量时派生落在默认状态目录 ~/.sebas（迁移后的新位置）。
        let saved = std::env::var_os("SEBAS_STATE_DIR");
        unsafe { std::env::remove_var("SEBAS_STATE_DIR") };
        let derived = cfg.node_link.registry_path(std::path::Path::new("/etc/sebas/config.toml"));
        match &saved {
            Some(v) => unsafe { std::env::set_var("SEBAS_STATE_DIR", v) },
            None => unsafe { std::env::remove_var("SEBAS_STATE_DIR") },
        }
        let expected = sebas_domain::state_paths::StatePath::NodeRegistry.resolve();
        assert_eq!(derived, expected, "目录派生与映射表一致");
    }

    #[test]
    fn node_link_bad_listen_is_rejected_before_startup() {
        let err = Config::parse("[node_link]\nenabled = true\nlisten = \"localhost:9878\"\n")
            .expect_err("域名监听地址应在解析期被拒");
        let msg = format!("{err:?}");
        assert!(msg.contains("IP:PORT"), "{msg}");
    }

    #[test]
    fn node_link_disabled_tolerates_any_listen() {
        // 没开就不校验：避免让一个没启用的段把进程拦在启动门外。
        assert!(Config::parse("[node_link]\nlisten = \"weird\"\n").is_ok());
    }
}
