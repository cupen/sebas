//! Anthropic `/v1/messages` 线协议**响应形状**构造（fake-provider-upstream 1.1）。
//!
//! 从 `test_provider` 下沉出来的共用事实源：内置 debug `test` provider
//! （`test_provider.rs`）与 `fake_provider.rs`（本地假上游）都用这里的
//! `AnthropicMessage` 生成非流式 JSON 与流式 SSE，避免两处各写一份 Anthropic
//! 线协议知识。
//!
//! 只构造**响应**方向（router 从不构造 Anthropic 请求）；请求方向的解析
//! （echo / wants_stream / tool 扫描）仍留在各自模块。

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

/// Anthropic usage 四元组（线协议字段名逐一对应）。`Default` = 全零
/// （debug `test` provider 的既有形状）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

impl AnthropicUsage {
    /// usage 对象（json! 保序：input → output → cache_read → cache_creation）。
    fn to_value(self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_read_input_tokens": self.cache_read_input_tokens,
            "cache_creation_input_tokens": self.cache_creation_input_tokens,
        })
    }
}

/// 一条完整的 assistant message（非流式 JSON 与流式 SSE 同一事实源）。
#[derive(Debug, Clone)]
pub struct AnthropicMessage {
    pub id: String,
    pub model: String,
    /// content 块数组（`text` / `tool_use`）。空数组合法（SSE 起始帧恒为空）。
    pub content: Vec<Value>,
    /// `end_turn` / `tool_use` / …（SSE 的 message_delta.delta.stop_reason）。
    pub stop_reason: String,
    pub usage: AnthropicUsage,
}

impl AnthropicMessage {
    /// 单 text 块的便捷构造（debug test provider 与 fake 的文本应答共用）。
    pub fn text(
        id: impl Into<String>,
        model: impl Into<String>,
        text: &str,
        stop_reason: impl Into<String>,
        usage: AnthropicUsage,
    ) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            content: vec![text_block(text)],
            stop_reason: stop_reason.into(),
            usage,
        }
    }

    /// 非流式 200 JSON message 体（`application/json`）。
    pub fn json(&self) -> String {
        self.to_message_value().to_string()
    }

    /// 流式 SSE 事件序列：
    /// `message_start → (content_block_start → content_block_delta(s) →
    /// content_block_stop)* → message_delta → message_stop`。
    ///
    /// text 块用 `text_delta`；tool_use 块用 `input_json_delta`（`partial_json`
    /// 为完整 input 的一次性序列化——Anthropic 官方 SDK 按增量拼接，单帧亦然）。
    pub fn sse(&self) -> String {
        let mut out = String::new();
        let mut push = |event: &str, data: Value| {
            out.push_str("event: ");
            out.push_str(event);
            out.push_str("\ndata: ");
            out.push_str(&data.to_string());
            out.push_str("\n\n");
        };

        // message_start：content 为空数组，usage 只带 input/cache（output 由
        // message_delta 的累计值承载——与 router 的 SseUsageParser 契约一致）。
        push(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": self.id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {
                        "input_tokens": self.usage.input_tokens,
                        "output_tokens": 0,
                        "cache_read_input_tokens": self.usage.cache_read_input_tokens,
                        "cache_creation_input_tokens": self.usage.cache_creation_input_tokens,
                    },
                },
            }),
        );

        for (index, block) in self.content.iter().enumerate() {
            let index = index as u64;
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                    push(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {"type": "text", "text": ""},
                        }),
                    );
                    push(
                        "content_block_delta",
                        json!({
                            "type": "content_block_delta",
                            "index": index,
                            "delta": {"type": "text_delta", "text": text},
                        }),
                    );
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(Value::as_str).unwrap_or("");
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                    let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                    push(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}},
                        }),
                    );
                    push(
                        "content_block_delta",
                        json!({
                            "type": "content_block_delta",
                            "index": index,
                            "delta": {
                                "type": "input_json_delta",
                                "partial_json": input.to_string(),
                            },
                        }),
                    );
                }
                // 未知块类型：只发 start/stop（保守透传形状，不发伪造 delta）。
                _ => {
                    push(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": block.clone(),
                        }),
                    );
                }
            }
            push(
                "content_block_stop",
                json!({"type": "content_block_stop", "index": index}),
            );
        }

        push(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": {"stop_reason": self.stop_reason, "stop_sequence": null},
                "usage": {"output_tokens": self.usage.output_tokens},
            }),
        );
        push("message_stop", json!({"type": "message_stop"}));
        out
    }

    fn to_message_value(&self) -> Value {
        json!({
            "id": self.id,
            "type": "message",
            "role": "assistant",
            "model": self.model,
            "content": self.content,
            "stop_reason": self.stop_reason,
            "stop_sequence": null,
            "usage": self.usage.to_value(),
        })
    }

    /// 200 非流式响应（`application/json`）。
    pub fn into_json_response(self) -> Response {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(self.json()))
            .expect("static response parts valid")
    }

    /// 200 流式响应（`text/event-stream`）。
    pub fn into_sse_response(self) -> Response {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(Body::from(self.sse()))
            .expect("static response parts valid")
    }
}

