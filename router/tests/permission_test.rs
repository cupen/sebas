use acp_claude::session::{AcpCommand, AcpEvent, Decision};
use feishu::events::{CardAction, FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::Mapping;
use router::state::SessionMap;
use std::time::Duration;

#[tokio::test]
async fn permission_request_emits_card_with_buttons() {
    let map = SessionMap::new();
    // The router resolves session_id -> SessionKey via the map, so seed it.
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

    let event = AcpEvent::PermissionRequest {
        session_id: "s1".into(),
        request_id: "r1".into(),
        tool_name: "Bash".into(),
        args: serde_json::json!({"cmd": "ls"}),
    };
    router.apply_event_to_out("s1".into(), &event).await;

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::SendCard { key, card, .. } => {
            assert_eq!(
                key.chat_id, "oc_x",
                "expected resolved SessionKey, got {key:?}"
            );
            let s = serde_json::to_string(&card).unwrap();
            assert!(
                s.contains("Allow once"),
                "missing 'Allow once' in card: {s}"
            );
            assert!(s.contains("Deny"), "missing 'Deny' in card: {s}");
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
}

#[tokio::test]
async fn button_callback_emits_permission_reply() {
    let map = SessionMap::new();
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    // on_button now requires a live session mapping before forwarding a reply.
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    // In-place flip (commit 658b312) requires a pre-recorded perm_card entry:
    // on_button takes it to flip the card to "已处理", then emits SendAcp. Without
    // this seed it sees no entry and emits "请求已过期" instead. Same recipe as
    // tests/permission_flow_test.rs:128.
    router
        .record_perm_card_msg_id(
            "r1".into(),
            key.clone(),
            "om_fake".into(),
            "Bash".into(),
            serde_json::json!({"cmd": "ls"}),
        )
        .await;
    let action = CardAction {
        session_id: "s1".into(),
        request_id: Some("r1".into()),
        decision: Some("allow_once".into()),
        value: serde_json::json!({ "decision": "allow_once" }),
    };

    router.dispatch(FeishuIn::ButtonCb { key, action }).await;

    // First Out is the in-place flip (UpdateCardByMsgId); drain until SendAcp.
    let out = loop {
        let got = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(got, Out::SendAcp { .. }) {
            break got;
        }
    };
    match out {
        Out::SendAcp {
            session_id,
            cmd:
                AcpCommand::PermissionReply {
                    session_id: sid,
                    request_id: rid,
                    decision,
                },
        } => {
            assert_eq!(session_id, "s1");
            assert_eq!(sid, "s1");
            assert_eq!(rid, "r1");
            assert!(matches!(decision, Decision::AllowOnce));
        }
        other => panic!("expected SendAcp PermissionReply, got {other:?}"),
    }
}

#[tokio::test]
async fn button_callback_on_dead_session_emits_help_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    let key = SessionKey {
        chat_id: "oc_gone".into(),
        thread_id: None,
    };
    let action = CardAction {
        session_id: "s_dead".into(),
        request_id: Some("r1".into()),
        decision: Some("allow_once".into()),
        value: serde_json::json!({ "decision": "allow_once" }),
    };

    router.dispatch(FeishuIn::ButtonCb { key, action }).await;

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::SendCard { key, card, .. } => {
            assert_eq!(key.chat_id, "oc_gone");
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("会话已结束"), "missing dead-session notice: {s}");
        }
        other => panic!("expected SendCard help, got {other:?}"),
    }
}

#[tokio::test]
async fn button_callback_unknown_decision_defaults_to_deny() {
    let map = SessionMap::new();
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    // In-place flip (commit 658b312) requires a pre-recorded perm_card entry;
    // see tests/permission_flow_test.rs:128 for the recipe.
    router
        .record_perm_card_msg_id(
            "r1".into(),
            key.clone(),
            "om_fake".into(),
            "Bash".into(),
            serde_json::json!({"cmd": "ls"}),
        )
        .await;
    let action = CardAction {
        session_id: "s1".into(),
        request_id: Some("r1".into()),
        decision: None, // malformed payload -> fail closed
        value: serde_json::json!({}),
    };
    router.dispatch(FeishuIn::ButtonCb { key, action }).await;
    // Drain the in-place flip (UpdateCardByMsgId) before SendAcp.
    let out = loop {
        let got = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(got, Out::SendAcp { .. }) {
            break got;
        }
    };
    match out {
        Out::SendAcp {
            cmd: AcpCommand::PermissionReply { decision, .. },
            ..
        } => assert!(matches!(decision, Decision::Deny)),
        other => panic!("expected SendAcp PermissionReply, got {other:?}"),
    }
}
