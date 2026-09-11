//! 节点二进制的命令行面。
//!
//! 刻意**不复用**主控 `sebas` 的子命令树：节点是独立二进制（add-remote-execution-node
//! D0），主控的子命令树不因它而变化，节点也不该继承主控的任何人格外壳。

use clap::Parser;
use std::path::PathBuf;

/// sebas 执行节点：在主控以外的机器上运行 agent 会话。
///
/// 节点主动拨号到主控（出站 websocket），因此不需要主控能反向访问它。节点持有
/// 自己会话的执行事实（子进程寿命、有序 turn 日志、审批悬空状态），会话身份与
/// 期望态来自主控。
#[derive(Debug, Clone, Parser)]
#[command(
    name = "sebas-node",
    version,
    about = "sebas 执行节点（独立二进制，不含主控角色）",
    long_about = "sebas 执行节点：在主控以外的机器上运行 agent 会话。\n\n\
                  本二进制只包含运行节点自身会话所需的东西——它不携带主控角色\n\
                  （core / webui / router / im），安装节点不需要安装主控。"
)]
pub struct Cli {
    /// 节点配置文件（TOML）。未给出时只用命令行、环境变量与默认值。
    #[arg(short, long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// 稳定节点标识。重装后沿用同一个 id 可让项目与会话继续指得中；缺省时复用
    /// 已存 id，从未存过则生成一个并落盘。
    #[arg(long, value_name = "ID")]
    pub node_id: Option<String>,

    /// 一次性 join token（首次接入用，在主控侧签发）。给出它会与主控配对并换取
    /// 长期凭据（凭据落盘后后续启动不再需要它）。
    #[arg(long, value_name = "TOKEN")]
    pub join_token: Option<String>,

    /// 主控端点（`ws://` 或 `wss://`），覆盖配置文件里的值。
    #[arg(long, value_name = "URL")]
    pub control_plane: Option<String>,

    /// 状态目录（节点标识、凭据、本地日志），覆盖 `SEBAS_NODE_DIR`。
    #[arg(long, value_name = "DIR")]
    pub state_dir: Option<PathBuf>,

    /// 只解析配置与身份并打印结果，不建立链路（部署自检用）。
    #[arg(long)]
    pub check: bool,
}
