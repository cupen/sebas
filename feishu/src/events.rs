use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone)]
pub enum FeishuIn {
    Text {
        key: SessionKey,
        text: String,
        reply_to: Option<String>,
    },
    Media {
        key: SessionKey,
        files: Vec<String>,
        caption: Option<String>,
    },
    ButtonCb {
        key: SessionKey,
        action: CardAction,
    },
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct SessionKey {
    pub chat_id: String,
    pub thread_id: Option<String>,
}

impl Serialize for SessionKey {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let s = match &self.thread_id {
            None => self.chat_id.clone(),
            Some(tid) => format!("{}\0{}", self.chat_id, tid),
        };
        ser.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for SessionKey {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        let (chat_id, thread_id) = match s.split_once('\0') {
            None => (s, None),
            Some((c, t)) => (c.to_owned(), Some(t.to_owned())),
        };
        Ok(SessionKey { chat_id, thread_id })
    }
}

#[derive(Debug, Clone)]
pub struct CardAction {
    pub session_id: String,
    pub request_id: Option<String>,
    /// Parsed at the events layer (which owns payload shape); the router
    /// consumes only this field for the allow/deny policy.
    pub decision: Option<String>,
    /// Raw event object, retained for debugging/future fields.
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct FeishuEnvelope {
    pub schema: String,
    pub header: FeishuHeader,
    pub event: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct FeishuHeader {
    pub event_type: String,
}

impl FeishuEnvelope {
    /// Convert a wire event to an internal event, filtering out non-owner senders.
    /// When `owner_id` is empty, the owner filter is skipped (single-user bots).
    pub fn into_event(self, owner_id: &str) -> Option<FeishuIn> {
        if !owner_id.is_empty() {
            let sender_open_id = self
                .event
                .pointer("/sender/sender_id/open_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if sender_open_id != owner_id {
                return None;
            }
        }

        if self.header.event_type == "card.action.trigger" {
            let chat_id = self
                .event
                .pointer("/chat_id")
                .or_else(|| self.event.pointer("/message/chat_id"))
                // Feishu's card.action.trigger envelope keeps the chat id
                // under /context/open_chat_id (not /chat_id or /message/...).
                .or_else(|| self.event.pointer("/context/open_chat_id"))
                .and_then(serde_json::Value::as_str)?
                .to_owned();
            let thread_id = self
                .event
                .pointer("/thread_id")
                .or_else(|| self.event.pointer("/message/thread_id"))
                .or_else(|| self.event.pointer("/context/open_thread_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            // Primary: the V2 button behaviors[].value round-trip location.
            // Fallback: the legacy flat layout (tolerance against payload drift).
            let pick = |primary: &str, legacy: &str| {
                self.event
                    .pointer(primary)
                    .or_else(|| self.event.pointer(legacy))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let session_id =
                pick("/action/value/session_id", "/action/session_id").unwrap_or_default();
            let request_id = pick("/action/value/request_id", "/action/request_id");
            let decision = pick("/action/value/decision", "/action/decision");
            return Some(FeishuIn::ButtonCb {
                key: SessionKey { chat_id, thread_id },
                action: CardAction {
                    session_id,
                    request_id,
                    decision,
                    value: self.event,
                },
            });
        }

        let message = self.event.pointer("/message")?;
        let chat_id = message.pointer("/chat_id")?.as_str()?.to_owned();
        let message_id = message.pointer("/message_id")?.as_str()?.to_owned();
        let message_type = message.pointer("/message_type")?.as_str()?;
        let content_str = message.pointer("/content")?.as_str()?;
        let thread_id = message
            .pointer("/thread_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let key = SessionKey { chat_id, thread_id };

        match (self.header.event_type.as_str(), message_type) {
            ("im.message.receive_v1", "text") => {
                let body: MessageBody = serde_json::from_str(content_str).ok()?;
                Some(FeishuIn::Text {
                    key,
                    text: body.text.unwrap_or_default(),
                    reply_to: Some(message_id),
                })
            }
            ("im.message.receive_v1", "image" | "file" | "audio") => Some(FeishuIn::Media {
                key,
                files: vec![message_id],
                caption: None,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct MessageBody {
    pub text: Option<String>,
}
