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

// ---- 会话键编解码：唯一实现（add-domain-layer 2.2） -----------------------
//
// 编码形态：percent-encoded `channel\0reference`（URL-safe，`%00` 是分隔符）。
// 原 6 份实现（dispatch engine / webui routes / 根 node_link projection /
// sebas-im frontend / 根 agent_backend 的 JSON 形态 / 本文件此前无编解码）
// 经逐点审计（等价性结论见 change 任务 2.1 与各调用点注释）后收敛到这里；
// 解码只按**第一个** NUL 切分——飞书 `chat_id\0thread_id` 这类复合 reference
// 完整保留在 reference 字段里，`node\0{node}\0{sess}` 嵌套行键也靠首切语义
// 解析（node_link::projection::row_reference 的既定约定）。

/// Encode a [`ChannelKey`] for URLs / the channel wire: percent-encoded
/// `channel\0reference`. Unreserved bytes (`A-Z a-z 0-9 - _ . ~`) pass
/// through; everything else becomes `%XX`（与 urlencoding crate 一致——
/// 此前 dispatch 的手写 encoder 与 webui 的 urlencoding::encode 输出逐字节
/// 相同，黄金样本测试钉住）.
pub fn encode_session_key(key: &ChannelKey) -> String {
    encode_channel_key(key.channel.as_str(), &key.reference)
}

/// Encode a bare `(channel, reference)` pair（webui routes 口径的直系后裔）.
pub fn encode_channel_key(channel: &str, reference: &str) -> String {
    urlencoding::encode(&format!("{channel}\0{reference}")).into_owned()
}

/// Decode a percent-encoded `channel\0reference` back into a [`ChannelKey`].
///
/// 严格版：percent 解码失败、或解码后不含 NUL 分隔符 → `None`。需要 feishu
/// 回退的调用方（dispatch 原生桥 / im 卡片）在各自调用点用
/// [`percent_decode`] 拼装自己的回退策略——策略留在消费方，编解码只有一份。
pub fn decode_session_key(encoded: &str) -> Option<ChannelKey> {
    let raw = percent_decode(encoded)?;
    let (channel, reference) = raw.split_once('\0')?;
    Some(ChannelKey::new(channel, reference))
}

/// Percent-decode a string（编解码方案的原语，供回退策略复用）.
///
/// **严格**语义（原 dispatch 手写 decoder 的逐字节后裔）：非法转义
/// （`%` 后不足两位或非 hex）→ `None`；解码结果必须合法 UTF-8。
pub fn percent_decode(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1])?;
                let lo = hex_val(bytes[i + 2])?;
                out.push(hi << 4 | lo);
                i += 3;
            }
            b'%' => return None,
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// add-domain-layer 2.2：黄金样本逐字节比对（样本 = 收敛前 dispatch
    /// 手写 percent-encoder 对同一批键的输出，/tmp 黄金转储逐行核对）。
    #[test]
    fn encoding_matches_pre_convergence_golden_samples() {
        let cases = [
            (ChannelKey::new("web", "web-1"), "web%00web-1"),
            (ChannelKey::feishu("oc_x", None), "feishu%00oc_x"),
            (ChannelKey::feishu("oc_x", Some("t1")), "feishu%00oc_x%00t1"),
            (
                ChannelKey::new("node", "nodeA\0sess-1"),
                "node%00nodeA%00sess-1",
            ),
            (
                ChannelKey::new("web", "a b/c?d=e&f+g%"),
                "web%00a%20b%2Fc%3Fd%3De%26f%2Bg%25",
            ),
            (ChannelKey::new("web", "中文引用"), "web%00%E4%B8%AD%E6%96%87%E5%BC%95%E7%94%A8"),
            (ChannelKey::new("ch", ""), "ch%00"),
            (
                ChannelKey::new("web", "tilde~.dash-under_score"),
                "web%00tilde~.dash-under_score",
            ),
        ];
        for (key, want) in cases {
            assert_eq!(encode_session_key(&key), want, "key={key}");
        }
    }

    #[test]
    fn decode_round_trips_and_splits_on_first_nul() {
        let k = ChannelKey::feishu("oc_x", Some("t1"));
        let d = decode_session_key(&encode_session_key(&k)).unwrap();
        assert_eq!(d, k, "feishu thread composite must survive");

        // 嵌套行键：只按第一个 NUL 切，channel=node、reference 原样带内层 NUL。
        let remote = decode_session_key("node%00nodeA%00sess-1").unwrap();
        assert_eq!(remote.channel_str(), "node");
        assert_eq!(remote.reference, "nodeA\0sess-1");
    }

    #[test]
    fn strict_decode_rejects_garbage_and_nul_free_input() {
        // 严格口径（webui routes 的既有行为）：无 NUL → None。
        assert!(decode_session_key("plain-text").is_none());
        assert!(decode_session_key("").is_none());
        // 非法转义 → None（原 dispatch 手写 decoder 的严格语义）。
        assert!(decode_session_key("%ZZ").is_none());
        assert!(decode_session_key("a%ZZ%00b").is_none());
        // 尾部截断的转义同样拒绝。
        assert!(decode_session_key("a%2").is_none());
    }

    #[test]
    fn percent_decode_is_the_reusable_primitive() {
        assert_eq!(percent_decode("a%20b").as_deref(), Some("a b"));
        assert_eq!(percent_decode("%00").as_deref(), Some("\0"));
        assert_eq!(percent_decode("%ZZ"), None);
        assert_eq!(percent_decode("a%2"), None);
    }

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
