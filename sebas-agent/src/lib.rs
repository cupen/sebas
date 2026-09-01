//! sebas-agent — sebas 的原生 in-process coding agent 内核（Phase 1a, headless）。
//!
//! - [`llm`]：Anthropic Messages 流式客户端（端点可配置：默认直连 provider，gateway 可选）
//!   与 [`llm::fake::FakeLlmClient`] 测试替身
//! - [`tools`]：统一 Tool trait + 六件套（bash / read / write / edit / glob / grep）
//! - [`loop_`]：turn 状态机（三重预算 / 取消安全 / 事件发射）
//! - [`session`]：SessionManager（多会话、mpsc 命令、broadcast 事件流、提示词装配）
//!
//! 事件词汇与 `acp-claude::AcpEvent` 一一对应（零新增变体，且不包含
//! PermissionRequest——1a 结构性保证不发出权限事件）；1b 由宿主 adapter 映射。

pub mod llm;
pub mod loop_;
pub mod message;
pub mod session;
pub mod tools;

pub use message::{BudgetConfig, ContentBlock, Message, Role, ToolErrorKind, ToolOutput};
