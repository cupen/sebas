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

/// POST body for sending a card (interactive) message.
///
/// `content` is the JSON-stringified card object (Feishu requires a
/// string, not nested JSON). `root_id` makes the new message render as a
/// reply to that parent message when present and non-empty; `#[serde(skip)]`
/// keeps an absent/empty `root_id` out of the body.
#[derive(Debug, Clone, Serialize)]
pub struct SendCardRequest {
    pub receive_id: String,
    /// URL query param — not part of the body. See `SendTextRequest`.
    #[serde(skip)]
    pub receive_id_type: ReceiveIdType,
    /// Always `"interactive"` for this struct.
    pub msg_type: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_id: Option<String>,
}

impl SendCardRequest {
    pub fn new(
        receive_id: impl Into<String>,
        receive_id_type: ReceiveIdType,
        card: &serde_json::Value,
    ) -> Self {
        Self {
            receive_id: receive_id.into(),
            receive_id_type,
            msg_type: "interactive".to_string(),
            content: card.to_string(),
            root_id: None,
        }
    }

    /// Builder-style setter for `root_id`. Silently drops empty strings so
    /// callers can pass `Option<&str>`-shaped data without an explicit
    /// `.filter(|s| !s.is_empty())`.
    pub fn with_reply(mut self, root_id: impl Into<String>) -> Self {
        let r: String = root_id.into();
        if !r.is_empty() {
            self.root_id = Some(r);
        }
        self
    }
}

/// PATCH body for updating an existing card message in place.
///
/// `content` is the JSON-stringified card object.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCardRequest {
    pub content: String,
}

/// POST body for adding an emoji reaction to a message.
#[derive(Debug, Clone, Serialize)]
pub struct ReactRequest {
    pub reaction_type: ReactionType,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReactionType {
    pub emoji_type: String,
}

impl ReactRequest {
    pub fn new(emoji_type: impl Into<String>) -> Self {
        Self {
            reaction_type: ReactionType {
                emoji_type: emoji_type.into(),
            },
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

    #[test]
    fn send_card_request_serializes_with_root_id() {
        let card = serde_json::json!({"header": {}, "body": {}});
        let r =
            SendCardRequest::new("oc_xyz", ReceiveIdType::ChatId, &card).with_reply("om_parent");
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["msg_type"], "interactive");
        assert_eq!(v["root_id"], "om_parent");
        assert!(v["content"].as_str().unwrap().contains("body"));
        let keys: std::collections::BTreeSet<&str> =
            v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            ["content", "msg_type", "receive_id", "root_id"]
                .iter()
                .copied()
                .collect()
        );
    }

    #[test]
    fn send_card_request_omits_empty_root_id() {
        let card = serde_json::json!({});
        let r = SendCardRequest::new("oc_xyz", ReceiveIdType::ChatId, &card).with_reply("");
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert!(v.get("root_id").is_none());
    }

    #[test]
    fn react_request_serializes_emoji_type() {
        let r = ReactRequest::new("Typing");
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["reaction_type"]["emoji_type"], "Typing");
    }
}
