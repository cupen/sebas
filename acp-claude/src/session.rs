//! Public session/command/event types and private session-state helpers
//! that bridge the official `agent-client-protocol` v2 SDK to the
//! `acp-claude` manager.

use agent_client_protocol::schema::v1::{
    ContentBlock, RequestPermissionResponse, SessionNotification, SessionUpdate, TextContent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

/// A type-erased callback that, when invoked, replies to a previously
/// captured `Responder<RequestPermissionResponse>`. We type-erase so the
/// pending-responder map does not have to be generic over every request
/// response type the SDK may add.
pub(crate) type ResponderSlot =
    Box<dyn FnOnce(RequestPermissionResponse) -> Result<(), agent_client_protocol::Error> + Send>;

/// Backwards-compat shim retained so `manager.rs` (and anything else
/// holding `AcpSessionHandle`) keeps compiling while we rewire the
/// internals onto the official SDK. No process handle is exposed any
/// more — the SDK owns the child. (No `Debug` derive: `ResponderSlot`
/// is a `Box<dyn FnOnce>` and does not implement `Debug`.)
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
}

/// Extract the text payload from a `ContentBlock` if it carries one.
/// Anything else (image / audio / resource link / resource) is returned
/// as an empty string — the agent's text-only pipeline doesn't surface
/// it.
pub(crate) fn content_block_text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(TextContent { text, .. }) => text.clone(),
        _ => String::new(),
    }
}

/// Translate a single `SessionUpdate` into the consumer-facing
/// `AcpEvent` set. Returns `None` for updates the consumer does not
/// care about (user-message echo, plan, mode/config changes, info
/// updates, usage updates).
pub(crate) fn translate_update(
    session_id: &str,
    notification: &SessionNotification,
) -> Option<AcpEvent> {
    match &notification.update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            let delta = content_block_text(&chunk.content);
            if delta.is_empty() {
                None
            } else {
                Some(AcpEvent::TextDelta {
                    session_id: session_id.to_string(),
                    delta,
                })
            }
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            let delta = content_block_text(&chunk.content);
            if delta.is_empty() {
                None
            } else {
                Some(AcpEvent::ThinkingDelta {
                    session_id: session_id.to_string(),
                    delta,
                })
            }
        }
        SessionUpdate::ToolCall(call) => Some(AcpEvent::ToolStart {
            session_id: session_id.to_string(),
            tool_name: call.title.clone(),
            args: call.raw_input.clone().unwrap_or(Value::Null),
        }),
        SessionUpdate::ToolCallUpdate(update) => {
            use agent_client_protocol::schema::v1::ToolCallStatus;
            let tool_name = update.fields.title.clone().unwrap_or_default();
            let progress = match update.fields.status {
                Some(ToolCallStatus::InProgress) => "in_progress".to_string(),
                Some(ToolCallStatus::Completed) => "completed".to_string(),
                Some(ToolCallStatus::Failed) => "failed".to_string(),
                Some(ToolCallStatus::Pending) => "pending".to_string(),
                Some(_) => "unknown".to_string(),
                None => String::new(),
            };
            if update.fields.raw_output.is_some() || update.fields.status.is_some() {
                if update.fields.raw_output.is_some() {
                    let result = update
                        .fields
                        .raw_output
                        .clone()
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    Some(AcpEvent::ToolEnd {
                        session_id: session_id.to_string(),
                        tool_name,
                        result,
                    })
                } else {
                    Some(AcpEvent::ToolProgress {
                        session_id: session_id.to_string(),
                        tool_name,
                        progress,
                    })
                }
            } else {
                None
            }
        }
        SessionUpdate::UserMessageChunk(_)
        | SessionUpdate::Plan(_)
        | SessionUpdate::AvailableCommandsUpdate(_)
        | SessionUpdate::CurrentModeUpdate(_)
        | SessionUpdate::ConfigOptionUpdate(_)
        | SessionUpdate::SessionInfoUpdate(_)
        | SessionUpdate::UsageUpdate(_) => None,
        _ => None,
    }
}
