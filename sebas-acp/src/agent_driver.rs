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

use crate::session::{AcpCommand, AcpEvent, ResponderSlot};
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
    /// The driver's read/command loop, boxed. Awaiting it drives the session
    /// to completion (cancel, terminal error, or child exit). The manager
    /// owns the command sender (`cmd_rx` was passed in via [`DriverConfig`]).
    pub run: futures::future::BoxFuture<'static, ()>,
}

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
