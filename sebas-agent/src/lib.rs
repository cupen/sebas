//! sebas-agent — sebas 的原生 in-process coding agent 内核。
//!
//! - [`llm`]：Anthropic Messages 流式客户端（端点可配置：默认直连 provider，gateway 可选）
//!   与 [`llm::fake::FakeLlmClient`] 测试替身
//! - [`tools`]：统一 Tool trait + 六件套（bash / read / write / edit / glob / grep）
//! - [`policy`]：权限策略引擎（三层判定 + 交互审批 + fail-closed）与 bash 沙箱
//!   （Landlock 进程内为主 + 防火墙回退，design N1/N2）
//! - [`loop_`]：turn 状态机（三重预算 / 取消安全 / 策略门控 / 事件发射）
//! - [`session`]：SessionManager（多会话、mpsc 命令、broadcast 事件流、提示词装配）
//!
//! 事件词汇沿 `AcpEvent` 形状（`type` tag + snake_case）；Phase 2 启用
//! `PermissionRequest`（策略审批面）并新增 `ToolPolicy`（策略结果事件）。

pub mod bench;
pub mod llm;
pub mod loop_;
pub mod message;
pub mod policy;
pub mod session;
pub mod tools;

pub use message::{BudgetConfig, ContentBlock, Message, Role, ToolErrorKind, ToolOutput};
pub use policy::{
    ApprovalAnswer, Approver, NetworkMode, PermissionRequestInfo, PolicyConfig, PolicyDecision,
    PolicyEngine, ToolRule,
};
