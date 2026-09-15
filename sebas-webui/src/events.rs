//! Real-time events pushed to WebUI clients over the WebSocket channel.

use sebas_dispatch::{PendingDisposition, TurnEntry};
use serde::Serialize;

/// （workbench-turn-queue 7.3）一次性「未执行」提示的条目形状：id + 文本 +
/// 处置 + 优先标记。position 不随提示下发（提示只点名哪些提交没有执行）。
#[derive(Debug, Clone, Serialize)]
pub struct PendingSubmissionView {
    pub id: u64,
    pub text: String,
    pub disposition: PendingDisposition,
    pub priority: bool,
}

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
    /// （workbench-turn-queue 5.2/7.3）会话终结时未执行的待生效提交，逐条
    /// 列出（id/文本/处置/优先）。在 session.removed 帧之前到达。
    #[serde(rename = "session.pending_dropped")]
    SessionPendingDropped {
        session_id: String,
        dropped: Vec<PendingSubmissionView>,
    },
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
    /// 实时回合内容（workbench-live-conversation-flow 2.1）：同一合并窗内
    /// 某会话追加的 transcript 条目（落库序、position 单调）。`seq` 是本帧
    /// 最后一条的 position（前端的去重锚）。纯增量补充：乱序/迟到/丢失由
    /// 客户端以快照重取收敛，不参与重放。
    #[serde(rename = "turn.append")]
    TurnAppend {
        session_id: String,
        entries: Vec<TurnEntry>,
        seq: u64,
    },
}

#[cfg(test)]
mod tests {

    use super::WebUiEvent;
    use serde_json::json;
    use sebas_dispatch::TurnEntry;

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
            (
                WebUiEvent::SessionPendingDropped {
                    session_id: "oc_b".into(),
                    dropped: vec![crate::events::PendingSubmissionView {
                        id: 5,
                        text: "never ran".into(),
                        disposition: sebas_dispatch::PendingDisposition::Turn,
                        priority: true,
                    }],
                },
                json!({
                    "type": "session.pending_dropped",
                    "session_id": "oc_b",
                    "dropped": [
                        {"id": 5, "text": "never ran", "disposition": "turn", "priority": true}
                    ]
                }),
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

    /// workbench-live-conversation-flow 2.1：turn.append 帧带 dotted type、
    /// encoded session id 与条目数组；seq = 最后一条的 position。
    #[test]
    fn turn_append_serializes_with_dotted_type_tag() {
        let event = WebUiEvent::TurnAppend {
            session_id: "web%00web-1".into(),
            entries: vec![
                TurnEntry::markdown(3, "hello "),
                TurnEntry::markdown(4, "world"),
            ],
            seq: 4,
        };
        let got = serde_json::to_value(&event).unwrap();
        assert_eq!(got["type"], "turn.append");
        assert_eq!(got["session_id"], "web%00web-1");
        assert_eq!(got["seq"], 4);
        assert_eq!(got["entries"].as_array().unwrap().len(), 2);
        assert_eq!(got["entries"][0]["content"], "hello ");
    }
}
