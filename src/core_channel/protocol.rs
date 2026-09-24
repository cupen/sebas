//! Core session channel protocol (openspec/changes/add-core-session-channel,
//! tasks 4.1/4.2): newline-delimited JSON frames over a Unix stream socket.
//!
//! **类型已迁入 `sebas_ipc::protocol`**（unify-ipc-protocol-home 2.1）：本模块
//! 只剩原位 `pub use`，`sebas::core_channel::protocol::*` 这条既有公开路径与
//! 全部调用点零改动。协议之家的准入面与演进纪律见 `sebas-ipc` 的 crate 文档。
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
//!
//! 本模块的单元测试留在原地：它们从**根 crate 的视角**复核同一份线形状
//! （迁出前后逐字节一致，见 `tests/fixtures/ipc_core_channel_wire.json`）。

pub use sebas_ipc::protocol::{
    Attachment, ChannelHandshake, ChannelHandshakeAck, CoreChannelRequest, CoreChannelResponse,
    NodeLinkOp, NodeLinkOutcome, NodeView, PROTOCOL_VERSION, SessionStreamFrame, StateStreamFrame,
    WireFrame, decode_line, default_protocol_version, encode_line,
};

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_channels::ChannelKey;
    use sebas_domain::session::{
        PendingSubmission, PermissionDecision, PermissionNotice, SessionEvent, SessionInfo,
        TurnEntry,
    };
    use sebas_webui::session_backend::SessionRejection;
    use serde::{Deserialize, Serialize};

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: &T) {
        let json = serde_json::to_string(v).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, v, "round-trip mismatch for {json}");
    }

    /// （type-session-vocabularies 3.3，design D5）**webui → 后端**（core channel）
    /// 边界的发送集：四值决定原样过线，一个不多一个不少，`escalate` 仍带 `reason`。
    ///
    /// 合一类型最容易出的错是「静默扩大发送面」——本断言就是那道闸门：若这里少发
    /// 或多发一个取值，集合大小/拼写立刻对不上。反面演示见 change 的验证记录
    /// （临时让本边界多发 `escalate` 之外的取值 → 本测试失败）。
    #[test]
    fn approval_answer_send_set_is_unchanged() {
        let decisions = [
            PermissionDecision::AllowOnce,
            PermissionDecision::AllowSession,
            PermissionDecision::Deny,
            PermissionDecision::Escalate {
                reason: "needs human".into(),
            },
        ];
        let sent: Vec<String> = decisions
            .into_iter()
            .map(|decision| {
                serde_json::to_string(&CoreChannelRequest::ApprovalAnswer {
                    request_id: "toolu_1".into(),
                    decision,
                })
                .expect("serialize")
            })
            .collect();
        assert_eq!(sent.len(), 4, "四值决定必须两两序列化不同: {sent:?}");
        for spelling in ["allow_once", "allow_session", "deny", "escalate"] {
            assert!(
                sent.iter().any(|j| j.contains(&format!("\"{spelling}\""))),
                "webui → 后端边界少发了 `{spelling}`: {sent:?}"
            );
        }
        // `escalate` 的理由不得在过线时丢失。
        let escalate = serde_json::to_string(&CoreChannelRequest::ApprovalAnswer {
            request_id: "toolu_1".into(),
            decision: PermissionDecision::Escalate {
                reason: "why".into(),
            },
        })
        .unwrap();
        assert!(
            escalate.contains("\"why\""),
            "escalate 的 reason 过线时丢了: {escalate}"
        );
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
                mode: Some("allow".into()),
                agent: "claudecode".into(),
                node: None,
            },
            CoreChannelRequest::CreatePlaceholder {
                project_dir: Some("/tmp/p".into()),
                model: Some("m1".into()),
                mode: None,
                agent: "claudecode".into(),
                node: None,
            },
            CoreChannelRequest::SetSessionModel {
                key: key.clone(),
                model_id: "m2".into(),
            },
            CoreChannelRequest::Message {
                key: key.clone(),
                message: "msg".into(),
                attachments: vec![Attachment {
                    path: "/tmp/img.png".into(),
                    mime: Some("image/png".into()),
                    name: Some("img.png".into()),
                }],
            },
            CoreChannelRequest::EnsureMessage {
                key: key.clone(),
                message: "hello from im".into(),
                attachments: vec![],
            },
            CoreChannelRequest::Cancel { key: key.clone() },
            CoreChannelRequest::Close { key: key.clone() },
            CoreChannelRequest::RestoreSession {
                key: key.clone(),
                session_id: Some("old-1".into()),
                project_dir: Some("/proj".into()),
                transcript: vec![TurnEntry::prompt(0, "p")],
                identity: sebas_dispatch::SessionIdentity::default(),
                label: None,
                prompt_preview: None,
            },
            // fix-webui-approval-restore-and-session-identity：待批审批读模型
            // 与会话命名的 wire 往返。
            CoreChannelRequest::PendingApprovals { key: key.clone() },
            CoreChannelRequest::SetSessionLabel {
                key: key.clone(),
                label: Some("label-1".into()),
            },
            CoreChannelRequest::RemovePending {
                key: key.clone(),
                pending_id: 7,
            },
            CoreChannelRequest::MovePending {
                key: key.clone(),
                pending_id: 7,
                to_index: 1,
            },
            CoreChannelRequest::Turns {
                key: key.clone(),
                from: 3,
            },
            CoreChannelRequest::SetFocus {
                key: Some(key.clone()),
            },
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
            CoreChannelRequest::FetchModels {
                provider: "deepseek".into(),
            },
            CoreChannelRequest::StateSubscribe,
            CoreChannelRequest::ApprovalAnswer {
                request_id: "toolu_1".into(),
                decision: PermissionDecision::AllowOnce,
            },
        ];
        for r in &requests {
            roundtrip(r);
        }

        let responses = vec![
            CoreChannelResponse::Snapshot { sessions: vec![] },
            CoreChannelResponse::Spawned { key: key.clone() },
            CoreChannelResponse::Ok,
            CoreChannelResponse::Closed {
                discarded_pending: 2,
            },
            CoreChannelResponse::PendingList {
                pending: vec![PendingSubmission {
                    id: 7,
                    text: "queued".into(),
                    position: 0,
                    disposition: sebas_dispatch::PendingDisposition::Turn,
                    priority: true,
                }],
            },
            CoreChannelResponse::Turns {
                entries: vec![TurnEntry::prompt(0, "p"), TurnEntry::markdown(1, "m")],
            },
            CoreChannelResponse::Focused {
                key: Some(key.clone()),
            },
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
            CoreChannelResponse::Models {
                provider: "p".into(),
                models: vec!["m1".into()],
            },
        ];
        for r in &responses {
            roundtrip(r);
        }

        roundtrip(&ChannelHandshake::new("s3cret"));
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
            agent_kind: None,
            usage: None,
            backend: Some("native".into()),
            pending: Vec::new(),
            remote: None,
            desired_mode: sebas_dispatch::engine::ask_mode(),
            effective_mode: None,
            msg_count: 0,
            turn_engaged: false,
            spawn_failure_reason: None,
            parked_approvals: 0,
            label: None,
            available_commands: Vec::new(),
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
            SessionStreamFrame::ApprovalRequested {
                notice: PermissionNotice {
                    request_id: "toolu_9".into(),
                    session_id: "feishu%3Aagent-1".into(),
                    tool_name: "bash".into(),
                    args: serde_json::json!({"command": "ls"}),
                    reason: "policy ask".into(),
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

    /// wire-webui-sebas-agent-e2e 1.1 验收：旧客户端报文（Spawn 无 backend 字段）
    /// 仍可反序列化 —— additive 兼容。
    #[test]
    fn legacy_wire_shapes_still_deserialize() {
        let legacy_spawn = r#"{"cmd":"spawn","prompt":"hi","project_dir":null,"model":null}"#;
        // 旧帧（backend 词汇）不再可读——wire 收紧是本 change 的 BREAKING 面
        // （workbench-agent-wire-fix D2）：legacy Spawn 反序列化必须失败。
        let req: Result<CoreChannelRequest, _> = serde_json::from_str(legacy_spawn);
        assert!(req.is_err(), "legacy backend frame must be rejected");
        // 快照条目的旧格式（无 backend/current_model 字段）同样可反序列化。
        let legacy_info = r#"{"channel":"feishu","key":"oc_1","session_id":null,"status":"active","last_active_unix":0}"#;
        let info: SessionInfo = serde_json::from_str(legacy_info).unwrap();
        assert_eq!(info.current_model, None);
        assert_eq!(info.backend, None);
        assert_eq!(info.agent_kind, None);
    }
}
