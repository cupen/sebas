//! Public session/command/event types for the claude-backed engine.
//!
//! Post-ACP (see openspec/specs/acp-driver/spec.md; rationale in
//! docs/design-history.md ADR-1):
//! `AcpEvent`/`AcpCommand`/`Decision` are the stable internal vocabulary the
//! router consumes — the name is historical, no ACP wire protocol is involved
//! anymore. The engine adapter lives in `crate::claude::driver`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};

/// A parked permission decision: the driver's PreToolUse hook callback
/// blocks on the receiving end; `SessionManager::send(PermissionReply)`
/// resolves it. `oneshot::Sender` gives exact FnOnce semantics — a request
/// can be answered at most once.
pub(crate) type ResponderSlot = oneshot::Sender<Decision>;

/// Per-session handle stored in the manager's table. No process handle is
/// exposed — the SDK owns the child; `cancel_tx` signals the driver loop to
/// exit (which disconnects and SIGKILLs the child).
/// (No `Debug` derive: `ResponderSlot` does not implement `Debug`.)
pub struct AcpSessionHandle {
    pub session_id: String,
    pub cmd_tx: mpsc::Sender<AcpCommand>,
    pub evt_rx: Arc<Mutex<mpsc::Receiver<AcpEvent>>>,
    pub cancel_tx: Option<oneshot::Sender<()>>,
    pub pending_responders: Arc<Mutex<std::collections::HashMap<String, ResponderSlot>>>,
}

pub struct SessionMeta {
    pub session_id: String,
    pub handle: AcpSessionHandle,
    /// Set by kill()/kill_all() before signalling shutdown, so the wrapper
    /// task does not synthesize a crash event for an explicit kill.
    pub expected_exit: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpCommand {
    CreateSession {
        session_id: String,
        prompt: String,
    },
    ContinueSession {
        session_id: String,
        prompt: String,
    },
    PermissionReply {
        session_id: String,
        request_id: String,
        decision: Decision,
    },
    Cancel {
        session_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    AllowSession,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnUsage {
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpEvent {
    TextDelta {
        session_id: String,
        delta: String,
    },
    ThinkingDelta {
        session_id: String,
        delta: String,
    },
    ToolStart {
        session_id: String,
        tool_name: String,
        args: Value,
    },
    ToolProgress {
        session_id: String,
        tool_name: String,
        progress: String,
    },
    ToolEnd {
        session_id: String,
        tool_name: String,
        result: String,
    },
    PermissionRequest {
        session_id: String,
        request_id: String,
        tool_name: String,
        args: Value,
    },
    Finished {
        session_id: String,
    },
    Error {
        session_id: String,
        message: String,
        /// True when the session is unrecoverably dead (process exit,
        /// transport failure) — the router removes the mapping and shows ❌.
        /// `#[serde(default)]` keeps legacy fixtures/deserialization working.
        #[serde(default)]
        terminal: bool,
    },
    /// Emitted when the SDK reports model info or token usage for a message
    /// or turn. Carries partial data: the model name may arrive on a
    /// session_start system message, while token counts arrive on each
    /// assistant message and the result message.
    UsageUpdate {
        session_id: String,
        #[serde(flatten)]
        usage: TurnUsage,
    },
}
