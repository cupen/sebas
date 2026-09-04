//! Crate-level session/command/event vocabulary (the anti-corrosion layer).
//!
//! `AcpEvent`/`AcpCommand`/`Decision` are the stable internal vocabulary the
//! router consumes — the name is historical (post-ACP; see ADR-1). Every
//! [`crate::AgentDriver`] implementation (the dedicated Claude driver and the
//! generic ACP driver) emits this same vocabulary, so downstream consumers
//! never see a driver-specific type.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};

/// A parked permission decision: a driver's permission hook parks on the
/// receiving end; `SessionManager::send(PermissionReply)` resolves it.
/// `oneshot::Sender` gives exact FnOnce semantics — a request can be answered
/// at most once.
pub(crate) type ResponderSlot = oneshot::Sender<Decision>;

/// Per-session handle stored in the manager's table. No process handle is
/// exposed — the driver owns the child; `cancel_tx` signals the driver loop to
/// exit (which disconnects and kills the child).
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
    /// The agent's real ACP session id when it differs from the routing id
    /// (native-ACP agents: `session/new` / loaded conversation id). `None`
    /// for Claude (routing id == conversation id) and for sessions whose
    /// driver did not report one. Persisted so a restart can resume the
    /// conversation by the id the agent actually knows.
    pub acp_session_id: Option<String>,
    /// The session's model selection surface reported by the agent
    /// (`configOptions` 里的 model 类选项)。`None` = agent 未暴露模型选项
    /// （webui 不显示模型下拉）。
    pub model: Option<AcpModelInfo>,
    pub handle: AcpSessionHandle,
    /// Set by kill()/kill_all() before signalling shutdown, so the wrapper
    /// task does not synthesize a crash event for an explicit kill.
    pub expected_exit: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// 会话的模型选择面：来自 agent 响应里的 `configOptions`（`id=="model"` /
/// category==model 的 select 选项），不是硬编码列表。`None` 表示 agent 未
/// 暴露模型选项（webui 不显示下拉、不报错）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AcpModelInfo {
    /// 当前生效的模型 id（agent 的 `currentValue`）。
    pub current: String,
    /// 可选的模型 id 列表（agent 的 select options 的 value 去重序）。
    pub options: Vec<String>,
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
    /// 把 ACP 会话的模型切到 `model_id`：driver 发标准
    /// `session/set_config_option {configId:"model", value:<model_id>}`。
    /// 失败（无效模型 / agent 无此能力）会显式报错——`SessionManager::send`
    /// 返回错误时调用方应把错误呈现给用户，且会话当前模型不变。
    SetModel {
        session_id: String,
        model_id: String,
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
    /// Emitted when the driver reports model info or token usage for a message
    /// or turn. Carries partial data: the model name may arrive on a
    /// session_start message, while token counts arrive on each assistant
    /// message and the result message.
    UsageUpdate {
        session_id: String,
        #[serde(flatten)]
        usage: TurnUsage,
    },
    /// Emitted by the driver after a successful `SetModel`（本地 current
    /// model 已更新，`session/set_config_option` 被 agent 接受）。
    ModelChanged {
        session_id: String,
        model_id: String,
    },
}
