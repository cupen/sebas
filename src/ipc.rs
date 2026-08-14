//! Pipe IPC 协议：watchdog（父进程）与 sebas（子进程）之间通过 JSON Lines 通信。
//!
//! ## 协议
//!
//! 子进程（sebas）→ 父进程（watchdog）：
//! - `{"cmd":"ready"}`        — sebas 启动完成
//! - `{"cmd":"upgrade"}`      — 请求升级到最新版
//! - `{"cmd":"upgrade-dev"}`  — 请求 dev 编译升级
//! - `{"cmd":"rollback"}`     — 请求回滚
//!
//! 父进程（watchdog）→ 子进程（sebas）：
//! - `{"status":"ack"}`               — 命令已收到
//! - `{"status":"ok","msg":"..."}`    — 进度信息
//! - `{"status":"error","msg":"..."}` — 错误信息
//! - `{"status":"done","msg":"..."}`  — 即将重启，准备退出

use crate::error::{Result, SebasError};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::mpsc;

// ─── 消息类型 ────────────────────────────────────────────

/// 子进程发给父进程的命令
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd")]
pub enum ChildMsg {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "upgrade")]
    Upgrade {
        #[serde(default)]
        dry_run: bool,
    },
    #[serde(rename = "upgrade-dev")]
    UpgradeDev {
        #[serde(default)]
        dry_run: bool,
    },
    #[serde(rename = "rollback")]
    Rollback,
}

/// 父进程回复子进程的状态
#[derive(Debug, Serialize, Deserialize)]
pub struct ParentMsg {
    pub status: String,
    #[serde(default)]
    pub msg: String,
}

static WATCHDOG_TX: OnceLock<mpsc::UnboundedSender<ChildMsg>> = OnceLock::new();

/// 注册 `sebas run` 进程内复用的 watchdog IPC sender。
pub fn install_watchdog_sender(tx: mpsc::UnboundedSender<ChildMsg>) -> Result<()> {
    WATCHDOG_TX
        .set(tx)
        .map_err(|_| SebasError::Upgrade("watchdog IPC sender 已初始化".into()))
}

/// 通过运行中子进程的 IPC sender 请求 watchdog 执行命令。
pub fn send_watchdog_command(cmd: ChildMsg) -> Result<()> {
    let tx = WATCHDOG_TX.get().ok_or_else(|| {
        SebasError::Upgrade("当前进程未运行在 watchdog 下，无法执行该操作".into())
    })?;
    tx.send(cmd)
        .map_err(|_| SebasError::Upgrade("watchdog IPC sender 已关闭".into()))
}

// ─── 父进程侧 IPC 句柄 ────────────────────────────────────

pub struct ParentIpc {
    reader: BufReader<ChildStdout>,
    writer: tokio::io::BufWriter<ChildStdin>,
}

impl ParentIpc {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            reader: BufReader::new(stdout),
            writer: tokio::io::BufWriter::new(stdin),
        }
    }

    /// 读取子进程发来的下一条命令
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

    /// 发送消息给子进程
    pub async fn send(&mut self, msg: &ParentMsg) -> Result<()> {
        let json = serde_json::to_string(msg)
            .map_err(|e| SebasError::Upgrade(format!("IPC 序列化失败: {e}")))?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// 发送 ok 消息
    pub async fn ok(&mut self, msg: &str) -> Result<()> {
        self.send(&ParentMsg {
            status: "ok".into(),
            msg: msg.into(),
        })
        .await
    }

    /// 发送 error 消息
    pub async fn error(&mut self, msg: &str) -> Result<()> {
        self.send(&ParentMsg {
            status: "error".into(),
            msg: msg.into(),
        })
        .await
    }

    /// 发送 done 消息（即将重启）
    pub async fn done(&mut self, msg: &str) -> Result<()> {
        self.send(&ParentMsg {
            status: "done".into(),
            msg: msg.into(),
        })
        .await
    }
}

// ─── 子进程侧 IPC 句柄 ────────────────────────────────────

pub struct ChildIpc {
    reader: BufReader<tokio::io::Stdin>,
    writer: tokio::io::BufWriter<tokio::io::Stdout>,
}

impl ChildIpc {
    pub fn new() -> Self {
        Self {
            reader: BufReader::new(tokio::io::stdin()),
            writer: tokio::io::BufWriter::new(tokio::io::stdout()),
        }
    }

    /// 读取父进程发来的下一条消息
    pub async fn recv(&mut self) -> Result<ParentMsg> {
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

    /// 发送命令给父进程
    pub async fn send(&mut self, cmd: &ChildMsg) -> Result<()> {
        let json = serde_json::to_string(cmd)
            .map_err(|e| SebasError::Upgrade(format!("IPC 序列化失败: {e}")))?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// 发送 ready 信号
    pub async fn ready(&mut self) -> Result<()> {
        self.send(&ChildMsg::Ready).await
    }
}

/// 检测当前进程是否运行在 watchdog 下
pub fn is_under_watchdog() -> bool {
    std::env::var("SEBAS_IPC").as_deref() == Ok("1")
}
