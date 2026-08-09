//! 内置 test provider（`--debug` / `[gateway] debug = true`）。
//!
//! debug 模式给 gateway 增加一个自定义模型 `test`：请求不转发到外部上游，
//! 而是由 gateway 自身应答——固定文字 + 回显输入里的最后一条用户消息，例如
//! `I'm test provider. I received your message "hello".`。
//! Anthropic（/v1/messages）与 OpenAI（/v1/chat/completions）两个协议面、
//! 流式与非流式都支持。

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::Value;

use crate::proto::Protocol;

/// 回显文案：`I'm test provider. I received your message "<echo>".`
pub fn test_message(echo: &str) -> String {
    format!("I'm test provider. I received your message \"{echo}\".")
}

/// 从 buffered 请求 body 提取「最后一条 user 消息的文本」用于回显。
/// 兼容 Anthropic / OpenAI 的 `messages` 数组：content 为字符串或 text 块数组。
/// 无 messages / 无 user 消息 → 空串。
pub fn echo_text(body: Option<&axum::body::Bytes>) -> String {
    let Some(bytes) = body else {
        return String::new();
    };
    let Ok(v) = serde_json::from_slice::<Value>(bytes.as_ref()) else {
        return String::new();
    };
    let Some(msgs) = v.get("messages").and_then(|m| m.as_array()) else {
        return String::new();
    };
    msgs.iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| content_text(m.get("content")))
        .unwrap_or_default()
}

/// 归一化 content 字段（字符串直接取；数组取各 text 块拼接）。
fn content_text(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(blocks)) => {
            let parts: Vec<&str> = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(""))
            }
        }
        _ => None,
    }
}

/// 请求是否要求流式（body 的 `stream` 布尔）。
pub fn wants_stream(body: Option<&axum::body::Bytes>) -> bool {
    body.and_then(|b| serde_json::from_slice::<Value>(b.as_ref()).ok())
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

/// 生成 test provider 响应（200），按协议面 + stream 标志选形状。
pub fn test_response(proto: Protocol, echoed: &str, stream: bool) -> Response {
    let text = test_message(echoed);
    match (proto, stream) {
        (Protocol::Anthropic, false) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(anthropic_json(&text)))
            .expect("static response parts valid"),
        (Protocol::Anthropic, true) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(Body::from(anthropic_sse(&text)))
            .expect("static response parts valid"),
        (Protocol::OpenAi, false) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(openai_json(&text)))
            .expect("static response parts valid"),
        (Protocol::OpenAi, true) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(Body::from(openai_sse(&text)))
            .expect("static response parts valid"),
    }
}

fn anthropic_json(text: &str) -> String {
    serde_json::json!({
        "id": "msg_test_debug",
        "type": "message",
        "role": "assistant",
        "model": "test",
        "content": [{"type": "text", "text": text}],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {"input_tokens": 0, "output_tokens": 0, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}
    })
    .to_string()
}

fn anthropic_sse(text: &str) -> String {
    let escaped = serde_json::to_string(text).expect("string serializes");
    format!(
        "event: message_start\n\
data: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_test_debug\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"test\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{{\"input_tokens\":0,\"output_tokens\":0}}}}}}\n\
\n\
event: content_block_start\n\
data: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\
\n\
event: content_block_delta\n\
data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{escaped}}}}}\n\
\n\
event: content_block_stop\n\
data: {{\"type\":\"content_block_stop\",\"index\":0}}\n\
\n\
event: message_delta\n\
data: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":0}}}}\n\
\n\
event: message_stop\n\
data: {{\"type\":\"message_stop\"}}\n\
\n"
    )
}

fn openai_json(text: &str) -> String {
    serde_json::json!({
        "id": "chatcmpl-test-debug",
        "object": "chat.completion",
        "created": 0,
        "model": "test",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
    })
    .to_string()
}

fn openai_sse(text: &str) -> String {
    let escaped = serde_json::to_string(text).expect("string serializes");
    format!(
        "data: {{\"id\":\"chatcmpl-test-debug\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"test\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"\"}},\"finish_reason\":null}}]}}\n\
\n\
data: {{\"id\":\"chatcmpl-test-debug\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"test\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{escaped}}},\"finish_reason\":null}}]}}\n\
\n\
data: {{\"id\":\"chatcmpl-test-debug\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"test\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\
\n\
data: [DONE]\n\
\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;

    fn body(s: &str) -> Bytes {
        Bytes::from(s.to_string())
    }

    #[test]
    fn echo_text_extracts_last_user_message_string_content() {
        let b = body(
            r#"{"model":"test","messages":[{"role":"user","content":"first"},{"role":"assistant","content":"hi"},{"role":"user","content":"hello"}]}"#,
        );
        assert_eq!(echo_text(Some(&b)), "hello");
    }

    #[test]
    fn echo_text_handles_content_blocks() {
        let b = body(
            r#"{"model":"test","messages":[{"role":"user","content":[{"type":"text","text":"hello"},{"type":"text","text":" world"}]}]}"#,
        );
        assert_eq!(echo_text(Some(&b)), "hello world");
    }

    #[test]
    fn echo_text_falls_back_to_empty() {
        assert_eq!(echo_text(None), "");
        assert_eq!(echo_text(Some(&body("not json"))), "");
        assert_eq!(echo_text(Some(&body(r#"{"model":"test"}"#))), "");
        assert_eq!(
            echo_text(Some(&body(
                r#"{"model":"test","messages":[{"role":"assistant","content":"x"}]}"#
            ))),
            ""
        );
    }

    #[test]
    fn wants_stream_detects_flag() {
        assert!(wants_stream(Some(&body(r#"{"stream":true}"#))));
        assert!(!wants_stream(Some(&body(r#"{"stream":false}"#))));
        assert!(!wants_stream(Some(&body(r#"{}"#))));
        assert!(!wants_stream(None));
    }

    #[test]
    fn test_message_matches_requested_wording() {
        assert_eq!(
            test_message("hello"),
            "I'm test provider. I received your message \"hello\"."
        );
    }

    #[tokio::test]
    async fn anthropic_json_response_contains_text_and_model() {
        let resp = test_response(Protocol::Anthropic, "hello", false);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("read body");
        let v: Value = serde_json::from_str(&String::from_utf8_lossy(&bytes)).expect("valid JSON");
        assert_eq!(v["model"], "test");
        assert_eq!(
            v["content"][0]["text"],
            "I'm test provider. I received your message \"hello\"."
        );
    }

    #[tokio::test]
    async fn anthropic_sse_response_contains_text() {
        let resp = test_response(Protocol::Anthropic, "hi", true);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        let text = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("read body");
        let s = String::from_utf8_lossy(&text);
        assert!(s.contains("event: message_start"));
        assert!(s.contains("I'm test provider. I received your message \\\"hi\\\"."));
    }

    #[tokio::test]
    async fn openai_json_response_contains_text() {
        let resp = test_response(Protocol::OpenAi, "hello", false);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("read body");
        let v: Value = serde_json::from_str(&String::from_utf8_lossy(&bytes)).expect("valid JSON");
        assert_eq!(
            v["choices"][0]["message"]["content"],
            "I'm test provider. I received your message \"hello\"."
        );
    }

    #[tokio::test]
    async fn openai_sse_response_contains_text_and_done() {
        let resp = test_response(Protocol::OpenAi, "hello", true);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("read body");
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("data: [DONE]"));
        assert!(s.contains("I'm test provider. I received your message \\\"hello\\\"."));
    }
}
