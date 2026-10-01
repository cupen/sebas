pub mod acp_driver;
pub mod agent_driver;
pub mod claude;
pub mod session;
pub mod win_exe;

/// 模型切换失败回执的稳定标记（fix-webui-qa-round7 2.2，acp-model-selection
/// D2）：两个驱动（通用 ACP / claude 控制面）的 `SetModel` 拒绝/未送达消息
/// 都以本标记收尾，调度引擎与 pump 据此把该类**非终态 Error** 识别为「模型
/// 切换失败」——回合状态语义层的终态边（拒绝即收尾，不等 600s watchdog），
/// 而非回合内容。标记常量只此一份（消息在 sebas-acp 组装，消费方
/// sebas-dispatch / core pump 经此处引用），杜绝裸字符串两处漂移。
pub const MODEL_UNCHANGED_MARKER: &str = "模型未变";

pub use acp_driver::AcpDriver;
pub use agent_driver::{AgentDriver, DriverConfig, DriverError, DriverHandle};
pub use claude::{ClaudeDriver, DEFAULT_CLAUDE_MODEL, builtin_claude_models};
pub use win_exe::resolve_windows_executable;
pub use session::{
    AcpCommand, AcpEvent, AcpModelInfo, AcpSessionHandle, AvailableCommand, Decision, SessionMeta,
    TurnUsage,
};
