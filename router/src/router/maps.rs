//! Router 的登记表：root 卡 msg_id、未决权限卡、会话级工具白名单。
//!
//! 从 router.rs 拆出；经 `super` re-export，外部路径 `router::MsgIdMap` 等不变。

use feishu::events::SessionKey;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Tracks root-card message_ids per session so `UpdateCard` can resolve a
/// `session_id` to a `message_id` (Feishu's PATCH endpoint needs the
/// message_id, not the session_id).
#[derive(Default, Clone)]
pub struct MsgIdMap {
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl MsgIdMap {
    /// Record the message_id of the **most recent** per-turn card for a session.
    /// Called by the dispatcher after each `send_card` returns. Streaming
    /// `UpdateCard`s resolve through `get(session_id)`, so each new turn's
    /// card "takes over" as the PATCH target — earlier turns stay frozen
    /// at their final state. See `docs/superpowers/plans/2026-08-04-per-turn-reply-quote.md`.
    pub async fn record(&self, session_id: String, msg_id: String) {
        self.inner.write().await.insert(session_id, msg_id);
    }

    pub async fn get(&self, session_id: &str) -> Option<String> {
        self.inner.read().await.get(session_id).cloned()
    }

    /// Return a snapshot of all message_id mappings.
    pub async fn snapshot_all(&self) -> HashMap<String, String> {
        self.inner.read().await.clone()
    }
}

/// One outstanding permission card: the chat to PATCH, the Feishu message_id
/// to PATCH by, and the (tool_name, args) needed to register the call in the
/// session allowlist when the user picks "Allow session".
#[derive(Debug, Clone)]
pub struct PermCardEntry {
    pub key: SessionKey,
    pub msg_id: String,
    pub tool_name: String,
    pub args: Value,
}

/// Tracks outstanding permission cards by `request_id` so the router can flip
/// them in place when the user clicks (or mark them expired on a stale click).
/// Keyed by request_id.
#[derive(Default, Clone)]
pub struct PermCardMap {
    inner: Arc<RwLock<HashMap<String, PermCardEntry>>>,
}

impl PermCardMap {
    pub async fn record(
        &self,
        request_id: String,
        key: SessionKey,
        msg_id: String,
        tool_name: String,
        args: Value,
    ) {
        self.inner.write().await.insert(
            request_id,
            PermCardEntry {
                key,
                msg_id,
                tool_name,
                args,
            },
        );
    }

    /// Take the entry for a given request_id. The entry is removed on
    /// `take` so a duplicate click finds nothing and is a no-op (Feishu still
    /// shows the resolved card; we don't re-update it).
    pub async fn take(&self, request_id: &str) -> Option<PermCardEntry> {
        self.inner.write().await.remove(request_id)
    }
}

/// Per-chat permission allowlist. When a user clicks "本会话不再询问" on a
/// permission card, the chat enters allow-all mode; subsequent
/// `PermissionRequest`s in the same chat are auto-approved without a card.
#[derive(Default, Clone)]
pub struct SessionAllowlist {
    inner: Arc<RwLock<HashMap<SessionKey, AllowEntry>>>,
}

/// Per-chat approval state. `allow_all` is set by "本会话不再询问" and
/// auto-approves every subsequent permission request in the chat;
/// `sigs` holds individual (tool, args) signatures (kept for the
/// granular-grant API and its tests).
///
/// Signature is `format!("{tool_name}|{args_json}")` where `args_json` is
/// `serde_json::to_string` of the canonicalized args value.
#[derive(Default)]
struct AllowEntry {
    allow_all: bool,
    sigs: std::collections::HashSet<String>,
}

impl SessionAllowlist {
    /// Check whether a (tool_name, args) call is allowed for the given chat:
    /// either the chat is in allow-all mode, or the exact signature was
    /// granted individually.
    pub async fn is_allowed(&self, key: &SessionKey, tool_name: &str, args: &Value) -> bool {
        let sig = tool_signature(tool_name, args);
        self.inner
            .read()
            .await
            .get(key)
            .map(|e| e.allow_all || e.sigs.contains(&sig))
            .unwrap_or(false)
    }

    /// Record an "Allow session" approval: from now on, auto-approve every
    /// permission request in this chat. Idempotent.
    pub async fn grant_all(&self, key: &SessionKey) {
        self.inner
            .write()
            .await
            .entry(key.clone())
            .or_default()
            .allow_all = true;
    }

    /// Record a single-signature approval. Idempotent.
    pub async fn grant(&self, key: &SessionKey, tool_name: &str, args: &Value) {
        let sig = tool_signature(tool_name, args);
        self.inner
            .write()
            .await
            .entry(key.clone())
            .or_default()
            .sigs
            .insert(sig);
    }

    /// Drop the allowlist for a chat (session ended). Called from
    /// `remove_by_session` and similar lifecycle hooks.
    pub async fn clear(&self, key: &SessionKey) {
        self.inner.write().await.remove(key);
    }
}

/// Canonical signature for matching tool calls. Canonicalizes `args` so
/// that two semantically-equal (tool, args) calls hash to the same string
/// regardless of:
///   - key order in objects (Claude may serialise the same object with
///     keys in different order on different invocations)
///   - null fields (Claude sometimes emits `parent_tool_use_id: null`
///     or other optional wrappers)
///
/// Array order is preserved (semantically meaningful for command args,
/// env, etc.).
pub fn tool_signature(tool_name: &str, args: &Value) -> String {
    let canonical = canonicalize_value(args);
    let args_str = serde_json::to_string(&canonical).unwrap_or_else(|_| "{}".to_string());
    format!("{tool_name}|{args_str}")
}

/// Recursively canonicalize a JSON value for stable hashing:
/// - Objects: drop `null` fields, sort remaining keys, recurse.
/// - Arrays: preserve order, recurse.
/// - Other: unchanged.
fn canonicalize_value(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), canonicalize_value(v)))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().collect())
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_value).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn msgid_record_and_get_round_trip() {
        let m = MsgIdMap::default();
        assert!(m.get("s1").await.is_none());
        m.record("s1".into(), "om_abc".into()).await;
        assert_eq!(m.get("s1").await.as_deref(), Some("om_abc"));
        // overwrite
        m.record("s1".into(), "om_def".into()).await;
        assert_eq!(m.get("s1").await.as_deref(), Some("om_def"));
        // isolation
        m.record("s2".into(), "om_xyz".into()).await;
        assert_eq!(m.get("s2").await.as_deref(), Some("om_xyz"));
    }
}
