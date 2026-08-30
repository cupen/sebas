//! Session lifecycle events and snapshots broadcast by the router.
//!
//! The router is the single authority on session existence and state. Every
//! mapping mutation it performs publishes a [`SessionEvent`] on a bounded
//! broadcast channel ([`RouterHandle::session_events`]), so detached
//! frontends — the WebUI today, any channel client tomorrow — can converge
//! on the router's own view without polling. [`SessionSnapshot`] carries the
//! fields a session row needs (identity, state, phase, recency, project
//! directory); derived presentation (encoded keys, status words, relative
//! times) stays with the consumer.

use crate::state::MappingState;
use sebas_feishu::events::SessionKey;
use serde::{Deserialize, Serialize};

/// Coarse lifecycle state of a session, mirroring `MappingState` without its
/// internal queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Placeholder inserted while a child process is being spawned.
    Spawning,
    /// A live child process exists (`session_id` is routable).
    Active,
    /// Restored from the state file; the id is known but no child is alive.
    Dormant,
}

impl From<&MappingState> for SessionState {
    fn from(state: &MappingState) -> Self {
        match state {
            MappingState::Spawning { .. } => Self::Spawning,
            MappingState::Active { .. } => Self::Active,
            MappingState::Dormant { .. } => Self::Dormant,
        }
    }
}

/// Everything a session row needs to know about one session. `phase` is the
/// card-state emoji (`""` before the first ACP event) that drives the
/// derived status (working/done/failed) on top of `state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub key: SessionKey,
    pub session_id: Option<String>,
    pub state: SessionState,
    pub phase: String,
    pub last_active_unix: i64,
    pub project_dir: Option<String>,
}

/// A session lifecycle event, published on every mapping mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    /// A mapping came into existence (fresh key, or a key replaced by `/new`).
    Created { session: SessionSnapshot },
    /// An existing mapping changed state, phase, recency, or project dir.
    Updated { session: SessionSnapshot },
    /// The mapping (and its queued turns) is gone.
    Removed { key: SessionKey },
}

#[cfg(test)]
mod tests {
    use super::{SessionEvent, SessionSnapshot, SessionState};

    fn snapshot() -> SessionSnapshot {
        SessionSnapshot {
            key: sebas_feishu::events::SessionKey {
                chat_id: "oc_chat".into(),
                thread_id: Some("om_thread".into()),
            },
            session_id: Some("ses_1".into()),
            state: SessionState::Active,
            phase: "OnIt".into(),
            last_active_unix: 1_700_000_000,
            project_dir: Some("/tmp/proj".into()),
        }
    }

    /// Each variant must survive a serde round-trip unchanged — channel
    /// clients re-hydrate these frames verbatim.
    #[test]
    fn every_variant_round_trips_through_serde() {
        let cases = vec![
            SessionEvent::Created {
                session: snapshot(),
            },
            SessionEvent::Updated {
                session: snapshot(),
            },
            SessionEvent::Removed {
                key: sebas_feishu::events::SessionKey {
                    chat_id: "oc_x".into(),
                    thread_id: None,
                },
            },
        ];
        for event in cases {
            let json = serde_json::to_string(&event).unwrap();
            let back: SessionEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(back, event, "round-trip changed {json}");
        }
    }

    /// The wire shape is tagged and snake_case — pinned so a protocol change
    /// is a conscious one.
    #[test]
    fn wire_shape_is_tagged_snake_case() {
        let event = SessionEvent::Removed {
            key: sebas_feishu::events::SessionKey {
                chat_id: "oc_x".into(),
                thread_id: Some("t".into()),
            },
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "removed");
        // SessionKey serializes as its "chat\0thread" string form.
        assert_eq!(value["key"], "oc_x\0t");
    }
}
