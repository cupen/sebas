//! Neutral inbound events (design D3): the four universal IM interaction
//! kinds — text, media, button callback, form callback — each addressed by a
//! [`ChannelKey`]. Channel-specific gating (chat-type, mention) and reply
//! threading happen in the adapter; what the core sees is only these events.

use crate::key::ChannelKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A callback button's parsed action (mirrors the historical `CardAction`):
/// the adapter owns the payload parsing and fills the fields; the router
/// consumes `session_id`/`request_id`/`decision` and treats `value` as raw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelAction {
    pub session_id: String,
    pub request_id: Option<String>,
    /// Parsed decision (allow/deny-style) for permission buttons, if the
    /// payload carries one.
    pub decision: Option<String>,
    /// Raw callback payload, retained for debugging/future fields.
    pub value: Value,
}

/// One inbound interaction from any channel. Serialized externally tagged
/// (`{"Text": {...}}`) — this is the replay journal's frame format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChannelEvent {
    Text {
        key: ChannelKey,
        text: String,
        /// Channel-neutral reply metadata (feishu: triggering message id or
        /// topic root). Opaque to the core; adapters use it for threading.
        reply_target: Option<String>,
    },
    Media {
        key: ChannelKey,
        /// Channel file references (feishu: file keys); never downloaded content.
        files: Vec<String>,
        caption: Option<String>,
        reply_target: Option<String>,
    },
    ButtonCb {
        key: ChannelKey,
        action: ChannelAction,
    },
    /// Form-container submission: `value` is the submit button's custom
    /// payload, `form_value` maps component name → submitted value, and
    /// `card_ref` is the opaque reference of the presentation instance to
    /// update in place (feishu: `context.open_message_id`).
    FormCb {
        key: ChannelKey,
        value: Value,
        form_value: BTreeMap<String, Value>,
        card_ref: Option<String>,
    },
}

impl ChannelEvent {
    /// The event's originating session key.
    pub fn key(&self) -> &ChannelKey {
        match self {
            ChannelEvent::Text { key, .. }
            | ChannelEvent::Media { key, .. }
            | ChannelEvent::ButtonCb { key, .. }
            | ChannelEvent::FormCb { key, .. } => key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::ChannelKey;

    #[test]
    fn key_accessor_covers_all_variants() {
        let k = ChannelKey::feishu("oc_x", None);
        let ev = ChannelEvent::Text {
            key: k.clone(),
            text: "hi".into(),
            reply_target: None,
        };
        assert_eq!(ev.key(), &k);
        let ev = ChannelEvent::FormCb {
            key: k.clone(),
            value: Value::Null,
            form_value: BTreeMap::new(),
            card_ref: None,
        };
        assert_eq!(ev.key(), &k);
    }
}
