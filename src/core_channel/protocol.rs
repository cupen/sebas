//! Core session channel protocol (openspec/changes/add-core-session-channel,
//! tasks 4.1/4.2): newline-delimited JSON frames over a Unix stream socket.
//!
//! Mirrors `RpcControlRequest`'s serde shape (`cmd` tag on requests, `cmd`
//! tag on responses). Session data types (`SessionInfo`, `SessionEvent`,
//! `TurnEntry`, `SessionRejection`) come from the router/webui crates so the
//! wire types and the trait types can never drift apart.
//!
//! ## Request/response (one connection per mutation)
//!
//! 1. client → server: one line of JSON, `CoreChannelRequest` (with the
//!    shared secret already checked in the handshake line before it).
//! 2. server → client: one line of JSON, `CoreChannelResponse`
//!    (`accepted` or `rejected` with the typed rejection).
//!
//! ## Subscription (one dedicated streaming connection)
//!
//! 1. client sends `CoreChannelRequest::Subscribe`.
//! 2. server → client: `SessionStreamFrame::Snapshot` (the state at subscribe
//!    time), then `SessionStreamFrame::Event` for every session event as it
//!    happens. The snapshot comes before any event (spec: "snapshot then
//!    subscribe" ordering is server-side subscribe-first + snapshot, so a
//!    mutation racing the subscribe is captured by the snapshot — no gap; the
//!    events it also produced are idempotent full-state updates — no visible
//!    duplicate). A lagging subscriber is dropped (connection closed) rather
//!    than delivered a gap; the client re-snapshots on reconnect.

use sebas_channels::ChannelKey;
use sebas_router::{SessionEvent, SessionInfo, TurnEntry};
use serde::{Deserialize, Serialize};

/// One request over the core session channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CoreChannelRequest {
    /// Full external session snapshot.
    Snapshot,
    /// Create a session (optionally rooted in a project directory).
    Spawn {
        prompt: String,
        project_dir: Option<String>,
        /// （add-acp-model-selection）创建时请求的模型 id（None = 默认模型）。
        #[serde(default)]
        model: Option<String>,
    },
    /// 中程切换会话模型（add-acp-model-selection）：`session/set_config_option`。
    SetSessionModel { key: ChannelKey, model_id: String },
    /// Send a message to an existing session.
    Message { key: ChannelKey, message: String },
    /// Close (kill) a session.
    Close { key: ChannelKey },
    /// Fetch rendered transcript content at/after a monotonic position.
    Turns { key: ChannelKey, from: u64 },
    /// Mark the focused session.
    SetFocus { key: Option<ChannelKey> },
    /// Ask for the focused session.
    Focused,
    /// Start the event stream (see module docs for the frame order).
    Subscribe,
    /// Snapshot a domain of the core state store (add-state-store).
    StateSnapshot { domain: String },
    /// Mutate a domain of the core state store (add-state-store).
    StateMutation { domain: String, payload: serde_json::Value },
    /// Subscribe to state change notifications (add-state-store).
    StateSubscribe,
}

/// One response over the core session channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CoreChannelResponse {
    /// Snapshot result.
    Snapshot { sessions: Vec<SessionInfo> },
    /// Spawn result: the new session key.
    Spawned { key: ChannelKey },
    /// Message/close/focus accepted; nothing to return.
    Ok,
    /// Turn-content result.
    Turns { entries: Vec<TurnEntry> },
    /// Focused-session result.
    Focused { key: Option<ChannelKey> },
    /// Typed rejection — names the reason; nothing was mutated.
    Rejected { #[serde(flatten)] rejection: sebas_webui::session_backend::SessionRejection },
    /// State snapshot result (add-state-store).
    StateSnapshot { domain: String, payload: serde_json::Value },
    /// State mutation accepted.
    StateMutationOk,
}

/// One frame of the subscription stream (task 4.2): exactly one snapshot
/// frame first, then event frames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "frame", rename_all = "snake_case")]
pub enum SessionStreamFrame {
    Snapshot { sessions: Vec<SessionInfo> },
    Event { event: SessionEvent },
}

