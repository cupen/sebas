//! `sebas node-link`：节点链路的管理入口（add-remote-execution-node 2.7）。
//!
//! **必须经 core session channel**，不能直接读写注册表文件：注册表是**单写者**
//! 文件，只有 core 进程持有那一份内存状态；另一个进程写同一个文件会与它互相
//! 覆盖（后者会把前者刚写的节点/令牌抹掉）。
//!
//! 三个操作：签发一次性配对 token、列出已注册节点、吊销节点凭据。core 侧未启用
//! 节点链路时如实回 [`NodeLinkOutcome::Disabled`]，这里翻成非零退出并说明成因，
//! 不假装成功。

use crate::config::Config;
use crate::core_channel::protocol::{NodeLinkOp, NodeLinkOutcome};
use crate::error::SebasError;

/// 管理命令参数（lib 侧形态；`main.rs` 从 clap 类型转换而来，与 `im_cmd` 同款）。
#[derive(Debug, Clone)]
pub struct Args {
    /// 主控配置文件（用于发现 core socket 与 secret）。
    pub config: String,
    /// 要执行的管理操作。
    pub cmd: Cmd,
}

/// 管理操作。
#[derive(Debug, Clone)]
pub enum Cmd {
    /// 签发一次性配对 token（`ttl_secs` 为 `None` 时由 core 决定缺省有效期）。
    Token { ttl_secs: Option<u64> },
    /// 列出已注册节点。
    List,
    /// 吊销节点凭据。
    Revoke { node_id: String },
}

/// 执行管理命令。
pub async fn run(args: Args) -> Result<(), SebasError> {
    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Config(format!("read config {}: {e}", args.config)))?;
    let cfg = Config::parse(&raw)?;

    // 与 webui / im / router 订阅同一套 secret 解析：env 优先，缺失时按同一份
    // config 发现 core 自动武装落盘的 secret 文件。
    let secret_file = crate::config::core_secret_file_path(
        cfg.watchdog.core.secret_file.as_deref(),
        std::path::Path::new(&args.config),
    );
    let backend = crate::core_channel::client::CoreChannelBackend::with_secret(
        crate::core_channel::socket_path(&cfg),
        crate::core_channel::secret::ChannelSecret::from_env_or_file(Some(secret_file)),
    );

    let op = match args.cmd {
        Cmd::Token { ttl_secs } => NodeLinkOp::IssueJoinToken { ttl_secs },
        Cmd::List => NodeLinkOp::ListNodes,
        Cmd::Revoke { node_id } => NodeLinkOp::RevokeNode { node_id },
    };

    let outcome = backend
        .node_link(op)
        .await
        .map_err(|r| SebasError::Config(format!("core 会话通道不可用或拒绝：{r:?}")))?;

    match outcome {
        NodeLinkOutcome::JoinToken {
            token,
            expires_unix,
        } => {
            // token 打到 stdout（机器可读、便于 `$(...)` 取用）；指引打 stderr。
            println!("{token}");
            eprintln!("配对 token（只显示这一次，{expires_unix} 到期）。在节点机上执行：");
            eprintln!(
                "  sebas-node --node-id <节点标识> --control-plane ws://<主控地址> --join-token {token}"
            );
            Ok(())
        }
        NodeLinkOutcome::Nodes { nodes } => {
            if nodes.is_empty() {
                eprintln!("（还没有已注册的节点）");
                return Ok(());
            }
            println!("{:<24} {:<10} {:>12}  last seen", "NODE", "STATUS", "PAIRED AT");
            for n in nodes {
                println!(
                    "{:<24} {:<10} {:>12}  {}",
                    n.id,
                    n.status,
                    n.created_unix,
                    n.last_seen_unix
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "-".into())
                );
            }
            Ok(())
        }
        NodeLinkOutcome::Revoked { node_id, found } => {
            if found {
                eprintln!("已吊销节点 {node_id}：其凭据立即失效，且不能用同一 id 重新配对绕过。");
                Ok(())
            } else {
                Err(SebasError::Config(format!(
                    "没有名为 {node_id} 的已注册节点（未做任何改动）"
                )))
            }
        }
        NodeLinkOutcome::Disabled { cause } => Err(SebasError::Config(format!(
            "节点链路未启用：{cause}（在 config 里设 [node_link] enabled = true 并重启 core）"
        ))),
        NodeLinkOutcome::Failed { cause } => {
            Err(SebasError::Config(format!("管理操作失败：{cause}")))
        }
    }
}
