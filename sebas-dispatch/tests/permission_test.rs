use sebas_acp::claude::session::{AcpCommand, AcpEvent, Decision};
use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};
use sebas_dispatch::engine::{Out, DispatchHandle};
use sebas_dispatch::state::Mapping;
use sebas_dispatch::state::SessionMap;
use std::time::Duration;

#[tokio::test]
async fn permission_request_emits_card_with_buttons() {
    let map = SessionMap::new();
    // The router resolves session_id -> SessionKey via the map, so seed it.
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

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
                key.reference, "oc_x",
                "expected resolved ChannelKey, got {key:?}"
            );
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("本次允许"), "missing '本次允许' in card: {s}");
            assert!(s.contains("拒绝"), "missing '拒绝' in card: {s}");
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
}

#[tokio::test]
async fn permission_card_in_topic_leaves_root_id_none() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_topic", Some("omt_t1"));
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

    // 入站话题消息写入 reply target（话题根消息 message_id，events 层归一化）。
    router
        .dispatch(ChannelEvent::Text { key: key.clone(), text: "hello".into(), reply_target: Some("om_topic_root".into()) })
        .await;

    let event = AcpEvent::PermissionRequest {
        session_id: "s1".into(),
        request_id: "r1".into(),
        tool_name: "Bash".into(),
        args: serde_json::json!({"cmd": "ls"}),
    };
    router.apply_event_to_out("s1".into(), &event).await;

    // 排掉 continue 产生的 per-turn card / SendAcp，只取权限卡（perm_request_id
    // 标记它）。权限卡的 root_id 由 dispatch 层 topic_reply_target 兜底，router
    // 层不再预填 → 恒为 None（F3）。
    let perm_card = loop {
        let out = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
            .await
            .expect("permission card not received in time")
            .expect("channel closed");
        match out {
            Out::SendCard {
                perm_request_id: Some(_),
                key: k,
                root_id,
                ..
            } => break (k, root_id),
            _ => continue,
        }
    };
    assert_eq!(perm_card.0.reference, "oc_topic\0omt_t1");
    assert_eq!(
        perm_card.1, None,
        "话题内权限卡 root_id 恒为 None：话题聚合由 dispatch 层兜底"
    );
}

#[tokio::test]
async fn permission_card_mainline_keeps_root_id_none() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

    router
        .dispatch(ChannelEvent::Text { key: key.clone(), text: "hello".into(), reply_target: Some("om_msg".into()) })
        .await;

    let event = AcpEvent::PermissionRequest {
        session_id: "s1".into(),
        request_id: "r1".into(),
        tool_name: "Bash".into(),
        args: serde_json::json!({"cmd": "ls"}),
    };
    router.apply_event_to_out("s1".into(), &event).await;

    let perm_card = loop {
        let out = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
            .await
            .expect("permission card not received in time")
            .expect("channel closed");
        match out {
            Out::SendCard {
                perm_request_id: Some(_),
                root_id,
                ..
            } => break root_id,
            _ => continue,
        }
    };
    assert_eq!(perm_card, None, "主线权限卡保持现状：root_id=None（Q7）");
}

#[tokio::test]
async fn button_callback_emits_permission_reply() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    // on_button now requires a live session mapping before forwarding a reply.
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
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
    let action = ChannelAction {
        session_id: "s1".into(),
        request_id: Some("r1".into()),
        decision: Some("allow_once".into()),
        value: serde_json::json!({ "decision": "allow_once" }),
    };

    router
        .dispatch(ChannelEvent::ButtonCb {
            key,
            action,
                    })
        .await;

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
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    let key = ChannelKey::feishu("oc_gone", None);
    let action = ChannelAction {
        session_id: "s_dead".into(),
        request_id: Some("r1".into()),
        decision: Some("allow_once".into()),
        value: serde_json::json!({ "decision": "allow_once" }),
    };

    router
        .dispatch(ChannelEvent::ButtonCb {
            key,
            action,
                    })
        .await;

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::SendCard { key, card, .. } => {
            assert_eq!(key.reference, "oc_gone");
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("会话已结束"), "missing dead-session notice: {s}");
        }
        other => panic!("expected SendCard help, got {other:?}"),
    }
}

#[tokio::test]
async fn button_callback_unknown_decision_defaults_to_deny() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
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
    let action = ChannelAction {
        session_id: "s1".into(),
        request_id: Some("r1".into()),
        decision: None, // malformed payload -> fail closed
        value: serde_json::json!({}),
    };
    router
        .dispatch(ChannelEvent::ButtonCb {
            key,
            action,
                    })
        .await;
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
