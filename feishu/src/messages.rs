//! Typed Feishu IM message request bodies (outbound).
//!
//! All outbound HTTP bodies for the IM message API live here. Each struct
//! mirrors the wire shape Feishu's "发送消息" docs (2024 v1 API,
//! `msg_type=text` / `interactive`) so a struct-literal construction
//! round-trips byte-for-byte to the previous `serde_json::json!` blocks.

use serde::Serialize;

/// Identifier kind passed as the URL query parameter `receive_id_type=...`.
/// Serialized as snake_case to match the Feishu wire format.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveIdType {
    /// Group chat: `receive_id` is a `chat_id` like `oc_xxx`.
    ChatId,
    /// Private DM: `receive_id` is a user's `open_id` like `ou_xxx`.
    OpenId,
}

/// POST body for sending a plain-text message.
///
/// `content` is pre-serialized JSON per Feishu's spec
/// (e.g. `{"text":"hello"}`) — Feishu requires a string, not a nested
/// object. The `SendTextRequest::new` constructor builds it from the
/// caller-supplied `text` so consumers don't have to.
#[derive(Debug, Clone, Serialize)]
pub struct SendTextRequest {
    pub receive_id: String,
    /// URL query param — not part of the body. `#[serde(skip)]` keeps the
    /// struct as a typed accessor for callers building the URL while
    /// excluding it from the serialized JSON.
    #[serde(skip)]
    pub receive_id_type: ReceiveIdType,
    /// Always `"text"` for this struct. Kept as a field so the wire shape
    /// stays auditable from the type alone.
    pub msg_type: String,
    pub content: String,
}

impl SendTextRequest {
    pub fn new(
        receive_id: impl Into<String>,
        receive_id_type: ReceiveIdType,
        text: impl Into<String>,
    ) -> Self {
        let content = serde_json::json!({ "text": text.into() }).to_string();
        Self {
            receive_id: receive_id.into(),
            receive_id_type,
            msg_type: "text".to_string(),
            content,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_text_request_serializes_with_open_id() {
        let r = SendTextRequest::new("ou_abc", ReceiveIdType::OpenId, "hi");
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["receive_id"], "ou_abc");
        assert_eq!(v["msg_type"], "text");
        // `content` is a JSON-stringified `{"text":"..."}`, per Feishu spec.
        assert_eq!(v["content"], r#"{"text":"hi"}"#);
        // `receive_id_type` is a URL query param, NOT a body field.
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            ["content", "msg_type", "receive_id"]
                .iter()
                .copied()
                .collect()
        );
    }

    #[test]
    fn send_text_request_serializes_with_chat_id() {
        let r = SendTextRequest::new("oc_xyz", ReceiveIdType::ChatId, "sebas 已启动");
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["receive_id"], "oc_xyz");
        assert_eq!(v["msg_type"], "text");
        assert_eq!(v["content"], r#"{"text":"sebas 已启动"}"#);
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            ["content", "msg_type", "receive_id"]
                .iter()
                .copied()
                .collect()
        );
    }
}