/// Anthropic text content 块。
pub fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

/// Anthropic tool_use content 块。
pub fn tool_use_block(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage() -> AnthropicUsage {
        AnthropicUsage {
            input_tokens: 3,
            output_tokens: 5,
            cache_read_input_tokens: 1,
            cache_creation_input_tokens: 2,
        }
    }

    #[test]
    fn text_json_carries_content_stop_reason_and_usage() {
        let msg = AnthropicMessage::text("msg_x", "m", "hello", "end_turn", usage());
        let v: Value = serde_json::from_str(&msg.json()).expect("valid JSON");
        assert_eq!(v["id"], "msg_x");
        assert_eq!(v["type"], "message");
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["model"], "m");
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][0]["text"], "hello");
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["stop_sequence"], Value::Null);
        assert_eq!(v["usage"]["input_tokens"], 3);
        assert_eq!(v["usage"]["output_tokens"], 5);
        assert_eq!(v["usage"]["cache_read_input_tokens"], 1);
        assert_eq!(v["usage"]["cache_creation_input_tokens"], 2);
    }

    #[test]
    fn zero_usage_matches_debug_provider_shape() {
        // test_provider 历史形状：全零 usage + 单 text 块。
        let msg = AnthropicMessage::text(
            "msg_test_debug",
            "test",
            "hi",
            "end_turn",
            AnthropicUsage::default(),
        );
        let v: Value = serde_json::from_str(&msg.json()).expect("valid JSON");
        assert_eq!(v["usage"]["input_tokens"], 0);
        assert_eq!(v["usage"]["output_tokens"], 0);
        assert_eq!(v["model"], "test");
    }

    #[test]
    fn text_sse_event_sequence_is_complete_and_text_matches_json() {
        let msg = AnthropicMessage::text("msg_s", "m", "he\"llo", "end_turn", usage());
        let sse = msg.sse();
        let events: Vec<&str> = sse
            .lines()
            .filter_map(|l| l.strip_prefix("event: "))
            .collect();
        assert_eq!(
            events,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );

        // text_delta 的文本与 JSON 一致（转义形式）。
        let json_text = serde_json::to_string("he\"llo").unwrap();
        assert!(
            sse.contains(&json_text),
            "escaped text delta missing:\n{sse}"
        );
        assert!(sse.contains("\"type\":\"text_delta\""));
        // message_delta 的 stop_reason + usage 与 JSON 一致。
        assert!(sse.contains("\"stop_reason\":\"end_turn\""));
        assert!(sse.contains("\"output_tokens\":5"));
        // 每个事件都以空行收尾（SSE 帧边界；router 的 SseUsageParser 依赖）。
        assert_eq!(sse.matches("\n\n").count(), events.len());
    }

    #[test]
    fn tool_use_sse_emits_input_json_delta_and_tool_stop_reason() {
        let msg = AnthropicMessage {
            id: "msg_t".into(),
            model: "m".into(),
            content: vec![tool_use_block("toolu_1", "Read", json!({"file_path": "x"}))],
            stop_reason: "tool_use".into(),
            usage: usage(),
        };
        let sse = msg.sse();
        assert!(sse.contains("\"type\":\"tool_use\""));
        assert!(sse.contains("\"name\":\"Read\""));
        assert!(sse.contains("\"type\":\"input_json_delta\""));
        assert!(sse.contains("\"stop_reason\":\"tool_use\""));
        // partial_json 是完整 input 的序列化（SDK 单帧拼接即可）——作为 JSON
        // 字符串嵌在 data 行里，按 JSON 转义后的形态比对。
        let partial = serde_json::to_string(&json!({"file_path": "x"})).unwrap();
        let embedded = serde_json::to_string(&partial).unwrap();
        assert!(sse.contains(&embedded), "partial_json missing:\n{sse}");
    }

    #[test]
    fn multi_block_indices_increment() {
        let msg = AnthropicMessage {
            id: "id".into(),
            model: "m".into(),
            content: vec![text_block("a"), text_block("b")],
            stop_reason: "end_turn".into(),
            usage: usage(),
        };
        let sse = msg.sse();
        assert!(sse.contains("\"index\":0"));
        assert!(sse.contains("\"index\":1"));
    }
}
