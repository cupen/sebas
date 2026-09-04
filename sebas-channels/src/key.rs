//! Neutral session identity: [`ChannelKey`] = channel name + opaque
//! channel-specific reference (design D2). The core never interprets the
//! reference; each adapter owns its own reference encoding (feishu's is
//! `chat_id` optionally composed with `thread_id` via `\0`, preserving the
//! historical wire composite).

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Name of a registered channel (`"feishu"`, `"web"`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelName(pub String);

impl ChannelName {
    pub const WEB: &'static str = "web";
    pub const FEISHU: &'static str = "feishu";

    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ChannelName {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for ChannelName {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for ChannelName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ChannelName {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ChannelName {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        Ok(Self(String::deserialize(de)?))
    }
}

/// Neutral session identity. `reference` is opaque to the core: adapters
/// encode and decode it, the core only compares and echoes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelKey {
    pub channel: ChannelName,
    pub reference: String,
}

impl ChannelKey {
    pub fn new(channel: impl Into<ChannelName>, reference: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            reference: reference.into(),
        }
    }

    /// Feishu-flavoured key: chat id, optionally composed with the topic
    /// thread id. Encoding owned by the feishu adapter — this constructor
    /// lives here so the historical composite stays byte-identical.
    pub fn feishu(chat_id: &str, thread_id: Option<&str>) -> Self {
        let reference = match thread_id {
            None => chat_id.to_owned(),
            Some(tid) => format!("{chat_id}\0{tid}"),
        };
        Self::new(ChannelName::FEISHU, reference)
    }

    /// Web-originated session key (historical `web-{nanos}` shape kept).
    pub fn web_new() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        Self::new(ChannelName::WEB, format!("web-{ts}-{seq}"))
    }

    /// The channel name as a string slice.
    pub fn channel_str(&self) -> &str {
        &self.channel.0
    }
}

impl fmt::Display for ChannelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.channel, self.reference.replace('\0', "\\0"))
    }
}

impl Serialize for ChannelKey {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            channel: &'a str,
            reference: &'a str,
        }
        Wire {
            channel: &self.channel.0,
            reference: &self.reference,
        }
        .serialize(ser)
    }
}

impl<'de> Deserialize<'de> for ChannelKey {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            channel: String,
            reference: String,
        }
        let w = Wire::deserialize(de)?;
        Ok(Self {
            channel: ChannelName(w.channel),
            reference: w.reference,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feishu_key_composes_thread_into_opaque_reference() {
        let k = ChannelKey::feishu("oc_x", Some("t1"));
        assert_eq!(k.reference, "oc_x\0t1");
        assert_eq!(k.channel_str(), "feishu");
        let plain = ChannelKey::feishu("oc_y", None);
        assert_eq!(plain.reference, "oc_y");
    }

    #[test]
    fn wire_shape_is_structured_channel_plus_reference() {
        let k = ChannelKey::feishu("oc_x", Some("t1"));
        let json = serde_json::to_value(&k).unwrap();
        assert_eq!(json["channel"], "feishu");
        assert_eq!(json["reference"], "oc_x\0t1");
        let back: ChannelKey = serde_json::from_value(json).unwrap();
        assert_eq!(back, k);
    }

    #[test]
    fn web_keys_are_unique_and_prefixed() {
        let a = ChannelKey::web_new();
        let b = ChannelKey::web_new();
        assert_ne!(a, b);
        assert!(a.reference.starts_with("web-"));
        assert_eq!(a.channel_str(), "web");
    }

    #[test]
    fn display_hides_the_nul_separator() {
        let k = ChannelKey::feishu("oc_x", Some("t1"));
        assert_eq!(k.to_string(), "feishu:oc_x\\0t1");
    }
}