/// The handshake line sent by the client immediately after connecting,
/// before any request. Wrong/absent secret → the server closes the
/// connection without reading a request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelHandshake {
    pub secret: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_webui::session_backend::SessionRejection;

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
        v: &T,
    ) {
        let json = serde_json::to_string(v).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, v, "round-trip mismatch for {json}");
    }

    /// 4.1 验收：每个请求/响应变体经 serde 往返后与原值一致。
    #[test]
    fn every_request_and_response_variant_round_trips() {
        let key = ChannelKey::feishu("oc_1", Some("om_t"));
        let requests = vec![
            CoreChannelRequest::Snapshot,
            CoreChannelRequest::Spawn {
                prompt: "hello".into(),
                project_dir: Some("/tmp/p".into()),
                model: Some("m1".into()),
            },
            CoreChannelRequest::SetSessionModel {
                key: key.clone(),
                model_id: "m2".into(),
            },
            CoreChannelRequest::Message {
                key: key.clone(),
                message: "msg".into(),
            },
            CoreChannelRequest::Close { key: key.clone() },
            CoreChannelRequest::Turns {
                key: key.clone(),
                from: 3,
            },
            CoreChannelRequest::SetFocus { key: Some(key.clone()) },
            CoreChannelRequest::SetFocus { key: None },
            CoreChannelRequest::Focused,
            CoreChannelRequest::Subscribe,
            CoreChannelRequest::StateSnapshot {
                domain: "providers".into(),
            },
            CoreChannelRequest::StateMutation {
                domain: "settings".into(),
                payload: serde_json::json!({"key": "card_config", "value": {}}),
            },
            CoreChannelRequest::StateSubscribe,
        ];
        for r in &requests {
            roundtrip(r);
        }

        let responses = vec![
            CoreChannelResponse::Snapshot { sessions: vec![] },
            CoreChannelResponse::Spawned { key: key.clone() },
            CoreChannelResponse::Ok,
            CoreChannelResponse::Turns {
                entries: vec![TurnEntry::prompt(0, "p"), TurnEntry::markdown(1, "m")],
            },
            CoreChannelResponse::Focused { key: Some(key.clone()) },
            CoreChannelResponse::Rejected {
                rejection: SessionRejection::UnknownSession { key: "k".into() },
            },
            CoreChannelResponse::Rejected {
                rejection: SessionRejection::UnusableProjectDir,
            },
            CoreChannelResponse::Rejected {
                rejection: SessionRejection::Capacity { limit: 8 },
            },
            CoreChannelResponse::Rejected {
                rejection: SessionRejection::Unavailable { cause: "c".into() },
            },
            CoreChannelResponse::StateSnapshot {
                domain: "providers".into(),
                payload: serde_json::json!({"providers": {}}),
            },
            CoreChannelResponse::StateMutationOk,
        ];
        for r in &responses {
            roundtrip(r);
        }

        roundtrip(&ChannelHandshake {
            secret: "s3cret".into(),
        });
    }

    /// 4.2 验收：一个 snapshot 帧后跟 event 帧，按原序解析回同一序列。
    #[test]
    fn stream_frame_parses_back_in_order() {
        let info = SessionInfo {
            channel: "feishu".into(),
            key: "oc_1".into(),
            session_id: Some("s1".into()),
            status: "active".into(),
            phase: None,
            user_prompt: None,
            last_active_unix: 0,
            project_dir: None,
            current_model: None,
            available_models: None,
        };
        let frames = vec![
            SessionStreamFrame::Snapshot {
                sessions: vec![info.clone()],
            },
            SessionStreamFrame::Event {
                event: SessionEvent::Updated {
                    session: info.clone(),
                },
            },
            SessionStreamFrame::Event {
                event: SessionEvent::Removed {
                    channel: "feishu".into(),
                    key: "oc_1".into(),
                },
            },
            SessionStreamFrame::Event {
                event: SessionEvent::Resync,
            },
        ];
        let mut parsed = Vec::new();
        for f in &frames {
            let json = serde_json::to_string(f).unwrap();
            parsed.push(serde_json::from_str::<SessionStreamFrame>(&json).unwrap());
        }
        assert_eq!(parsed, frames);
        // The wire shape carries the "frame" tag.
        assert_eq!(
            serde_json::to_value(&frames[0]).unwrap()["frame"],
            "snapshot"
        );
        assert_eq!(serde_json::to_value(&frames[3]).unwrap()["frame"], "event");
    }
}
