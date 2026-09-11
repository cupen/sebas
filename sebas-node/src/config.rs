//! 节点侧配置。
//!
//! 节点是独立二进制，因此**不复用主控的配置 schema**（`src/config.rs`）：那份配置
//! 描述的是 core / webui / router / im / watchdog 的装配，节点一个都不需要。节点
//! 配置只有一个 `[node]` 段。
//!
//! 取值优先级（高 → 低）：**命令行 > 环境变量 > 配置文件 > 默认值**。环境变量优先于
//! 配置文件是仓库既有约定（同 `SEBAS_FEISHU_APP_ID` 覆盖 TOML），也让沙箱/容器无需
//! 改文件即可把状态目录钉在一次性目录里（AGENTS.md 沙箱规则）。

use crate::cli::Cli;
use crate::error::NodeError;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 状态目录的环境变量覆盖（节点标识、凭据、本地日志都落在这里）。
pub const STATE_DIR_ENV: &str = "SEBAS_NODE_DIR";
/// 主控端点的环境变量覆盖。
pub const CONTROL_PLANE_ENV: &str = "SEBAS_NODE_CONTROL_PLANE";
/// 缺省并发会话上限。
pub const DEFAULT_MAX_SESSIONS: u32 = 8;
/// 缺省本地 turn 日志保留天数（与 webui 归档保留期的缺省口径一致）。
pub const DEFAULT_LOG_RETENTION_DAYS: u32 = 30;
/// 状态目录缺省名（`<data_dir>/sebas-node`）。
const DEFAULT_STATE_DIR_NAME: &str = "sebas-node";

/// 模型流量的上游：节点本地凭据（缺省）或经主控 router 出网（节点零凭据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Upstream {
    /// 节点持自己的 provider 配置与凭据（缺省）。
    #[default]
    Local,
    /// 节点不持任何 provider 凭据，模型流量回主控经 router 出网。
    ControlPlaneRouter,
}

impl Upstream {
    /// 稳定字符串（日志/自检输出用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Upstream::Local => "local",
            Upstream::ControlPlaneRouter => "control-plane-router",
        }
    }
}

/// 配置文件的原始形状。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NodeConfigFile {
    /// `[node]` 段。
    #[serde(default)]
    pub node: NodeSection,
}

/// `[node]` 段。未知键**报错**：这是全新的配置格式，没有历史包袱，打错字应当立刻
/// 被指出来（而不是被静默忽略后以缺省值运行）。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSection {
    /// 稳定节点标识。
    pub id: Option<String>,
    /// 主控端点（`ws://` 或 `wss://`）。
    pub control_plane: Option<String>,
    /// 并发会话上限。
    pub max_sessions: Option<u32>,
    /// 本地 turn 日志保留天数。
    pub log_retention_days: Option<u32>,
    /// 无项目会话的默认工作目录（须为绝对路径）。
    pub default_work_dir: Option<String>,
    /// 状态目录。
    pub state_dir: Option<String>,
    /// 模型流量上游。
    pub upstream: Option<Upstream>,
    /// 本节点配置的 agent 执行体（开放注册表：加一个 kind 只改配置）。
    ///
    /// ```toml
    /// [node.agents.claude]
    /// command = "claude"        # 用于可达性探测；缺省按 kind 名当命令
    /// enforces_mode = false     # 该执行体能否**强制** mode（缺省 false：不假定能做到）
    /// ```
    #[serde(default)]
    pub agents: BTreeMap<String, AgentSection>,
    /// 本节点持有的 provider 名字（**只有名字**；凭据留在节点自己的配置里，
    /// 不上报、不过河）。`upstream = control-plane-router` 时这里应当为空——
    /// 节点不持凭据，如实上报「没有」。
    #[serde(default)]
    pub providers: Vec<String>,
    /// 节点本地的 provider profile（7.1）：一个会话可以按名字选其中一个，
    /// 凭据留在节点上（profile 里只写取凭据的 env 名）。
    ///
    /// ```toml
    /// [node.provider_profiles.anthropic]
    /// protocol = "anthropic"        # 缺省 anthropic
    /// base_url = "https://api.anthropic.com"
    /// api_key_env = "ANTHROPIC_API_KEY"
    /// ```
    #[serde(default)]
    pub provider_profiles: BTreeMap<String, ProviderProfileSection>,
    /// 控制面未指定 provider 时节点应用的默认 profile 名。
    ///
    /// 显式配置而不是「只有一个就用它」：隐式回退会让「控制面没选」与「控制面选了
    /// 这个」在回报里长得一样（7.1 的 desired/effective 差异就没意义了）。
    pub default_provider: Option<String>,
}

