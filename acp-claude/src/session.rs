use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::process::Child;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionId(pub String);

#[derive(Debug)]
pub struct AcpSessionHandle {
    pub child_id: String,
    pub child: Option<Child>,
    pub cmd_tx: mpsc::Sender<AcpCommand>,
    pub evt_rx: Arc<Mutex<mpsc::Receiver<AcpEvent>>>,
    pub _stdin_task: JoinHandle<()>,
}

#[derive(Debug)]
pub struct SessionMeta {
    pub session_id: String,
    pub handle: AcpSessionHandle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpCommand {
    CreateSession { session_id: String, prompt: String },
    ContinueSession { session_id: String, prompt: String },
    PermissionReply { session_id: String, request_id: String, decision: Decision },
    Cancel { session_id: String },
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
    TextDelta { session_id: String, delta: String },
    ThinkingDelta { session_id: String, delta: String },
    ToolStart { session_id: String, tool_name: String, args: Value },
    ToolProgress { session_id: String, tool_name: String, progress: String },
    ToolEnd { session_id: String, tool_name: String, result: String },
    PermissionRequest { session_id: String, request_id: String, tool_name: String, args: Value },
    Finished { session_id: String },
    Error { session_id: String, message: String },
}