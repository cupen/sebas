//! Tracks the current emoji reaction on each session's root card so the
//! router's phase machine can **swap** reactions (EYES->OnIt->DONE) rather
//! than pile them up. Feishu's unreact API needs the `reaction_id` returned
//! by the `react` call, so `react` returns it and we stash it here.
//!
//! Also tracks ack reactions (immediate "received" emoji on user messages)
//! keyed by Feishu message_id, so the phase reaction handler can clean up
//! the ack emoji before adding the new one.
//!
//! The *when* (which emoji on which phase transition) lives in the router;
//! this struct owns only the *what* (the id bookkeeping) and the swap plan.

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
    /// Ack reactions keyed by Feishu message_id (not session_id). The phase
    /// reaction handler consumes these so EYES gets removed before the new
    /// phase emoji is added.
    ack_map: Mutex<HashMap<String, (String, String)>>, // message_id -> (emoji, reaction_id)
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

    /// Record an ack reaction keyed by Feishu message_id, so the phase
    /// reaction handler can later remove it before adding the new emoji.
    pub async fn record_ack(&self, message_id: &str, emoji: String, reaction_id: String) {
        self.ack_map
            .lock()
            .await
            .insert(message_id.into(), (emoji, reaction_id));
    }

    /// Take (remove and return) the ack reaction for the given message_id,
    /// if one exists. Returns the emoji and reaction_id.
    pub async fn take_ack(&self, message_id: &str) -> Option<(String, String)> {
        self.ack_map.lock().await.remove(message_id)
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
    async fn record_ack_and_take_ack_round_trip() {
        let t = ReactionTracker::default();
        t.record_ack("om_1", "EYES".into(), "rid_eyes".into()).await;
        let taken = t.take_ack("om_1").await;
        assert_eq!(taken, Some(("EYES".into(), "rid_eyes".into())));
        // Second take returns None
        assert!(t.take_ack("om_1").await.is_none());
    }

    #[tokio::test]
    async fn take_ack_returns_none_when_not_present() {
        let t = ReactionTracker::default();
        assert!(t.take_ack("nonexistent").await.is_none());
    }
}
