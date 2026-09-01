//! `FakeLlmClient`（task 2.1，design N10）：测试替身，双模式。
//!
//! - **脚本式（scripted）**：按序回放预设的 `Vec<LlmTurn>`——验证多步循环、
//!   事件序、预算上限。
//! - **有状态式（stateful）**：闭包按上一轮 tool result 动态生成下一轮——
//!   验证自愈场景（第一轮让 bash 失败，第二轮看到失败结果后改发成功命令）。

use crate::llm::{LlmError, LlmRequest, LlmTurn, StreamEvent};
use crate::message::ContentBlock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// 有状态模式：按请求历史（最近一条 user 消息里的 tool_result）生成响应。
pub type StatefulFn =
    Box<dyn Fn(&[crate::message::Message]) -> Vec<ContentBlock> + Send + Sync>;

enum Mode {
    /// 按序回放；耗尽后返回 terminal 错误（测试断言不应走到这里）。
    Scripted(Vec<LlmTurn>, AtomicUsize),
    Stateful(StatefulFn),
}

pub struct FakeLlmClient {
    mode: Mutex<Mode>,
    /// 每次调用经由 sink 发出的 StreamEvent 序列（脚本式按 LlmTurn 内容生成）。
    emit_deltas: bool,
}

impl FakeLlmClient {
    /// 脚本式：按序回放。
    pub fn scripted(turns: Vec<LlmTurn>) -> Self {
        Self {
            mode: Mutex::new(Mode::Scripted(turns, AtomicUsize::new(0))),
            emit_deltas: true,
        }
    }

    /// 脚本式 + 关闭 delta 发射（验证 sink 顺序的测试默认开）。
    pub fn scripted_silent(turns: Vec<LlmTurn>) -> Self {
        Self {
            mode: Mutex::new(Mode::Scripted(turns, AtomicUsize::new(0))),
            emit_deltas: false,
        }
    }

    /// 有状态式：闭包接收完整历史（含最近 tool_result）。
    pub fn stateful(f: StatefulFn) -> Self {
        Self {
            mode: Mutex::new(Mode::Stateful(f)),
            emit_deltas: true,
        }
    }

    fn text_turn(text: &str) -> LlmTurn {
        LlmTurn {
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            stop_reason: crate::llm::StopReason::EndTurn,
        }
    }

    /// 便捷构造：一段文本 turn。
    pub fn say(text: &str) -> LlmTurn {
        Self::text_turn(text)
    }

    /// 便捷构造：一段文本 + 多个工具调用的 turn（stop_reason = ToolUse）。
    pub fn call_tools(calls: Vec<(&str, &str, serde_json::Value)>) -> LlmTurn {
        LlmTurn {
            content: calls
                .into_iter()
                .map(|(id, name, input)| ContentBlock::ToolUse {
                    id: id.to_string(),
                    name: name.to_string(),
                    input,
                })
                .collect(),
            stop_reason: crate::llm::StopReason::ToolUse,
        }
    }
}

#[async_trait::async_trait]
impl crate::llm::LlmClient for FakeLlmClient {
    async fn stream_turn(
        &self,
        req: &LlmRequest,
        sink: &(dyn Fn(StreamEvent) + Send + Sync),
    ) -> Result<LlmTurn, LlmError> {
        let turn = {
            let mut mode = self.mode.lock().expect("fake llm poisoned");
            match &mut *mode {
                Mode::Scripted(turns, idx) => {
                    let i = idx.fetch_add(1, Ordering::SeqCst);
                    turns.get(i).cloned().ok_or_else(|| {
                        LlmError::terminal(format!(
                            "FakeLlmClient scripted sequence exhausted (called {i} times)"
                        ))
                    })?
                }
                Mode::Stateful(f) => LlmTurn {
                    content: f(&req.messages),
                    stop_reason: crate::llm::StopReason::EndTurn,
                },
            }
        };
        if self.emit_deltas {
            // 按 Anthropic 流语义回放 delta：Text/Thinking 块逐段回调。
            for block in &turn.content {
                match block {
                    ContentBlock::Text { text } => {
                        sink(StreamEvent::TextDelta(text.clone()));
                    }
                    ContentBlock::Thinking { thinking } => {
                        sink(StreamEvent::ThinkingDelta(thinking.clone()));
                    }
                    _ => {}
                }
            }
        }
        Ok(turn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmClient, StopReason};
    use crate::message::Message;
    use std::sync::Arc;

    #[tokio::test]
    async fn scripted_drives_a_two_round_conversation() {
        let client = FakeLlmClient::scripted(vec![
            FakeLlmClient::say("checking"),
            LlmTurn {
                content: vec![ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                }],
                stop_reason: StopReason::ToolUse,
            },
        ]);
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let e2 = events.clone();
        let sink = move |ev: StreamEvent| e2.lock().unwrap().push(ev);

        let r1 = client
            .stream_turn(
                &LlmRequest {
                    model: "m".into(),
                    system: String::new(),
                    messages: vec![Message::user_text("hi")],
                    tools: vec![],
                    max_tokens: 100,
                },
                &sink,
            )
            .await
            .unwrap();
        assert_eq!(r1.stop_reason, StopReason::EndTurn);
        assert!(matches!(
            events.lock().unwrap().pop(),
            Some(StreamEvent::TextDelta(t)) if t == "checking"
        ));

        let r2 = client
            .stream_turn(
                &LlmRequest {
                    model: "m".into(),
                    system: String::new(),
                    messages: vec![],
                    tools: vec![],
                    max_tokens: 100,
                },
                &sink,
            )
            .await
            .unwrap();
        assert_eq!(r2.stop_reason, StopReason::ToolUse);

        // 耗尽 → terminal 错误（测试断言不应走到这里）。
        let r3 = client
            .stream_turn(
                &LlmRequest {
                    model: "m".into(),
                    system: String::new(),
                    messages: vec![],
                    tools: vec![],
                    max_tokens: 100,
                },
                &sink,
            )
            .await;
        assert!(r3.unwrap_err().terminal);
    }

    #[tokio::test]
    async fn stateful_sees_the_latest_tool_result() {
        // 自愈测试的基石：闭包读取最近一条 user 消息的 tool_result 内容。
        let client = FakeLlmClient::stateful(Box::new(|history: &[Message]| {
            let saw_failure = history.iter().any(|m| {
                m.content.iter().any(|b| {
                    matches!(
                        b,
                        ContentBlock::ToolResult { content, is_error: true, .. }
                            if content.contains("exit 1")
                    )
                })
            });
            if saw_failure {
                vec![ContentBlock::Text {
                    text: "recovered".into(),
                }]
            } else {
                vec![ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "exit 1"}),
                }]
            }
        }));
        let sink =|_: StreamEvent| {};
        let req = |messages: Vec<Message>| LlmRequest {
            model: "m".into(),
            system: String::new(),
            messages,
            tools: vec![],
            max_tokens: 100,
        };
        let r1 = client
            .stream_turn(&req(vec![Message::user_text("run it")]), &sink)
            .await
            .unwrap();
        assert!(matches!(&r1.content[0], ContentBlock::ToolUse { name, .. } if name == "bash"));

        let r2 = client
            .stream_turn(
                &req(vec![Message {
                    role: crate::message::Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: "t1".into(),
                        content: "exit 1".into(),
                        is_error: true,
                    }],
                }]),
                &sink,
            )
            .await
            .unwrap();
        assert!(matches!(&r2.content[0], ContentBlock::Text { text } if text == "recovered"));
    }
}
