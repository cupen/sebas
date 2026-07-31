//! Tracks the current emoji reaction on each session's root card so the
//! router's phase machine can **swap** reactions (👀→🚧→✅) rather than
//! pile them up. Feishu's unreact API needs the `reaction_id` returned by
//! the react call, so `react` returns it and we stash it here.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn plan_react_only_when_no_current_reaction() {
        let t = ReactionTracker::default();
        assert_eq!(t.plan("s1", "🚧").await, ReactPlan::ReactOnly);
    }

    #[tokio::test]
    async fn plan_skip_when_same_emoji_already_current() {
        let t = ReactionTracker::default();
        t.record("s1", "🚧".into(), "rid_1".into()).await;
        assert_eq!(t.plan("s1", "🚧").await, ReactPlan::Skip);
    }

    #[tokio::test]
    async fn plan_swap_with_old_reaction_id_when_emoji_changes() {
        let t = ReactionTracker::default();
        t.record("s1", "👀".into(), "rid_eyes".into()).await;
        assert_eq!(
            t.plan("s1", "🚧").await,
            ReactPlan::Swap {
                unreact_id: "rid_eyes".into()
            }
        );
    }

    #[tokio::test]
    async fn record_updates_current_so_next_plan_skips() {
        let t = ReactionTracker::default();
        t.record("s1", "👀".into(), "rid_1".into()).await;
        // after swapping to 🚧 and recording, a duplicate 🚧 must skip
        t.record("s1", "🚧".into(), "rid_2".into()).await;
        assert_eq!(t.plan("s1", "🚧").await, ReactPlan::Skip);
    }

    #[tokio::test]
    async fn sessions_are_isolated() {
        let t = ReactionTracker::default();
        t.record("s1", "🚧".into(), "rid_s1".into()).await;
        assert_eq!(t.plan("s2", "🚧").await, ReactPlan::ReactOnly);
    }
}
