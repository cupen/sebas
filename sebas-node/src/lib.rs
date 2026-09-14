//! sebas **执行节点**：在主控以外的机器上运行 agent 会话的独立二进制。
//!
//! 与主控的关系（add-remote-execution-node）：
//!
//! - 节点**出站**拨号到主控（websocket），不需要主控能反向访问它；
//! - 权威按域切分：主控持会话身份与期望态，节点持**执行事实**（子进程寿命、
//!   有序 turn 日志、审批悬空状态）；
//! - 节点在任何情况下都**不自我放行**权限：放行只有一个来源——主控。
//!
//! 二进制纪律（D0）：本 crate **不含主控角色**（core / webui / router / im）。
//! 安装节点不需要安装主控；节点机上也不存在"能起一个 core"的可能。
//!
//! 启动分两段：[`startup`] 是同步的解析段（配置 → 身份 → 凭据就位性检查），
//! [`link::LinkClient`] 是异步的长驻段（拨号 / 握手 / 退避重连）。分开是为了让
//! 解析段可以被单测与 `--check` 直接驱动，而不必起网络。

pub mod body;
pub mod cli;
pub mod config;
pub mod error;
pub mod identity;
pub mod link;
pub mod log;
pub mod materials;
pub mod session;

pub use cli::Cli;
pub use config::{NodeBodyConfig, NodeConfig, Upstream};
pub use error::NodeError;
pub use identity::{IdentityStore, NodeId};

use std::path::PathBuf;

/// env 是进程全局的：动 `SEBAS_*` 的测试用例（config 与启动装配）共用这把锁
/// 串行化，防止跨模块的并行用例互相污染。
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 启动解析段的结果。
///
/// `checked_only` 为真表示只做了自检（`--check`），不要进入链路。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Startup {
    /// 本次生效的节点标识。
    pub node_id: NodeId,
    /// 主控端点。
    pub control_plane: String,
    /// 状态目录。
    pub state_dir: PathBuf,
    /// 模型流量上游。
    pub upstream: Upstream,
    /// 并发会话上限。
    pub max_sessions: u32,
    /// 本地日志保留天数。
    pub log_retention_days: u32,
    /// 默认工作目录（无项目会话的落脚点）。
    pub default_work_dir: Option<PathBuf>,
    /// 生效的 workspace root（add-workspace-root）：显式配置，或未配置时回退的
    /// 进程 cwd。路径判定（`CheckPath`）以它为界内/越界的边界。
    pub workspace_root: PathBuf,
    /// workspace root 是否为**回退值**（env 与 `[node] workspace_root` 都没有）。
    /// 启动装配处据此打一次告警；判定路径上不再打日志。
    pub workspace_root_fell_back: bool,
    /// 是否只做自检。
    pub checked_only: bool,
    /// 握手要上报的能力清单（agent kinds + 可达性 + provider 清单 + mode 强制能力）。
    pub manifest: sebas_node_link::CapabilityManifest,
    /// 执行体工厂需要的节点侧配置（agent 运行时、provider profile、默认工作目录）。
    pub body: NodeBodyConfig,
}

impl Startup {
    /// 人类可读的自检输出（`--check` 用，一行一条）。
    pub fn check_report(&self) -> String {
        let work_dir = self
            .default_work_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(未设置：无项目会话将无法落脚)".to_string());
        let workspace = if self.workspace_root_fell_back {
            format!(
                "{}（回退 cwd；建议显式配置 {} 或 [node] workspace_root）",
                self.workspace_root.display(),
                config::WORKSPACE_ROOT_ENV
            )
        } else {
            self.workspace_root.display().to_string()
        };
        format!(
            "node id:        {}\n\
             control plane:  {}\n\
             state dir:      {}\n\
             upstream:       {}\n\
             max sessions:   {}\n\
             log retention:  {} 天\n\
             default work:   {}\n\
             workspace root: {}",
            self.node_id,
            self.control_plane,
            self.state_dir.display(),
            self.upstream.as_str(),
            self.max_sessions,
            self.log_retention_days,
            work_dir,
            workspace,
        )
    }
}