/// 一个配置的 agent 执行体。
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSection {
    /// 可执行文件（缺省用 kind 名）。argv[0]；参数走 `args`。
    pub command: Option<String>,
    /// 追加到 argv 的参数。
    #[serde(default)]
    pub args: Vec<String>,
    /// 驱动这个 kind 的运行时。**缺省不驱动**——「命令在 PATH 上」与「本节点真能
    /// 驱动它」是两回事（`manifest` 的诚实原则）。显式写明才算接入了运行时。
    pub driver: Option<AgentDriverKind>,
    /// 该执行体能否强制会话 mode。缺省 `false`：**不假定**它能。
    pub enforces_mode: Option<bool>,
}

/// 驱动 agent 子进程的运行时（6.3）。
///
/// 只有这两种被真正接入；`driver` 缺省即「没接入」——节点不会因为命令存在就假装
/// 能驱动它（那会让清单上的可达性变成一句谎）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentDriverKind {
    /// Claude Code CLI（专用驱动，`PreToolUse` 钩子覆盖每一次工具调用——
    /// 因此它是唯一可以诚实声明 `enforces_mode = true` 的驱动）。
    Claude,
    /// 通用 ACP agent（权限请求由 agent 主动发起；没有全量拦截点，
    /// 因此**不能**声明 `enforces_mode = true`）。
    Acp,
}

impl AgentDriverKind {
    /// 稳定字符串（日志与成因用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentDriverKind::Claude => "claude",
            AgentDriverKind::Acp => "acp",
        }
    }

    /// 该驱动是否有覆盖每一次工具调用的拦截点（决定 `enforces_mode` 能否为真）。
    pub fn has_full_interception_point(&self) -> bool {
        matches!(self, AgentDriverKind::Claude)
    }
}

/// provider 的协议形状（决定注入哪一组 env var）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderProtocol {
    /// Anthropic 协议（`ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`）。
    #[default]
    Anthropic,
    /// OpenAI 协议（`OPENAI_BASE_URL` + `OPENAI_API_KEY`）。
    OpenAi,
}

/// 配置文件里的 provider profile。
///
/// **凭据只以环境变量名出现**：profile 里写的是「去哪个 env 取钥匙」，钥匙本身
/// 留在节点的进程环境里（`NodeConfig` 会进日志/自检输出的 Debug，明文密钥不该
/// 出现在那儿）。
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfileSection {
    /// 协议形状；缺省 Anthropic。
    pub protocol: Option<ProviderProtocol>,
    /// 端点基址。
    pub base_url: Option<String>,
    /// 取凭据的环境变量名（缺省 = 该端点不需要鉴权）。
    pub api_key_env: Option<String>,
}

/// 解析并校验后的 provider profile。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProfile {
    /// profile 名（控制面按它选择）。
    pub name: String,
    /// 协议形状。
    pub protocol: ProviderProtocol,
    /// 端点基址。
    pub base_url: String,
    /// 取凭据的环境变量名（`None` = 不带鉴权）。
    pub api_key_env: Option<String>,
}

/// 解析并校验后的节点配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeConfig {
    /// 稳定节点标识（可能来自文件/命令行，未必已落盘）。
    pub id: Option<String>,
    /// 主控端点，已校验为 `ws://` / `wss://`。
    pub control_plane: String,
    /// 并发会话上限。
    pub max_sessions: u32,
    /// 本地 turn 日志保留天数。
    pub log_retention_days: u32,
    /// 默认工作目录（绝对路径）。
    pub default_work_dir: Option<PathBuf>,
    /// 状态目录（绝对路径或相对路径按原样使用）。
    pub state_dir: PathBuf,
    /// 模型流量上游。
    pub upstream: Upstream,
    /// 已配置的 agent 执行体（kind → 配置）。
    pub agents: BTreeMap<String, AgentSection>,
    /// 节点持有的 provider 名字。
    pub providers: Vec<String>,
    /// 节点本地的 provider profile（名字 → 已校验的定义）。
    pub provider_profiles: BTreeMap<String, ProviderProfile>,
    /// 控制面未指定 provider 时应用的默认 profile 名。
    pub default_provider: Option<String>,
}

impl NodeConfig {
    /// 从 TOML 文本解析（不解析环境变量与命令行）。
    pub fn parse(text: &str) -> Result<NodeConfigFile, NodeError> {
        toml::from_str::<NodeConfigFile>(text)
            .map_err(|e| NodeError::config(format!("配置文件解析失败：{e}")))
    }

