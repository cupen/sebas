//! 核心数据模型：消息、内容块、工具结果与预算配置（task 1.2）。
//!
//! serde 形状与 Anthropic Messages wire format 对齐（`type` tag + snake_case），
//! 便于 `llm::anthropic` 直接装配请求体。

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 会话消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// 消息内容块。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

/// 一条消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }
}

/// 工具错误类别。错误是数据（checklist C4）：以结构化结果回填给模型，绝不中断循环。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorKind {
    InvalidArgs,
    NotFound,
    Denied {
        reason: String,
    },
    Cancelled,
    Timeout,
    Io(String),
}

impl std::fmt::Display for ToolErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgs => write!(f, "invalid arguments"),
            Self::NotFound => write!(f, "not found"),
            Self::Denied { reason } => write!(f, "denied: {reason}"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Timeout => write!(f, "timeout"),
            Self::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

/// 工具执行结果。`ok=false` 时 `error` 携带类别，`output` 携带给模型看的细节。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub ok: bool,
    pub output: String,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolErrorKind>,
}

impl ToolOutput {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            ok: true,
            output: output.into(),
            truncated: false,
            exit_code: None,
            error: None,
        }
    }

    pub fn error(kind: ToolErrorKind, output: impl Into<String>) -> Self {
        Self {
            ok: false,
            output: output.into(),
            truncated: false,
            exit_code: None,
            error: Some(kind),
        }
    }

    /// 尾部截断到 `max_chars` 字符（字符边界安全）。
    pub fn capped(mut self, max_chars: usize) -> Self {
        let total = self.output.chars().count();
        if total > max_chars {
            let skipped = total - max_chars;
            let tail: String = self.output.chars().skip(skipped).collect();
            self.output = format!("[truncated: skipped first {skipped} chars]\n{tail}");
            self.truncated = true;
        }
        self
    }
}

/// 每 turn 的预算（C8）：模型调用次数、工具执行次数、墙钟时限，
/// 以及 Assembly 维度（Phase 2，design N4）：消息条数与 token 估算。
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    pub max_model_calls: u32,
    pub max_tool_calls: u32,
    pub turn_timeout: Duration,
    /// 单次模型调用允许携带的最大消息条数（超过 → budget 收尾，不是错误）。
    pub max_messages: usize,
    /// 单次模型调用允许携带的估算 token 上限（chars/4 + 块常数）。
    pub est_token_budget: usize,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_model_calls: 20,
            max_tool_calls: 50,
            turn_timeout: Duration::from_secs(10 * 60),
            max_messages: 80,
            est_token_budget: 100_000,
        }
    }
}

/// 工具结果入库改写上限（design N4）：首段保留 + 截断标记。
pub const RESULT_REWRITE_CAP: usize = 8_000;

/// 入库改写（task 3.1）：确定性——首段 `RESULT_REWRITE_CAP` 字符 + 显式标记。
/// `ToolEnd` 事件仍带改写前（cap 后）版本；只有回填模型的 tool_result 走改写。
pub fn rewrite_for_history(output: &str) -> String {
    let total = output.chars().count();
    if total <= RESULT_REWRITE_CAP {
        return output.to_string();
    }
    let head: String = output.chars().take(RESULT_REWRITE_CAP).collect();
    format!(
        "{head}\n[truncated: {rest} more characters omitted — use read with offset/limit or a narrower query for detail]"
    , rest = total - RESULT_REWRITE_CAP)
}

/// 构造请求消息时剔除 thinking 块（多轮回传需要 signature，1a 不启用扩展思考）。
pub fn strip_thinking(content: &[ContentBlock]) -> Vec<ContentBlock> {
    content
        .iter()
        .filter(|b| !matches!(b, ContentBlock::Thinking { .. }))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_turn_transcript_round_trips() {
        let transcript = vec![
            Message::user_text("list rust files"),
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Thinking {
                        thinking: "need glob".into(),
                    },
                    ContentBlock::Text {
                        text: "I'll check.".into(),
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_1".into(),
                        name: "glob".into(),
                        input: serde_json::json!({"pattern": "**/*.rs"}),
                    },
                ],
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "toolu_1".into(),
                    content: "a.rs\nb.rs".into(),
                    is_error: false,
                }],
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "toolu_2".into(),
                    content: "boom".into(),
                    is_error: true,
                }],
            },
        ];
        for m in &transcript {
            let json = serde_json::to_string(m).unwrap();
            let back: Message = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, m);
        }
        // wire tag 形状与 Anthropic 对齐
        let j = serde_json::to_value(&transcript[1].content[2]).unwrap();
        assert_eq!(j["type"], "tool_use");
        assert_eq!(j["name"], "glob");
        let j = serde_json::to_value(&transcript[2].content[0]).unwrap();
        assert_eq!(j["type"], "tool_result");
        assert_eq!(j["tool_use_id"], "toolu_1");
        assert_eq!(j["is_error"], false);
    }

    #[test]
    fn tool_output_error_round_trips() {
        let out = ToolOutput::error(
            ToolErrorKind::Denied {
                reason: "read-before-write".into(),
            },
            "refused",
        );
        let json = serde_json::to_string(&out).unwrap();
        let back: ToolOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(back, out);
        assert!(!back.ok);
    }

    #[test]
    fn tail_cap_is_char_boundary_safe() {
        let s = "你好世界".repeat(100); // 400 chars
        let out = ToolOutput::ok(s).capped(50);
        assert!(out.truncated);
        assert!(!out.output.contains('\u{FFFD}')); // 无替换符 = 没切在半个字符上
        assert!(out.output.contains("skipped first 350 chars"));
    }

    #[test]
    fn strip_thinking_removes_only_thinking() {
        let blocks = vec![
            ContentBlock::Thinking {
                thinking: "x".into(),
            },
            ContentBlock::Text {
                text: "y".into(),
            },
        ];
        let stripped = strip_thinking(&blocks);
        assert_eq!(stripped.len(), 1);
        assert!(matches!(&stripped[0], ContentBlock::Text { .. }));
    }
}
