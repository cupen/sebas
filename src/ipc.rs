//! Pipe IPC 协议：watchdog（父进程）与 sebas core（子进程）之间仅剩 readiness 握手。
//!
//! 子进程启动完成后向 stdout 写一行 `{"cmd":"ready"}`；父进程读到即认为就绪。
//! 控制操作（升级 / 回滚 / 重启 / 服务管理）一律走 control RPC Unix socket，
//! 不再经过管道。
//!
//! 子进程存活检测由父进程对 child.wait() 的等待承担，管道不承载生命周期语义
//! 之外的任何信息。

use crate::error::{Result, SebasError};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};

// ─── 消息类型 ────────────────────────────────────────────

/// 子进程发给父进程的命令（仅剩 readiness 握手）
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd")]
pub enum ChildMsg {
    #[serde(rename = "ready")]
    Ready,
}

// ─── 父进程侧 IPC 句柄 ────────────────────────────────────

pub struct ParentIpc {
    reader: BufReader<ChildStdout>,
    /// 持有子进程 stdin 的写端：一旦 drop，子进程 stdin 立刻收到 EOF。
    /// 协议已收缩为 Ready-only，父进程不再写管道，但句柄必须活到监督结束。
    _stdin: ChildStdin,
}

impl ParentIpc {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            reader: BufReader::new(stdout),
            _stdin: stdin,
        }
    }

    /// 读取子进程发来的下一条命令（实际只有 Ready；EOF/解析错误返回 Err）
    pub async fn recv(&mut self) -> Result<ChildMsg> {
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .await
            .map_err(|e| SebasError::Upgrade(format!("IPC recv 失败: {e}")))?;
        if line.is_empty() {
            return Err(SebasError::Upgrade("IPC 连接已关闭".into()));
        }
        serde_json::from_str(&line).map_err(|e| SebasError::Upgrade(format!("IPC 解析失败: {e}")))
    }
}

// ─── 子进程侧 IPC 句柄 ────────────────────────────────────

pub struct ChildIpc {
    writer: tokio::io::BufWriter<tokio::io::Stdout>,
}

impl Default for ChildIpc {
    fn default() -> Self {
        Self::new()
    }
}

impl ChildIpc {
    pub fn new() -> Self {
        Self {
            writer: tokio::io::BufWriter::new(tokio::io::stdout()),
        }
    }

    /// 发送 ready 信号：父进程据此判定本进程已就绪
    pub async fn ready(&mut self) -> Result<()> {
        let json = serde_json::to_string(&ChildMsg::Ready)
            .map_err(|e| SebasError::Upgrade(format!("IPC 序列化失败: {e}")))?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}

/// 检测当前进程是否运行在 watchdog 下
pub fn is_under_watchdog() -> bool {
    std::env::var("SEBAS_IPC").as_deref() == Ok("1")
}