    /// 读取配置文件（可选）并与命令行/环境变量/默认值合成。
    pub fn load(config_path: Option<&Path>, cli: &Cli) -> Result<NodeConfig, NodeError> {
        let file = match config_path {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|e| {
                    NodeError::config(format!("无法读取配置文件 {}：{e}", path.display()))
                })?;
                Self::parse(&text)?
            }
            None => NodeConfigFile::default(),
        };
        Self::resolve(file, cli)
    }

    /// 合成：命令行 > 环境变量 > 文件 > 默认值，然后校验。
    pub fn resolve(file: NodeConfigFile, cli: &Cli) -> Result<NodeConfig, NodeError> {
        let section = file.node;

        let id = cli
            .node_id
            .clone()
            .or(section.id)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let control_plane = first_non_empty([
            cli.control_plane.clone(),
            std::env::var(CONTROL_PLANE_ENV).ok(),
            section.control_plane,
        ])
        .ok_or_else(|| {
            NodeError::config(format!(
                "缺少主控端点：[node] control_plane、--control-plane 或 {CONTROL_PLANE_ENV} 至少给一个"
            ))
        })?;

        let config = NodeConfig {
            id,
            control_plane,
            max_sessions: section.max_sessions.unwrap_or(DEFAULT_MAX_SESSIONS),
            log_retention_days: section
                .log_retention_days
                .unwrap_or(DEFAULT_LOG_RETENTION_DAYS),
            default_work_dir: section.default_work_dir.map(PathBuf::from),
            state_dir: resolve_state_dir(cli.state_dir.clone(), section.state_dir)?,
            upstream: section.upstream.unwrap_or_default(),
            agents: section.agents,
            providers: section
                .providers
                .into_iter()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect(),
            provider_profiles: resolve_provider_profiles(section.provider_profiles)?,
            default_provider: section
                .default_provider
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty()),
        };
        config.validate()?;
        Ok(config)
    }

    /// 字段级校验。任何一条不满足都指名**哪个字段、什么取值、为什么**。
    pub fn validate(&self) -> Result<(), NodeError> {
        if !(self.control_plane.starts_with("ws://") || self.control_plane.starts_with("wss://")) {
            return Err(NodeError::config(format!(
                "control_plane {:?} 不是 websocket 端点（须以 ws:// 或 wss:// 开头）",
                self.control_plane
            )));
        }
        if self.max_sessions == 0 {
            return Err(NodeError::config(
                "max_sessions 必须 ≥ 1（并发上限为 0 会让节点拒绝一切会话）",
            ));
        }
        if self.log_retention_days == 0 {
            return Err(NodeError::config(
                "log_retention_days 必须 ≥ 1（保留期为 0 会让本地日志无法被主控同步）",
            ));
        }
        for kind in self.agents.keys() {
            if sebas_node_link::validate_node_id(kind).is_err() {
                return Err(NodeError::config(format!(
                    "agent kind {kind:?} 不是合法标识（只允许 ASCII 字母、数字与 - _ .）"
                )));
            }
        }
        // 「能强制 mode」是有前提的：驱动必须有覆盖每一次工具调用的拦截点。
        // 缺了它却声明 true，就是拿「请求 agent 遵守」冒充「强制」（设计 D6）。
        for (kind, section) in &self.agents {
            if section.enforces_mode == Some(true)
                && !section
                    .driver
                    .is_some_and(|d| d.has_full_interception_point())
            {
                return Err(NodeError::config(format!(
                    "agent {kind:?} 声明 enforces_mode = true，但其 driver 是 {}——没有覆盖每一次工具调用的\
                     拦截点，强制不了；请改用 driver = \"claude\" 或把 enforces_mode 设为 false",
                    section.driver.map(|d| d.as_str()).unwrap_or("未配置")
                )));
            }
        }
        // 默认 profile 必须真存在：指向一个不存在的名字等于每次 spawn 都在运行时
        // 才发现（而且很可能被当成「没配」。）
        if let Some(default) = &self.default_provider
            && !self.provider_profiles.contains_key(default)
        {
            let configured = if self.provider_profiles.is_empty() {
                "（未配置任何 provider profile）".to_string()
            } else {
                self.provider_profiles
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            return Err(NodeError::config(format!(
                "default_provider {default:?} 不是已配置的 provider profile：{configured}"
            )));
        }
        if let Some(dir) = &self.default_work_dir
            && !dir.is_absolute()
        {
            return Err(NodeError::config(format!(
                "default_work_dir {} 必须是绝对路径",
                dir.display()
            )));
        }
        Ok(())
    }
}

