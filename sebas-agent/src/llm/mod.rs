//! LLM 客户端抽象（design N5）：trait + 请求/响应类型。
//!
//! 生产实现 [`anthropic::AnthropicMessagesClient`] 面向任意 Anthropic Messages
//! 流式端点——直连 provider（默认）或经可选 gateway；测试实现 [`fake::FakeLlmClient`]。

pub mod anthropic;

pub use anthropic::AnthropicMessagesClient;
pub mod fake;

use crate::message::{ContentBlock, Message};

/// 协议咨询常量（task 4.3，design N6）：注册面与预算收尾的硬边界。
pub mod consult {
    /// Anthropic `tools` 数组的声明上限。
    pub const MAX_TOOL_DECLARATIONS: usize = 128;
    /// 上下文逼近此比例即干净收尾（budget-exhausted），不做侥幸调用。
    pub const CONTEXT_FINISH_RATIO: f64 = 0.9;
}

/// 流式增量事件：从 SSE 流解析出来就立刻回调（checklist C2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
}

/// 暴露给模型的工具声明（映射 Anthropic `tools` 数组条目）。
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// 一次 LLM 请求。
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub max_tokens: u32,
}

/// 停止原因（Anthropic stop_reason 映射）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other(String),
}

/// 一轮装配完成的模型响应。
#[derive(Debug, Clone)]
pub struct LlmTurn {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
}

/// LLM 错误。`terminal=true` 表示会话不可恢复（进程/协议层崩坏），
/// `terminal=false` 表示可重试（网络、429、5xx、provider 上报的流内错误）。
#[derive(Debug, Clone)]
pub struct LlmError {
    pub terminal: bool,
    pub message: String,
}

impl LlmError {
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            terminal: false,
            message: message.into(),
        }
    }

    pub fn terminal(message: impl Into<String>) -> Self {
        Self {
            terminal: true,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}llm error: {}", if self.terminal { "[terminal] " } else { "" }, self.message)
    }
}

impl std::error::Error for LlmError {}

/// LLM 客户端。`sink` 在流式解析过程中被同步回调（delta 尽快到达界面），
/// future resolve 时整轮内容已装配完成。
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    async fn stream_turn(
        &self,
        req: &LlmRequest,
        sink: &(dyn Fn(StreamEvent) + Send + Sync),
    ) -> Result<LlmTurn, LlmError>;
}