/// 启动解析段：**配置 → 身份 → 凭据就位性**。
///
/// 返回 `Ok` 只意味着「可以进入链路」，不意味着已经连上——链路是否可达由
/// [`link::LinkClient`] 如实汇报。任何一步不成立都返回带成因的 [`NodeError`]。
pub fn startup(cli: &Cli) -> Result<Startup, NodeError> {
    let config = NodeConfig::load(cli.config.as_deref(), cli)?;
    let store = IdentityStore::new(config.state_dir.clone());
    let node_id = store.load_or_create_id(config.id.as_deref())?;

    // workspace root 的 cwd 回退发生在**启动装配处**（这里）而不是判定函数：
    // 回退是一次性的启动期事实，配一条告警；`resolve_workspace_root` 与主控
    // `src/config.rs` 同名函数同语义（add-workspace-root）。
    let cwd = std::env::current_dir()
        .map_err(|e| NodeError::config(format!("无法取进程 cwd 作为 workspace root 回退：{e}")))?;
    let (workspace_root, workspace_root_fell_back) =
        config::resolve_workspace_root(config.workspace_root.clone(), &cwd);

    let made = Startup {
        node_id,
        control_plane: config.control_plane.clone(),
        state_dir: config.state_dir.clone(),
        upstream: config.upstream,
        max_sessions: config.max_sessions,
        log_retention_days: config.log_retention_days,
        default_work_dir: config.default_work_dir.clone(),
        workspace_root,
        workspace_root_fell_back,
        checked_only: cli.check,
        manifest: config.manifest(),
        body: config.body_config(),
    };

    if cli.check {
        return Ok(made);
    }

    // 首次接入必须带一次性 join token；已配对则可凭凭据直接接入。
    // 两者都没有时如实失败，不进入"半工作"状态（不伪装受理）。
    if cli.join_token.is_none() && !store.has_credential() {
        return Err(NodeError::unpaired(format!(
            "节点 {} 的状态目录 {} 里没有凭据，且未提供 --join-token；\
             首次接入需要先在主控侧签发一次性 join token",
            made.node_id,
            made.state_dir.display()
        )));
    }

    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn cli(state_dir: &Path, check: bool) -> Cli {
        Cli {
            config: None,
            node_id: Some("test-node".into()),
            join_token: None,
            control_plane: Some("wss://control.example/ws".into()),
            state_dir: Some(state_dir.to_path_buf()),
            check,
        }
    }

    fn cli_with_token(state_dir: &Path) -> Cli {
        Cli {
            join_token: Some("join-token-abc".into()),
            ..cli(state_dir, false)
        }
    }

    #[test]
    fn check_reports_resolved_startup_without_linking() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let started = startup(&cli(&state, true)).unwrap();
        assert!(started.checked_only);
        assert_eq!(started.node_id.as_str(), "test-node");
        assert_eq!(started.control_plane, "wss://control.example/ws");
        assert_eq!(started.upstream, Upstream::Local);
        assert_eq!(started.max_sessions, config::DEFAULT_MAX_SESSIONS);
        assert_eq!(
            started.log_retention_days,
            config::DEFAULT_LOG_RETENTION_DAYS
        );
        // 自检不应因为"未配对"而失败——它只回答"配置与身份解析成什么"。
        let report = started.check_report();
        assert!(report.contains("test-node"), "{report}");
        assert!(report.contains("wss://control.example/ws"), "{report}");
    }

    #[test]
    fn unpaired_node_without_token_fails_with_a_named_cause() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let err = startup(&cli(&state, false)).unwrap_err();
        match err {
            NodeError::Unpaired { cause } => {
                assert!(cause.contains("test-node"), "{cause}");
                assert!(cause.contains("--join-token"), "{cause}");
            }
            other => panic!("未配对应报 Unpaired，实际：{other:?}"),
        }
    }

    #[test]
    fn join_token_allows_startup_without_a_credential() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let started = startup(&cli_with_token(&state)).unwrap();
        assert_eq!(started.node_id.as_str(), "test-node");
        assert!(!started.checked_only);
    }

    #[test]
    fn paired_node_starts_without_a_token() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let store = IdentityStore::new(&state);
        store.save_credential("paired-secret").unwrap();
        let started = startup(&cli(&state, false)).unwrap();
        assert_eq!(started.node_id.as_str(), "test-node");
    }

    #[test]
    fn identity_is_created_during_startup_and_reused_afterwards() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let first = startup(&cli(&state, true)).unwrap().node_id;
        let second = startup(&cli(&state, true)).unwrap().node_id;
        assert_eq!(first, second);
        assert!(state.join(identity::NODE_ID_FILE).exists());
    }

    // ── workspace root 装配（add-workspace-root）───────────────────────────

    #[test]
    fn workspace_root_falls_back_to_cwd_with_a_flag_when_unconfigured() {
        let _env = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var(config::WORKSPACE_ROOT_ENV) };
        let tmp = tempfile::tempdir().unwrap();
        let started = startup(&cli(&tmp.path().join("state"), true)).unwrap();
        assert!(started.workspace_root_fell_back, "env 与配置都没有 → 回退");
        assert_eq!(started.workspace_root, std::env::current_dir().unwrap());
        // 自检报告要点名回退与显式配置建议（--check 也要能看见）。
        let report = started.check_report();
        assert!(report.contains("workspace root"), "{report}");
        assert!(report.contains("回退 cwd"), "{report}");
        assert!(report.contains(config::WORKSPACE_ROOT_ENV), "{report}");
    }

    #[test]
    fn an_explicit_workspace_root_reaches_startup_without_fallback() {
        let _env = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var(config::WORKSPACE_ROOT_ENV) };
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("node.toml");
        // TOML 字面量字符串（单引号）：Windows 路径的反斜杠不能走基本字符串转义。
        std::fs::write(
            &config_path,
            format!(
                "[node]\ncontrol_plane = \"wss://c/ws\"\nworkspace_root = '{}'\n",
                tmp.path().display()
            ),
        )
        .unwrap();
        let c = Cli {
            config: Some(config_path),
            ..cli(&tmp.path().join("state"), true)
        };
        let started = startup(&c).unwrap();
        assert!(!started.workspace_root_fell_back);
        assert_eq!(started.workspace_root, tmp.path());
    }
}