impl NodeConfig {
    /// 组装**能力清单**（握手时上报给控制面）。
    ///
    /// 三条诚实原则：
    ///
    /// 1. **可达 = 二进制在 PATH 上 且 本节点真的能驱动它**。只满足前者不算可达——
    ///    否则清单会声称一个我们其实起不来的 agent（「找到命令」与「能干活」是两回事）。
    /// 2. 不可达必须**带成因**（命令缺失 / 运行时未接入），而不是一个光秃秃的 false。
    /// 3. `provideers` 如实：`upstream = control-plane-router` 时节点不持凭据，
    ///    清单里就是空的（不是"暂时没填"）。
    pub fn manifest(&self) -> sebas_node_link::CapabilityManifest {
        use sebas_node_link::{AgentKindCapability, CapabilityManifest, ModeEnforcement};

        let mut agent_kinds: Vec<AgentKindCapability> = vec![AgentKindCapability {
            kind: "echo".into(),
            reachable: true,
            cause: None,
        }];
        let mut mode_enforcement: Vec<ModeEnforcement> = vec![ModeEnforcement {
            execution_body: "echo".into(),
            // echo 与宿主配合：宿主的门控会拦住它请求的受门控动作，因此可以强制。
            enforces_mode: true,
        }];

        for (kind, section) in &self.agents {
            let command = section
                .command
                .clone()
                .unwrap_or_else(|| kind.clone());
            let (reachable, cause) = match (probe_command(&command), section.driver) {
                // 命令不在 → 不可达，成因指名命令。
                (false, _) => (false, Some(format!("命令 {command:?} 不在 PATH 上"))),
                // 命令在但没接运行时 → 仍不可达：找到命令与能干活是两回事。
                (true, None) => (
                    false,
                    Some(format!(
                        "命令 {command:?} 在 PATH 上，但该 kind 未配置 driver（运行时尚未接入）"
                    )),
                ),
                // 命令在且运行时已接入 → 可达；真正能不能握上手由 spawn 如实回答。
                (true, Some(_)) => (true, None),
            };
            agent_kinds.push(AgentKindCapability {
                kind: kind.clone(),
                reachable,
                cause,
            });
            mode_enforcement.push(ModeEnforcement {
                execution_body: kind.clone(),
                // 配置说了才算；缺省不假定它做得到。
                enforces_mode: section.enforces_mode.unwrap_or(false),
            });
        }

        // provider 清单 = `providers` ∪ profile 名。写 profile 就等于持有了一个
        // provider，漏报它会让控制面以为节点没有这个可选。
        let mut providers: Vec<String> = match self.upstream {
            // 经主控 router 出网 = 节点零凭据，清单如实为空。
            Upstream::ControlPlaneRouter => Vec::new(),
            Upstream::Local => {
                let mut names = self.providers.clone();
                for name in self.provider_profiles.keys() {
                    if !names.contains(name) {
                        names.push(name.clone());
                    }
                }
                names
            }
        };
        providers.sort();

        CapabilityManifest {
            agent_kinds,
            providers,
            mode_enforcement,
        }
    }

    /// 组装执行体工厂需要的节点侧配置（6.3 / 7.1 / 7.2）。
    pub fn body_config(&self) -> NodeBodyConfig {
        NodeBodyConfig {
            agents: self.agents.clone(),
            provider_profiles: self.provider_profiles.clone(),
            default_provider: self.default_provider.clone(),
            upstream: self.upstream,
            default_work_dir: self.default_work_dir.clone(),
        }
    }
}

/// 执行体工厂需要的节点侧配置（与 [`NodeConfig`] 分开，便于单测直接构造）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeBodyConfig {
    /// agent kind → 配置。
    pub agents: BTreeMap<String, AgentSection>,
    /// provider profile（名字 → 定义）。
    pub provider_profiles: BTreeMap<String, ProviderProfile>,
    /// 控制面未指定 provider 时的默认 profile。
    pub default_provider: Option<String>,
    /// 模型流量上游。
    pub upstream: Upstream,
    /// 无项目会话的默认工作目录。
    pub default_work_dir: Option<PathBuf>,
}

impl NodeBodyConfig {
    /// 某 kind 的配置（未配置 → `None`）。
    pub fn agent(&self, kind: &str) -> Option<&AgentSection> {
        self.agents.get(kind)
    }

