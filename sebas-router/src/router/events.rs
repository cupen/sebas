//! Session event stream + snapshot shapes for out-of-process observers
//! (openspec/changes/add-core-session-channel — tasks 1.1/1.3).
//!
//! `SessionInfo` is the externally visible view of one session: the mapping
//! state joined with the card-derived fields the WebUI renders. `SessionEvent`
//! is published on a bounded broadcast in `RouterHandle` for every mapping
//! mutation (create / status-or-phase change / removal). `TurnEntry` is one
//! block of a session's rendered transcript, addressed by a monotonic
//! position so channel clients can fetch only what they have not seen.
//!
//! These types are serde-native by design: they cross the core session
//! channel as newline-delimited JSON. (`CardElement` deliberately is not
//! `Serialize` — the transcript carries rendered view shapes instead.)

use serde::{Deserialize, Serialize};

/// One session as the outside world sees it: mapping state joined with the
/// card-derived fields the WebUI renders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInfo {
    pub chat_id: String,
    pub thread_id: Option<String>,
    /// Live routing id — `None` for Spawning placeholders.
    pub session_id: Option<String>,
    /// `"spawning"` | `"active"` | `"dormant"`.
    pub status: String,
    /// Card phase emoji (`SEED`/`OnIt`/`DONE`/`CrossMark`) when a card exists.
    pub phase: Option<String>,
    /// Current turn's user prompt, when a card exists.
    pub user_prompt: Option<String>,
    pub last_active_unix: i64,
    /// Working directory for project sessions (WebUI-spawned).
    pub project_dir: Option<String>,
}

/// Session change event published on the router's broadcast channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    /// A mapping was inserted (Spawning placeholder or restored Dormant).
    Created { session: SessionInfo },
    /// Status or phase changed (Spawning→Active, Dormant→Spawning resume,
    /// project_dir set, card emoji transition).
    Updated { session: SessionInfo },
    /// The mapping was removed (web close, terminal error, failed spawn).
    Removed {
        chat_id: String,
        thread_id: Option<String>,
    },
    /// Emitted by channel clients (never by the router itself) after a
    /// reconnect: subscribers should re-snapshot because the client resumed
    /// from a fresh snapshot and the view must converge. See the channel
    /// spec's "reconnect resumes from a snapshot" scenario.
    Resync,
}

/// One rendered block of a session's transcript, addressed by a monotonic
/// position. `kind` distinguishes the user's prompt from agent/tool output;
/// `element_type` tells the client how to render `content`
/// (`"markdown"` | `"thinking"`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnEntry {
    /// 0-based monotonic position within the session's transcript.
    pub position: u64,
    /// `"prompt"` (user turn input) or `"content"` (agent/tool output).
    pub kind: String,
    /// `"markdown"` | `"thinking"`.
    pub element_type: String,
    pub content: String,
}

impl TurnEntry {
    pub fn prompt(position: u64, content: impl Into<String>) -> Self {
        Self {
            position,
            kind: "prompt".into(),
            element_type: "markdown".into(),
            content: content.into(),
        }
    }

    pub fn markdown(position: u64, content: impl Into<String>) -> Self {
        Self {
            position,
            kind: "content".into(),
            element_type: "markdown".into(),
            content: content.into(),
        }
    }

    pub fn thinking(position: u64, content: impl Into<String>) -> Self {
        Self {
            position,
            kind: "content".into(),
            element_type: "thinking".into(),
            content: content.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_event_round_trips_through_serde() {
        // 1.1 验收：每个变体经 serde 往返后与原值一致。
        let info = SessionInfo {
            chat_id: "oc_1".into(),
            thread_id: Some("om_t".into()),
            session_id: Some("s1".into()),
            status: "active".into(),
            phase: Some("DONE".into()),
            user_prompt: Some("hello".into()),
            last_active_unix: 1234,
            project_dir: Some("/tmp/p".into()),
        };
        let cases = vec![
            SessionEvent::Created {
                session: info.clone(),
            },
            SessionEvent::Updated { session: info },
            SessionEvent::Removed {
                chat_id: "oc_2".into(),
                thread_id: None,
            },
            SessionEvent::Resync,
        ];
        for ev in cases {
            let json = serde_json::to_string(&ev).expect("serialize");
            let back: SessionEvent = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, ev, "round-trip mismatch for {json}");
        }
    }

    #[test]
    fn session_event_uses_type_tag() {
        // wire 形态带 "type" tag，与 control RPC 的 cmd tag 姿态一致。
        let ev = SessionEvent::Removed {
            chat_id: "oc_x".into(),
            thread_id: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "removed");
    }

    #[test]
    fn turn_entry_round_trips_through_serde() {
        let e = TurnEntry::prompt(3, "fix the bug");
        let back: TurnEntry =
            serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }
}
