//! Real-time events pushed to WebUI clients over the WebSocket channel.

use serde::Serialize;

/// Events that the WebUI can push to connected clients.
///
/// Each event serializes to a JSON object with a `type` tag, so a single
/// WebSocket text frame carries a complete, self-describing message. Names
/// are dotted (`session.created`), replacing the former SSE two-part
/// `event: update` encoding.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WebUiEvent {
    /// A new session was created.
    #[serde(rename = "session.created")]
    SessionCreated { session_id: String },
    /// A session's state was updated.
    #[serde(rename = "session.updated")]
    SessionUpdated { session_id: String, status: String },
    /// A session was removed.
    #[serde(rename = "session.removed")]
    SessionRemoved { session_id: String },
    /// Configuration was updated. No sender exists yet; the variant is
    /// reserved so clients must tolerate it (and unknown types) arriving.
    #[serde(rename = "config.updated")]
    ConfigUpdated,
    /// A gated tool call awaits an operator decision (the review card).
    /// `args` carries the call's arguments verbatim; the client answers via
    /// `POST /api/permissions/{request_id}/answer`.
    #[serde(rename = "permission.requested")]
    PermissionRequested {
        request_id: String,
        session_id: String,
        tool_name: String,
        args: serde_json::Value,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::WebUiEvent;
    use serde_json::json;

    /// Every event serializes to a JSON object tagged with its dotted
    /// `type`; this shape is the WS contract clients key off.
    #[test]
    fn events_serialize_with_dotted_type_tag() {
        let cases: Vec<(WebUiEvent, serde_json::Value)> = vec![
            (
                WebUiEvent::SessionCreated {
                    session_id: "oc_a".into(),
                },
                json!({"type": "session.created", "session_id": "oc_a"}),
            ),
            (
                WebUiEvent::SessionUpdated {
                    session_id: "oc_a".into(),
                    status: "active".into(),
                },
                json!({"type": "session.updated", "session_id": "oc_a", "status": "active"}),
            ),
            (
                WebUiEvent::SessionRemoved {
                    session_id: "oc_b".into(),
                },
                json!({"type": "session.removed", "session_id": "oc_b"}),
            ),
            (WebUiEvent::ConfigUpdated, json!({"type": "config.updated"})),
            (
                WebUiEvent::PermissionRequested {
                    request_id: "req1".into(),
                    session_id: "oc_a".into(),
                    tool_name: "bash".into(),
                    args: json!({"command": "rm -rf build"}),
                    reason: "may modify state".into(),
                },
                json!({
                    "type": "permission.requested",
                    "request_id": "req1",
                    "session_id": "oc_a",
                    "tool_name": "bash",
                    "args": {"command": "rm -rf build"},
                    "reason": "may modify state"
                }),
            ),
        ];
        for (event, want) in cases {
            let got = serde_json::to_value(&event).unwrap();
            assert_eq!(got, want, "wrong JSON shape for {want}");
        }
    }
}