    /// 某 profile（未配置 → `None`）。
    pub fn profile(&self, name: &str) -> Option<&ProviderProfile> {
        self.provider_profiles.get(name)
    }
}

/// 把文件里的 profile 段翻译成已校验的定义。任何一条不合法都指名**哪个 profile、
/// 哪个字段、什么取值**。
fn resolve_provider_profiles(
    raw: BTreeMap<String, ProviderProfileSection>,
) -> Result<BTreeMap<String, ProviderProfile>, NodeError> {
    let mut out = BTreeMap::new();
    for (name, section) in raw {
        if sebas_node_link::validate_node_id(&name).is_err() {
            return Err(NodeError::config(format!(
                "provider profile 名 {name:?} 不是合法标识（只允许 ASCII 字母、数字与 - _ .）"
            )));
        }
        let base_url = section.base_url.unwrap_or_default().trim().to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(NodeError::config(format!(
                "provider profile {name:?} 的 base_url {base_url:?} 不是 http(s) 端点"
            )));
        }
        let api_key_env = section
            .api_key_env
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        out.insert(
            name.clone(),
            ProviderProfile {
                name,
                protocol: section.protocol.unwrap_or_default(),
                base_url,
                api_key_env,
            },
        );
    }
    Ok(out)
}

/// 命令是否在 PATH 上（含显式路径）。
///
/// 只做存在性与可执行位判断，**不执行**它——清单构建发生在握手路径上，不该在这里
/// 起子进程（既慢又给远端一个"让节点执行任意命令"的入口）。
fn probe_command(command: &str) -> bool {
    if command.trim().is_empty() {
        return false;
    }
    let path = std::path::Path::new(command);
    if command.contains('/') || command.contains('\\') {
        return is_executable_file(path);
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| is_executable_file(&dir.join(command)))
}

#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}

/// 状态目录：`--state-dir` > `SEBAS_NODE_DIR` > `[node] state_dir` > `<data_dir>/sebas-node`。
fn resolve_state_dir(cli: Option<PathBuf>, from_file: Option<String>) -> Result<PathBuf, NodeError> {
    if let Some(dir) = cli {
        return Ok(dir);
    }
    if let Some(dir) = non_empty(std::env::var(STATE_DIR_ENV).ok()) {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = non_empty(from_file) {
        return Ok(PathBuf::from(dir));
    }
    let data = dirs::data_dir().ok_or_else(|| {
        NodeError::state_dir(format!(
            "无法推断数据目录；请显式设置 --state-dir 或 {STATE_DIR_ENV}"
        ))
    })?;
    Ok(data.join(DEFAULT_STATE_DIR_NAME))
}

/// 取第一个非空（trim 后）的候选。
fn first_non_empty(candidates: [Option<String>; 3]) -> Option<String> {
    candidates.into_iter().find_map(non_empty)
}

fn non_empty(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// env 是进程全局的：并行用例互相污染，用互斥锁串行化（同 startup crate 模式）。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn cli_with(state_dir: &Path) -> Cli {
        Cli {
            config: None,
            join_token: None,
            node_id: None,
            control_plane: None,
            state_dir: Some(state_dir.to_path_buf()),
            check: false,
        }
    }

    fn base_cli() -> Cli {
        Cli {
            config: None,
            join_token: None,
            node_id: None,
            control_plane: Some("wss://control.example/ws".into()),
            state_dir: Some(PathBuf::from("/tmp/sebas-node-test")),
            check: false,
        }
    }

    #[test]
    fn defaults_are_applied() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let cfg = NodeConfig::resolve(NodeConfigFile::default(), &base_cli()).unwrap();
        assert_eq!(cfg.max_sessions, DEFAULT_MAX_SESSIONS);
        assert_eq!(cfg.log_retention_days, DEFAULT_LOG_RETENTION_DAYS);
        assert_eq!(cfg.upstream, Upstream::Local);
        assert_eq!(cfg.default_work_dir, None);
        assert_eq!(cfg.id, None);
    }

    #[test]
    fn file_values_are_read() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
id = "dev-box"
control_plane = "wss://cp.example/ws"
max_sessions = 3
log_retention_days = 7
default_work_dir = "/srv/work"
state_dir = "/var/lib/sebas-node"
upstream = "control-plane-router"
"#;
        let file = NodeConfig::parse(text).unwrap();
        let cli = Cli {
            config: None,
            join_token: None,
            node_id: None,
            control_plane: None,
            state_dir: None,
            check: false,
        };
        let cfg = NodeConfig::resolve(file, &cli).unwrap();
        assert_eq!(cfg.id.as_deref(), Some("dev-box"));
        assert_eq!(cfg.control_plane, "wss://cp.example/ws");
        assert_eq!(cfg.max_sessions, 3);
        assert_eq!(cfg.log_retention_days, 7);
        assert_eq!(cfg.default_work_dir, Some(PathBuf::from("/srv/work")));
        assert_eq!(cfg.state_dir, PathBuf::from("/var/lib/sebas-node"));
        assert_eq!(cfg.upstream, Upstream::ControlPlaneRouter);
    }

    #[test]
    fn cli_beats_file() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
