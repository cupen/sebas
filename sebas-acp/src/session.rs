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
/// exit (which disconnects and kills the child), and `run_task` lets an
/// explicit kill hard-abort the run loop when the agent is stuck mid-turn
/// (P3: cancel alone does not fire while the driver awaits a slow
/// `session/prompt`, so the child process would linger).
/// (No `Debug` derive: `ResponderSlot` does not implement `Debug`.)
pub struct AcpSessionHandle {
    pub session_id: String,
    pub cmd_tx: mpsc::Sender<AcpCommand>,
    pub evt_rx: Arc<Mutex<mpsc::Receiver<AcpEvent>>>,
    pub cancel_tx: Option<oneshot::Sender<()>>,
    pub pending_responders: Arc<Mutex<std::collections::HashMap<String, ResponderSlot>>>,
    /// The spawned run loop task (drives the driver's connect/read loop).
    /// `kill()` aborts it so the connection (and the child process group)
    /// drops immediately.
    pub run_task: Option<tokio::task::JoinHandle<()>>,
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

/// 一条 agent 自广告的斜杠命令（session-slash-commands D1）。`name` 是提交时
/// 的命令词（`/name`），`description` 是面板说明，`hint` 是参数提示
/// （claude 的 `argumentHint` / ACP 的 `UnstructuredCommandInput.hint`）。
///
/// （type-session-vocabularies）定义已移入 `sebas-domain::session`（断开
/// `sebas-domain → sebas-acp` 依赖边，使共享审批决定类型能被本 crate 取用），
/// 此处 `pub use` 原位再导出：字段、serde 属性与公开路径不变。
pub use sebas_domain::session::AvailableCommand;

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
    /// （add-agent-mode-selection）把会话切到控制面 mode（`ask`/`edit`/
    /// `allow`/`auto`）。claude driver 经 SDK 运行时 `set_permission_mode`
    /// 下发（控制面词汇→CLI permission mode 的映射在 driver 内）；接受后
    /// 发 `AcpEvent::ModeChanged`，拒绝/失败发非终态 `Error`（mode 不变、
    /// 会话存活）。不支持 mode 的执行体如实报错，不假装生效。
    SetMode {
        session_id: String,
        mode: String,
    },
}

/// （type-session-vocabularies 3.2）审批决定收敛为**唯一**共享定义
/// [`sebas_domain::session::PermissionDecision`]（四值 + 未知值路径），此处原位
/// 再导出（`Decision` 别名保住既有路径）。ACP 侧**没有** escalate 等价物：
/// 该值由边界降级为 `allow_once` 并记日志（`agent-driver` spec 明写这是层间
/// 唯一的语义适配），本 change 不改语义。线形状仍是 `{"decision": …}` 信封。
pub use sebas_domain::session::PermissionDecision;
pub use sebas_domain::session::PermissionDecision as Decision;

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
    /// （add-agent-mode-selection）运行时权限模式切换被 agent 接受后由
    /// driver 发出：`mode` 是**控制面词汇**（`ask`/`edit`/`allow`/`auto`），
    /// webui 据此更新快照的 effective mode。
    ModeChanged {
        session_id: String,
        mode: String,
    },
    /// （session-slash-commands D1）agent 广告的会话命令表：claude 路径取自
    /// `get_server_info()` 初始化握手、通用 ACP 路径解析
    /// `SessionUpdate::AvailableCommandsUpdate`，两条 intake 汇入同一条事件。
    /// 引擎消费后物化进会话快照（`SessionInfo.available_commands`）；
    /// 二次通知（重新广告）覆盖旧表。空表 = 无命令面板（诚实退化，非错误）。
    AvailableCommands {
        session_id: String,
        /// `#[serde(default)]` 兼容旧 fixture/旧报文反序列化。
        #[serde(default)]
        commands: Vec<AvailableCommand>,
    },
}
