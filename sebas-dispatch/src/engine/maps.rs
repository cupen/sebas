//! Router 的登记表：root 卡 msg_id、未决权限卡、在飞自动模式切换。
//!
//! 从 router.rs 拆出；经 `super` re-export，外部路径 `sebas_dispatch::MsgIdMap` 等不变。
//! （permission-mode-auto-gate：聊天级 grant_all 白名单已退役——「本会话不再
//! 询问」改走会话 mode 门控，自动放行不再有 dispatch 侧存储。）

use sebas_channels::ChannelKey;
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
    /// at their final state. See openspec/specs/feishu-reactions/spec.md.
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

    /// Drop the mapping for `session_id`. Called when a session is torn down
    /// (closed via the WebUI, process died) so a future session with a
    /// recycled id never inherits a stale message_id.
    pub async fn drop(&self, session_id: &str) {
        self.inner.write().await.remove(session_id);
    }
}

/// One outstanding permission card: the chat to PATCH, the Feishu message_id
/// to PATCH by, and the (tool_name, args) call metadata (diagnostics only
/// since permission-mode-auto-gate retired the allowlist — the click handler
/// keys off `key`/`msg_id`).
#[derive(Debug, Clone)]
pub struct PermCardEntry {
    pub key: ChannelKey,
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
        key: ChannelKey,
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

/// 「本会话不再询问」点击后在飞的自动模式切换（permission-mode-auto-gate）。
/// 会话 routing id → 本次点击的卡片信息；SetMode 结果到达时取用：
/// `ModeChanged{auto}` = 成功（静默消费）；带驱动「模式未变」标记的非终态
/// `Error` = 失败（据实翻卡 + 写事件契约条目，放行不回滚）。
#[derive(Default, Clone)]
pub struct AutoModeSwitchMap {
    inner: Arc<RwLock<HashMap<String, AutoModeSwitch>>>,
}

/// 一次在飞的自动模式切换：发起它的权限卡（request_id + 飞书 message_id，
/// 用于失败时就地翻卡）与所属 chat（`UpdateCardByMsgId` 的寻址键）。
#[derive(Debug, Clone)]
pub struct AutoModeSwitch {
    pub request_id: String,
    /// 权限卡的飞书 message_id；`None` = 卡不归本进程跟踪（如 detached
    /// 前端渲染），失败只走事件契约、无卡可翻。
    pub msg_id: Option<String>,
    pub key: ChannelKey,
}

impl AutoModeSwitchMap {
    /// Record the in-flight auto-mode switch for `session_id`（后到覆盖先到：
    /// 同会话连点两张卡时以最后一次点击为准，早的那张已被 take 处理）。
    pub async fn record(&self, session_id: String, switch: AutoModeSwitch) {
        self.inner.write().await.insert(session_id, switch);
    }

    /// Take（并移除）the in-flight entry for `session_id`。取走即消费——
    /// 成功/失败各处理一次，重复事件不重复上报。
    pub async fn take(&self, session_id: &str) -> Option<AutoModeSwitch> {
        self.inner.write().await.remove(session_id)
    }
}

/// Per-ChannelKey 最近一次入站消息的回复目标。话题内 = 话题根消息的
/// `message_id`（`root_id` 归一化后）；主线 = 触发消息 `message_id`。
///
/// 话题出站卡（权限卡、初始 root 卡、失败提示卡）用它作为 `root_id`，保证
/// 回复聚合在同一个话题里。纯内存、不持久化：重启后由下一条入站消息重建。
#[derive(Default, Clone)]
pub struct ReplyTargetMap {
    inner: Arc<RwLock<HashMap<ChannelKey, String>>>,
}

impl ReplyTargetMap {
    /// 记录最近入站消息的回复目标。幂等覆盖。
    pub async fn set(&self, key: ChannelKey, target: String) {
        self.inner.write().await.insert(key, target);
    }

    /// 取最近一次入站回复目标（如果有）。
    pub async fn get(&self, key: &ChannelKey) -> Option<String> {
        self.inner.read().await.get(key).cloned()
    }

    /// 删除一个 key 的回复目标（会话结束时调用，防无界增长）。
    pub async fn clear(&self, key: &ChannelKey) {
        self.inner.write().await.remove(key);
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

    #[tokio::test]
    async fn auto_mode_switch_take_consumes_entry() {
        let m = AutoModeSwitchMap::default();
        assert!(m.take("s1").await.is_none());
        m.record(
            "s1".into(),
            AutoModeSwitch {
                request_id: "r1".into(),
                msg_id: Some("om_1".into()),
                key: ChannelKey::feishu("oc_x", None),
            },
        )
        .await;
        let taken = m.take("s1").await.expect("entry recorded");
        assert_eq!(taken.request_id, "r1");
        assert_eq!(taken.msg_id.as_deref(), Some("om_1"));
        // 取走即消费：重复事件不重复上报。
        assert!(m.take("s1").await.is_none());
    }
}
