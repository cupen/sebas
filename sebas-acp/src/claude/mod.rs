//! Claude 引擎绑定：ACP 会话的 Claude Code 具体实现（cc-agent-sdk）。
//!
//! crate 对外只暴露本模块；未来若接入其他 agent 引擎，平级新增兄弟模块即可。

pub mod agent_driver;
pub mod driver;
pub mod manager;
pub mod session;

pub use agent_driver::{AgentProtocol, ClaudeCodeDriver, ProviderResolution};
pub use manager::{SessionManager, SessionStart, SpawnOutcome};
pub use session::{AcpCommand, AcpEvent, AcpSessionHandle, Decision, SessionMeta};
