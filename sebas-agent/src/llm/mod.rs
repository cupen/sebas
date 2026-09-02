//! LLM 客户端抽象（design N5）：trait + 请求/响应类型。
//!
//! 生产实现 [`anthropic::AnthropicMessagesClient`] 面向任意 Anthropic Messages
//! 流式端点——直连 provider（默认）或经可选 gateway；测试实现 [`fake::FakeLlmClient`]。

pub mod anthropic;

pub use anthropic::AnthropicMessagesClient;
pub mod fake;

use crate::message::{ContentBlock, Message};

/// 协议咨询常量（task 4.3，design N6 的「LlmConsult 常量组」）：注册面与
/// 预算收尾的硬边界。常量必须被请求组装与预算逻辑真正消费，不做摆设。
pub mod consult {
    use crate::message::BudgetConfig;

    /// Anthropic Messages `tools` 数组的声明上限（API 硬约束）。请求组装边界
    /// （`loop_::TurnEngine`）据此对 schema 列表做确定性截断。
    pub const MAX_TOOL_DECLARATIONS: usize = 128;

    /// 上下文窗口逼近此比例即干净收尾（budget-exhausted 语义，不是错误）。
    /// 现有 Assembly 预算（`BudgetConfig::est_token_budget` 到限即 finish）是
    /// 该语义的绝对值形态；[`budget_for_context_window`] 按本比率从窗口推导。
    pub const CONTEXT_FINISH_RATIO: f64 = 0.9;

    /// 按模型上下文窗口推导 Assembly 预算（design N6）：`est_token_budget =
    /// 窗口 × [`CONTEXT_FINISH_RATIO`]`（向下取整，略保守），其余维度取默认。
    /// 默认 [`BudgetConfig`] 不受影响（绝对值 100k，语义出处见此）。
    pub fn budget_for_context_window(window_tokens: usize) -> BudgetConfig {
        BudgetConfig {
            est_token_budget: (window_tokens as f64 * CONTEXT_FINISH_RATIO) as usize,
            ..BudgetConfig::default()
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::BudgetConfig;

    /// task 4.3 验证：CONTEXT_FINISH_RATIO 真正参与预算推导——
    /// 窗口 × 0.9 即 finish 预算，其余预算维度保持默认。
    #[test]
    fn budget_for_context_window_derives_ratio_budget() {
        let b = consult::budget_for_context_window(200_000);
        assert_eq!(b.est_token_budget, 180_000);
        let d = BudgetConfig::default();
        assert_eq!(b.max_messages, d.max_messages);
        assert_eq!(b.max_model_calls, d.max_model_calls);
        assert_eq!(b.max_tool_calls, d.max_tool_calls);
        assert_eq!(b.turn_timeout, d.turn_timeout);
        // 常量本身即语义来源：90% 比率，工具声明上限 128
        assert_eq!(consult::CONTEXT_FINISH_RATIO, 0.9);
        assert_eq!(consult::MAX_TOOL_DECLARATIONS, 128);
    }
}