control_plane = "wss://from-file/ws"
state_dir = "/from/file"
id = "file-id"
"#;
        let file = NodeConfig::parse(text).unwrap();
        let cli = Cli {
            config: None,
            join_token: None,
            node_id: Some("cli-id".into()),
            control_plane: Some("wss://from-cli/ws".into()),
            state_dir: Some(PathBuf::from("/from/cli")),
            check: false,
        };
        let cfg = NodeConfig::resolve(file, &cli).unwrap();
        assert_eq!(cfg.control_plane, "wss://from-cli/ws");
        assert_eq!(cfg.state_dir, PathBuf::from("/from/cli"));
        assert_eq!(cfg.id.as_deref(), Some("cli-id"));
    }

    #[test]
    fn env_beats_file_but_loses_to_cli() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
control_plane = "wss://from-file/ws"
state_dir = "/from/file"
"#;
        let file = NodeConfig::parse(text).unwrap();
        // 文件 + env：env 赢。
        let cli = Cli {
            config: None,
            join_token: None,
            node_id: None,
            control_plane: None,
            state_dir: None,
            check: false,
        };
        unsafe {
            std::env::set_var(CONTROL_PLANE_ENV, "wss://from-env/ws");
            std::env::set_var(STATE_DIR_ENV, "/from/env");
        }
        let cfg = NodeConfig::resolve(file.clone(), &cli).unwrap();
        assert_eq!(cfg.control_plane, "wss://from-env/ws");
        assert_eq!(cfg.state_dir, PathBuf::from("/from/env"));

        // 文件 + env + cli：cli 赢。
        let cli = Cli {
            config: None,
            join_token: None,
            node_id: None,
            control_plane: Some("wss://from-cli/ws".into()),
            state_dir: Some(PathBuf::from("/from/cli")),
            check: false,
        };
        let cfg = NodeConfig::resolve(file, &cli).unwrap();
        assert_eq!(cfg.control_plane, "wss://from-cli/ws");
        assert_eq!(cfg.state_dir, PathBuf::from("/from/cli"));

        unsafe {
            std::env::remove_var(CONTROL_PLANE_ENV);
            std::env::remove_var(STATE_DIR_ENV);
        }
    }

    #[test]
    fn unknown_key_is_rejected() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let err = NodeConfig::parse("[node]\nnod_id = \"typo\"\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nod_id"), "报错须指名打错的键：{msg}");
    }

    #[test]
    fn missing_control_plane_is_rejected() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var(CONTROL_PLANE_ENV) };
        let cli = Cli {
            config: None,
            join_token: None,
            node_id: None,
            control_plane: None,
            state_dir: Some(PathBuf::from("/tmp/x")),
            check: false,
        };
        let err = NodeConfig::resolve(NodeConfigFile::default(), &cli).unwrap_err();
        assert!(err.to_string().contains("缺少主控端点"), "{err}");
    }

    #[test]
    fn non_websocket_endpoint_is_rejected() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let cli = Cli {
            config: None,
            join_token: None,
            node_id: None,
            control_plane: Some("https://cp.example".into()),
            state_dir: Some(PathBuf::from("/tmp/x")),
            check: false,
        };
        let err = NodeConfig::resolve(NodeConfigFile::default(), &cli).unwrap_err();
        assert!(err.to_string().contains("ws://"), "{err}");
    }

    #[test]
    fn zero_limits_are_rejected() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let text = "[node]\ncontrol_plane = \"wss://c/ws\"\nmax_sessions = 0\n";
        let err = NodeConfig::resolve(NodeConfig::parse(text).unwrap(), &cli_with(Path::new("/tmp/x")))
            .unwrap_err();
        assert!(err.to_string().contains("max_sessions"), "{err}");

        let text = "[node]\ncontrol_plane = \"wss://c/ws\"\nlog_retention_days = 0\n";
        let err = NodeConfig::resolve(NodeConfig::parse(text).unwrap(), &cli_with(Path::new("/tmp/x")))
            .unwrap_err();
        assert!(err.to_string().contains("log_retention_days"), "{err}");
    }

    #[test]
    fn relative_default_work_dir_is_rejected() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let text = "[node]\ncontrol_plane = \"wss://c/ws\"\ndefault_work_dir = \"work\"\n";
        let err = NodeConfig::resolve(NodeConfig::parse(text).unwrap(), &cli_with(Path::new("/tmp/x")))
            .unwrap_err();
        assert!(err.to_string().contains("绝对路径"), "{err}");
    }

    #[test]
    fn missing_config_file_is_reported_with_path() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let err = NodeConfig::load(Some(Path::new("/nonexistent/sebas-node.toml")), &base_cli())
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("sebas-node.toml"), "{msg}");
    }

    #[test]
    fn config_file_round_trips_through_load() {
        // env 是进程全局的：凡是走 resolve 的用例都必须串行化，
        // 否则别的用例设置的 SEBAS_NODE_* 会漏进来（测试污染）。
        let _env = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.toml");
        std::fs::write(&path, "[node]\ncontrol_plane = \"wss://c/ws\"\nmax_sessions = 2\n").unwrap();
        // CLI 不覆盖 control_plane，才能断言文件里的值生效（CLI > 文件）。
        let cli = Cli {
            config: None,
            join_token: None,
            node_id: None,
            control_plane: None,
            state_dir: Some(dir.path().join("state")),
            check: false,
        };
        let cfg = NodeConfig::load(Some(&path), &cli).unwrap();
        assert_eq!(cfg.control_plane, "wss://c/ws");
        assert_eq!(cfg.max_sessions, 2);
    }
    // ── agent 注册表 / provider 清单 / 能力清单（2.4 / 7.1）────────────────

    #[test]
    fn agents_and_providers_parse() {
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
control_plane = "wss://c/ws"
providers = ["anthropic", "  deepseek  ", ""]

[node.agents.claude]
command = "claude"
driver = "claude"
enforces_mode = true

[node.agents.gemini]
driver = "acp"
args = ["--acp"]
"#;
        let cfg = NodeConfig::resolve(
            NodeConfig::parse(text).unwrap(),
            &cli_with(Path::new("/tmp/x")),
        )
        .unwrap();
        assert_eq!(
            cfg.providers,
            vec!["anthropic".to_string(), "deepseek".to_string()],
            "去空白、去空项"
        );
        assert_eq!(cfg.agents.len(), 2);
        assert_eq!(cfg.agents["claude"].command.as_deref(), Some("claude"));
        assert_eq!(cfg.agents["claude"].driver, Some(AgentDriverKind::Claude));
        assert_eq!(cfg.agents["gemini"].args, vec!["--acp".to_string()]);
        assert_eq!(cfg.agents["gemini"].driver, Some(AgentDriverKind::Acp));
    }

    #[test]
    fn claiming_mode_enforcement_without_an_interception_point_is_rejected() {
        // 「请求 agent 遵守」冒充「强制」正是设计 D6 要点名的撒谎形态。
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
control_plane = "wss://c/ws"
[node.agents.gemini]
driver = "acp"
enforces_mode = true
"#;
        let err = NodeConfig::resolve(
            NodeConfig::parse(text).unwrap(),
            &cli_with(Path::new("/tmp/x")),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("gemini"), "{msg}");
        assert!(msg.contains("enforces_mode"), "{msg}");
        assert!(msg.contains("acp"), "成因须指名 driver：{msg}");
    }

    #[test]
    fn provider_profiles_parse_and_must_name_a_real_endpoint() {
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
control_plane = "wss://c/ws"
default_provider = "anthropic"

[node.provider_profiles.anthropic]
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"

[node.provider_profiles.local-proxy]
protocol = "openai"
base_url = "http://127.0.0.1:8080/v1"
"#;
        let cfg = NodeConfig::resolve(
            NodeConfig::parse(text).unwrap(),
            &cli_with(Path::new("/tmp/x")),
        )
        .unwrap();
        assert_eq!(cfg.default_provider.as_deref(), Some("anthropic"));
        let anthropic = &cfg.provider_profiles["anthropic"];
        assert_eq!(anthropic.protocol, ProviderProtocol::Anthropic);
        assert_eq!(anthropic.base_url, "https://api.anthropic.com");
        assert_eq!(anthropic.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
        let proxy = &cfg.provider_profiles["local-proxy"];
        assert_eq!(proxy.protocol, ProviderProtocol::OpenAi);
        assert!(proxy.api_key_env.is_none(), "不写 env 名 = 该端点不带鉴权");

        // 清单要如实包含 profile 名（写 profile 就是持有了一个 provider）。
        let manifest = cfg.manifest();
        assert!(manifest.providers.contains(&"anthropic".to_string()));
        assert!(manifest.providers.contains(&"local-proxy".to_string()));
    }

    #[test]
    fn a_profile_without_an_http_endpoint_is_rejected() {
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
control_plane = "wss://c/ws"
[node.provider_profiles.anthropic]
base_url = "api.anthropic.com"
"#;
        let err = NodeConfig::resolve(
            NodeConfig::parse(text).unwrap(),
            &cli_with(Path::new("/tmp/x")),
        )
        .unwrap_err();
        assert!(err.to_string().contains("anthropic"), "{err}");
        assert!(err.to_string().contains("http"), "{err}");
    }

    #[test]
    fn a_default_provider_that_does_not_exist_is_rejected() {
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
control_plane = "wss://c/ws"
default_provider = "ghost"
[node.provider_profiles.anthropic]
base_url = "https://api.anthropic.com"
"#;
        let err = NodeConfig::resolve(
            NodeConfig::parse(text).unwrap(),
            &cli_with(Path::new("/tmp/x")),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ghost"), "{msg}");
        assert!(msg.contains("anthropic"), "成因须列出已配置的：{msg}");
    }

    #[test]
    fn an_illegal_agent_kind_is_rejected() {
        let _env = ENV_LOCK.lock().unwrap();
        let text = "[node]\ncontrol_plane = \"wss://c/ws\"\n[node.agents.\"bad kind\"]\n";
        let err = NodeConfig::resolve(
            NodeConfig::parse(text).unwrap(),
            &cli_with(Path::new("/tmp/x")),
        )
        .unwrap_err();
        assert!(err.to_string().contains("bad kind"), "{err}");
    }

    #[test]
    fn the_manifest_reports_reachability_with_causes() {
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
control_plane = "wss://c/ws"
providers = ["anthropic"]

[node.agents.sh]

[node.agents.definitely-not-a-real-command-xyz]

[node.agents.gemini]
driver = "acp"
"#;
        let cfg = NodeConfig::resolve(
            NodeConfig::parse(text).unwrap(),
            &cli_with(Path::new("/tmp/x")),
        )
        .unwrap();
        let manifest = cfg.manifest();

        let echo = manifest
            .agent_kinds
            .iter()
            .find(|k| k.kind == "echo")
            .expect("宿主自带的 echo 必须在清单里");
        assert!(echo.reachable, "echo 永远可达");
        assert!(echo.cause.is_none());

        let sh = manifest.agent_kinds.iter().find(|k| k.kind == "sh").unwrap();
        assert!(!sh.reachable, "只有命令在不算可达");
        let cause = sh.cause.as_deref().unwrap();
        assert!(cause.contains("运行时尚未接入"), "{cause}");

        // driver 已接入且命令在 → 可达（真正能不能握上手由 spawn 如实回答）。
        let gemini = manifest
            .agent_kinds
            .iter()
            .find(|k| k.kind == "gemini")
            .unwrap();
        assert!(gemini.reachable, "{:?}", gemini.cause);
        assert!(gemini.cause.is_none());

        let bogus = manifest
            .agent_kinds
            .iter()
            .find(|k| k.kind.starts_with("definitely"))
            .unwrap();
        assert!(!bogus.reachable);
        assert!(
            bogus.cause.as_deref().unwrap().contains("不在 PATH 上"),
            "{:?}",
            bogus.cause
        );

        let unproven = manifest
            .mode_enforcement
            .iter()
            .find(|e| e.execution_body.starts_with("definitely"))
            .unwrap();
        assert!(!unproven.enforces_mode, "缺省不假定能强制 mode");
    }

    #[test]
    fn the_manifest_hides_providers_when_routed_through_the_control_plane() {
        let _env = ENV_LOCK.lock().unwrap();
        let text = r#"
[node]
control_plane = "wss://c/ws"
upstream = "control-plane-router"
providers = ["anthropic", "deepseek"]
"#;
        let cfg = NodeConfig::resolve(
            NodeConfig::parse(text).unwrap(),
            &cli_with(Path::new("/tmp/x")),
        )
        .unwrap();
        assert_eq!(cfg.upstream, Upstream::ControlPlaneRouter);
        assert!(
            cfg.manifest().providers.is_empty(),
            "经主控 router 出网 = 节点零凭据，清单如实为空"
        );
    }
}
