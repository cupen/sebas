//! Tracks the current emoji reaction on each session's root card so the
//! router's phase machine can **swap** reactions (SEED/OnIt->DONE) rather
//! than pile them up. Feishu's unreact API needs the `reaction_id` returned
//! by the `react` call, so `react` returns it and we stash it here.
//!
//! Also tracks ack reactions ("已收到" 👌 on the user's inbound message,
//! keyed by Feishu message_id) so duplicate inbound events / replays never
//! stack a second 👌, and one-shot terminal marks (✅/❌ on that same
//! message once the turn settles) so repeated phase events don't double-mark.
//!
//! The *when* (which emoji on which phase transition) lives in the dispatch
//! FSM + im frontend; this struct owns only the *what* (the id bookkeeping)
//! and the swap plan.

use std::collections::HashMap;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactPlan {
    /// Same emoji is already the current reaction — nothing to do.
    Skip,
    /// No current reaction — just `react(new)`.
    ReactOnly,
    /// A different reaction is current — `unreact(old)` then `react(new)`.
    Swap { unreact_id: String },
}

#[derive(Default)]
pub struct ReactionTracker {
    inner: Mutex<HashMap<String, (String, String)>>, // session -> (emoji, reaction_id)
    /// Ack reactions keyed by Feishu message_id (not session_id): dedupe
    /// gate for the immediate "已收到" 👌 (kept for the message's lifetime;
    /// the terminal ✅/❌ is stacked next to it, not swapped in).
    ack_map: Mutex<HashMap<String, (String, String)>>, // message_id -> (emoji, reaction_id)
    /// One-shot terminal marks on the user's inbound message, keyed by
    /// message_id → emoji. First writer wins; the rest skip the API call.
    user_marks: Mutex<HashMap<String, String>>, // message_id -> emoji
}

impl ReactionTracker {
    pub async fn plan(&self, session_id: &str, emoji: &str) -> ReactPlan {
        let g = self.inner.lock().await;
        match g.get(session_id) {
            Some((cur, _)) if cur == emoji => ReactPlan::Skip,
            Some((_, rid)) => ReactPlan::Swap {
                unreact_id: rid.clone(),
            },
            None => ReactPlan::ReactOnly,
        }
    }

    pub async fn record(&self, session_id: &str, emoji: String, reaction_id: String) {
        self.inner
            .lock()
            .await
            .insert(session_id.into(), (emoji, reaction_id));
    }

    /// Record an ack reaction keyed by Feishu message_id. The entry doubles
    /// as the dedupe gate (`is_acked`) for duplicate inbound events.
    pub async fn record_ack(&self, message_id: &str, emoji: String, reaction_id: String) {
        self.ack_map
            .lock()
            .await
            .insert(message_id.into(), (emoji, reaction_id));
    }

    /// Whether an ack reaction was already recorded for this message.
    pub async fn is_acked(&self, message_id: &str) -> bool {
        self.ack_map.lock().await.contains_key(message_id)
    }

    /// One-shot terminal mark bookkeeping for the user's inbound message:
    /// returns true (= caller should `react`) on first claim, false when the
    /// message was already marked (phase events can repeat across resyncs).
    pub async fn claim_user_mark(&self, message_id: &str, emoji: &str) -> bool {
        let mut g = self.user_marks.lock().await;
        g.insert(message_id.into(), emoji.into()).is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn plan_react_only_when_no_current_reaction() {
        let t = ReactionTracker::default();
        assert_eq!(t.plan("s1", "OnIt").await, ReactPlan::ReactOnly);
    }

    #[tokio::test]
    async fn plan_skip_when_same_emoji_already_current() {
        let t = ReactionTracker::default();
        t.record("s1", "OnIt".into(), "rid_1".into()).await;
        assert_eq!(t.plan("s1", "OnIt").await, ReactPlan::Skip);
    }

    #[tokio::test]
    async fn plan_swap_with_old_reaction_id_when_emoji_changes() {
        let t = ReactionTracker::default();
        t.record("s1", "EYES".into(), "rid_eyes".into()).await;
        assert_eq!(
            t.plan("s1", "OnIt").await,
            ReactPlan::Swap {
                unreact_id: "rid_eyes".into()
            }
        );
    }

    #[tokio::test]
    async fn record_updates_current_so_next_plan_skips() {
        let t = ReactionTracker::default();
        t.record("s1", "EYES".into(), "rid_1".into()).await;
        // after swapping to OnIt and recording, a duplicate OnIt must skip
        t.record("s1", "OnIt".into(), "rid_2".into()).await;
        assert_eq!(t.plan("s1", "OnIt").await, ReactPlan::Skip);
    }

    #[tokio::test]
    async fn sessions_are_isolated() {
        let t = ReactionTracker::default();
        t.record("s1", "OnIt".into(), "rid_s1".into()).await;
        assert_eq!(t.plan("s2", "OnIt").await, ReactPlan::ReactOnly);
    }

    #[tokio::test]
    async fn record_ack_and_is_acked_round_trip() {
        let t = ReactionTracker::default();
        assert!(!t.is_acked("om_1").await);
        t.record_ack("om_1", "Get".into(), "rid_eyes".into()).await;
        assert!(t.is_acked("om_1").await, "重复入站事件不得叠挂 ack");
    }

    #[tokio::test]
    async fn is_acked_returns_none_when_not_present() {
        let t = ReactionTracker::default();
        assert!(!t.is_acked("nonexistent").await);
    }

    #[tokio::test]
    async fn claim_user_mark_is_one_shot() {
        let t = ReactionTracker::default();
        assert!(t.claim_user_mark("om_1", "DONE").await, "首标放行");
        assert!(
            !t.claim_user_mark("om_1", "DONE").await,
            "同 emoji 重复 phase 事件不重复 react"
        );
        assert!(
            !t.claim_user_mark("om_1", "CrossMark").await,
            "已终态的消息不换标（首标即定）"
        );
        assert!(t.claim_user_mark("om_2", "DONE").await, "消息间互不影响");
    }
}
