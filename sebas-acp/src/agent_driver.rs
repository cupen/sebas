//! The [`AgentDriver`] abstraction: a seam that lets `SessionManager` drive
//! any third-party coding-agent subprocess without knowing its wire protocol.
//!
//! Two implementations exist today:
//! - [`crate::claude::driver::ClaudeDriver`] — the dedicated Claude Code
//!   driver, kept on `cc-agent-sdk` for Claude-specific depth (token usage).
//! - [`crate::acp_driver::AcpDriver`] — the generic ACP driver, speaking Agent
//!   Client Protocol v1 to any native-ACP agent (`gemini --acp`, etc.).
//!
//! Both emit the crate-level [`crate::session::AcpEvent`]/[`AcpCommand`]
//! vocabulary, so nothing downstream branches on which driver is bound to a
//! session.

use crate::session::{AcpCommand, AcpEvent, AcpModelInfo, ResponderSlot};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};

/// Everything a driver needs to establish one session and start its loop.
pub struct DriverConfig {
    /// The open kind slug this session is bound to (`"claude"`, `"gemini"`, …).
    /// Drivers use it to namespace `request_id`s and to annotate diagnostics.
    pub kind_slug: String,
    /// The full argv to spawn: `command[0]` is the executable, the rest args.
    pub command: Vec<String>,
    /// Optional working directory for the child.
    pub work_dir: Option<String>,
    /// Extra env vars merged into the child's environment (provider-derived
    /// keys like `ANTHROPIC_BASE_URL`/`OPENAI_API_KEY`). Empty when none.
    pub extra_env: Vec<(String, String)>,
    /// The sebas routing id (also becomes the agent conversation id).
    pub session_id: String,
    /// The agent's real session id to pass to `session/load`, when it differs
    /// from the routing id (native-ACP agents like opencode address a
    /// conversation by their own id). `None` → the driver falls back to
    /// `session_id` for loads (agents without a distinct id, legacy records).
    pub load_session_id: Option<String>,
    /// True → resume/load the previous conversation instead of starting fresh.
    pub resume: bool,
    /// Timeout for the spawn + initialize handshake.
    pub startup_timeout: std::time::Duration,
    /// Event sink; the driver sends `AcpEvent`s here as they occur.
    pub evt_tx: mpsc::Sender<AcpEvent>,
    /// Command channel the driver consumes to receive prompts/cancels.
    pub cmd_rx: mpsc::Receiver<AcpCommand>,
    /// Fires on kill(); the driver loop exits when it does.
    pub cancel_rx: oneshot::Receiver<()>,
    /// Shared responder map: the driver parks permission oneshots here keyed
    /// by `request_id`; the manager resolves them on `PermissionReply`.
    pub pending_perms: Arc<Mutex<HashMap<String, ResponderSlot>>>,
    /// Set when the driver itself emits a terminal `Error`, so the manager's
    /// wrapper doesn't synthesize a second one ("agent process exited").
    pub terminal_sent: Arc<std::sync::atomic::AtomicBool>,
}

/// What a successful [`AgentDriver::spawn`] returns: the routing id (which may
/// differ from the requested one when a resume was rejected and fell back to
/// fresh), plus the command sender and the boxed run loop the manager awaits.
pub struct DriverHandle {
    pub session_id: String,
    /// True only when a resume actually resumed the old conversation; false
    /// means fresh spawn or a resume-rejection fallback.
    pub resumed: bool,
    /// The agent's real ACP session id when it differs from the routing id
    /// (native-ACP agents). `Some(sid)` for fresh spawns (the `session/new`
    /// id) and successful loads (the loaded conversation id); always `None`
    /// for Claude, where the conversation id equals the routing id. Valid
    /// only after the handshake completes (see [`DriverHandle::handshake`]).
    pub acp_session_id: Option<String>,
    /// （add-acp-model-selection）会话建立时 agent 经 `configOptions` 暴露的
    /// 模型选择面。由 drive 在 `session/new`/`session/load` 后填充；`None` =
    /// agent 未暴露模型选项。有效时间同 `acp_session_id`（handshake 完成后）。
    pub model: Option<AcpModelInfo>,
    /// Optional async handshake completion: the driver's final routing id +
    /// `resumed` flag. The generic ACP driver completes its initialize/load
    /// handshake inside the run loop (the `agent-client-protocol` connect
    /// closure owns the connection), so the definitive id/`resumed` are only
    /// known after the run loop starts — this receiver delivers them.
    ///
    /// - `Some(rx)`: ACP driver; the manager must await the handshake (under
    ///   the startup timeout) before inserting the session and building the
    ///   spawn outcome.
    /// - `None`: drivers that handshake synchronously inside `spawn` (the
    ///   Claude driver); `session_id`/`resumed` are already final.
    ///
    /// 元组四元组见 [`HandshakeRx`]。`acp_session_id` 与 `model` 由 ACP 驱动
    /// 在连接闭包内解析后一次性递出。
    pub handshake: Option<HandshakeRx>,
    /// The driver's read/command loop, boxed. Awaiting it drives the session
    /// to completion (cancel, terminal error, or child exit). The manager
    /// owns the command sender (`cmd_rx` was passed in via [`DriverConfig`]).
    pub run: futures::future::BoxFuture<'static, ()>,
}

/// [`DriverHandle::handshake`] 递出的四元组：
/// `(final_routing_id, resumed, acp_session_id, model_info)`。
pub type HandshakeRx =
    tokio::sync::oneshot::Receiver<(String, bool, Option<String>, Option<AcpModelInfo>)>;

/// Why a spawn failed.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// The configured executable is absent or not runnable.
    #[error("agent binary not found or not runnable: {0}")]
    NotFound(String),
    /// A resume was rejected by the agent (conversation gone) and a fresh
    /// fallback is handled by the driver itself — callers should not see this.
    #[error("resume rejected: {0}")]
    ResumeRejected(String),
    /// The spawn/initialize handshake timed out.
    #[error("agent start timed out after {0:?}")]
    Timeout(std::time::Duration),
    /// Any other spawn failure.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The seam every third-party agent driver satisfies.
#[async_trait::async_trait]
pub trait AgentDriver: Send + Sync {
    /// Spawn the child, complete the initialize handshake, start the run loop,
    /// and return the handle. The run loop is already spawned (or boxed into
    /// `DriverHandle::run`); the manager only awaits it and wraps the handle.
    async fn spawn(&self, cfg: DriverConfig) -> Result<DriverHandle, DriverError>;
}
