//! 路由分支补测（覆盖率目标 router/ ≥ 90% 的补齐测试）：on_button 死会话/
//! 缺 rid/未知 decision，slash 转发臂，Media 组合 prompt，dispatch_acp_event
//! 全事件类型，MsgIdMap 存取，terminal error 清理。

use acp_claude::session::{AcpCommand, AcpEvent, Decision};
use feishu::events::{CardAction, FeishuIn, SessionKey};
use router::router::{compose_media_prompt, Out, RouterHandle};
use router::state::{Mapping, SessionMap};
use std::time::Duration;

fn key() -> SessionKey {
    SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    }
}

async fn next_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    tokio::time::timeout(Duration::from_millis(300), rx.recv())
        .await
        .expect("out within 300ms")
        .expect("channel open")
}

// ---------- on_button 分支 ----------

#[tokio::test]
async fn button_cb_dead_session_gets_dead_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::ButtonCb {
            key: key(),
            action: CardAction {
                session_id: "ghost".into(),
                request_id: Some("r1".into()),
                decision: Some("allow_once".into()),
                value: serde_json::Value::Null,
            },
        })
        .await;
    match next_out(&mut out_rx).await {
        Out::SendCard { card, .. } => {
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("已结束") || s.contains("无效"), "dead card: {s}");
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
}

#[tokio::test]
async fn button_cb_missing_request_id_gets_help() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::ButtonCb {
            key: key(),
            action: CardAction {
                session_id: "s1".into(),
                request_id: None,
                decision: Some("allow_once".into()),
                value: serde_json::Value::Null,
            },
        })
        .await;
    assert!(matches!(next_out(&mut out_rx).await, Out::HelpText { .. }));
}

#[tokio::test]
async fn button_cb_unknown_decision_fails_closed_to_deny() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::ButtonCb {
            key: key(),
            action: CardAction {
                session_id: "s1".into(),
                request_id: Some("r9".into()),
                decision: Some("yolo".into()),
                value: serde_json::Value::Null,
            },
        })
        .await;
    match next_out(&mut out_rx).await {
        Out::SendAcp { cmd, .. } => match cmd {
            AcpCommand::PermissionReply {
                decision,
                request_id,
                ..
            } => {
                assert!(matches!(decision, Decision::Deny));
                assert_eq!(request_id, "r9");
            }
            other => panic!("expected PermissionReply, got {other:?}"),
        },
        other => panic!("expected SendAcp, got {other:?}"),
    }
}

// ---------- slash 转发臂 ----------

#[tokio::test]
async fn slash_compact_cost_cancel_forward_to_live_session() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);

    for (text, expect) in [("/compact", "/compact"), ("/cost", "/cost")] {
        router
            .dispatch(FeishuIn::Text {
                key: key(),
                text: text.into(),
                reply_to: None,
            })
            .await;
        match next_out(&mut out_rx).await {
            Out::SendAcp {
                cmd: AcpCommand::ContinueSession { session_id, prompt },
                ..
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(prompt, expect);
            }
            other => panic!("expected ContinueSession for {text}, got {other:?}"),
        }
    }

    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "/cancel".into(),
            reply_to: None,
        })
        .await;
    match next_out(&mut out_rx).await {
        Out::SendAcp {
            cmd: AcpCommand::Cancel { session_id },
            ..
        } => assert_eq!(session_id, "s1"),
        other => panic!("expected Cancel, got {other:?}"),
    }
}

#[tokio::test]
async fn slash_compact_without_session_gets_help() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "/compact".into(),
            reply_to: None,
        })
        .await;
    assert!(matches!(next_out(&mut out_rx).await, Out::HelpText { .. }));
}

// ---------- Media 组合 ----------

#[tokio::test]
async fn media_message_composes_prompt_and_spawns() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::Media {
            key: key(),
            files: vec!["/tmp/a.png".into(), "/tmp/b.pdf".into()],
            caption: Some("看这两张".into()),
        })
        .await;
    match next_out(&mut out_rx).await {
        Out::SpawnAcp { prompt, .. } => {
            assert!(prompt.contains("看这两张"), "caption in prompt: {prompt}");
            assert!(
                prompt.contains("[attached: /tmp/a.png, /tmp/b.pdf]"),
                "files in prompt: {prompt}"
            );
        }
        other => panic!("expected SpawnAcp, got {other:?}"),
    }
}

#[test]
fn compose_media_prompt_handles_empty_caption() {
    let p = compose_media_prompt("", &["/x".to_string()]);
    assert_eq!(p, "\n[attached: /x]");
    let p2 = compose_media_prompt("cap", &[]);
    assert!(p2.starts_with("cap\n"));
}

// ---------- dispatch_acp_event 全类型 ----------

#[tokio::test]
async fn dispatch_acp_event_routes_every_variant() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    router.seed_card("s1".into(), "p".into()).await;

    let events = vec![
        AcpEvent::TextDelta {
            session_id: "s1".into(),
            delta: "d".into(),
        },
        AcpEvent::ThinkingDelta {
            session_id: "s1".into(),
            delta: "t".into(),
        },
        AcpEvent::ToolStart {
            session_id: "s1".into(),
            tool_name: "Bash".into(),
            args: serde_json::Value::Null,
        },
        AcpEvent::ToolProgress {
            session_id: "s1".into(),
            tool_name: "Bash".into(),
            progress: "p".into(),
        },
        AcpEvent::ToolEnd {
            session_id: "s1".into(),
            tool_name: "Bash".into(),
            result: "r".into(),
        },
        AcpEvent::Error {
            session_id: "s1".into(),
            message: "warn".into(),
            terminal: false,
        },
    ];
    for evt in events {
        router.dispatch_acp_event(evt).await;
        let out = next_out(&mut out_rx).await;
        assert!(
            matches!(out, Out::UpdateCard { .. }),
            "UpdateCard, got {out:?}"
        );
    }
}

#[tokio::test]
async fn dispatch_acp_event_permission_request_sends_card() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: "s1".into(),
            request_id: "r1".into(),
            tool_name: "Write".into(),
            args: serde_json::json!({}),
        })
        .await;
    assert!(matches!(next_out(&mut out_rx).await, Out::SendCard { .. }));
}

#[tokio::test]
async fn permission_request_without_key_is_dropped() {
    // session 无对应 SessionKey 时权限卡丢弃（warn 日志路径）。
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: "ghost".into(),
            request_id: "r1".into(),
            tool_name: "Write".into(),
            args: serde_json::json!({}),
        })
        .await;
    let r = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
    assert!(r.is_err(), "no Out may be emitted for unknown session");
}

#[tokio::test]
async fn terminal_error_removes_mapping_and_drops_card() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    router.seed_card("s1".into(), "p".into()).await;
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s1".into(),
            message: "boom".into(),
            terminal: true,
        })
        .await;
    let out = next_out(&mut out_rx).await;
    assert!(matches!(out, Out::UpdateCard { .. }));
    assert!(map.get(&key()).await.is_none(), "mapping removed");
    // CardState 已清：后续 flush 是 no-op。
    router.flush_card("s1").await;
    let r = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
    assert!(r.is_err());
}

// ---------- MsgIdMap / insert_mapping / fail_spawn 杂项 ----------

#[tokio::test]
async fn root_msg_id_round_trip_via_handle() {
    let map = SessionMap::new();
    let (router, _rx) = RouterHandle::new(map);
    assert!(router.root_msg_id("s1").await.is_none());
    router.record_root_msg_id("s1".into(), "om_1".into()).await;
    assert_eq!(router.root_msg_id("s1").await.as_deref(), Some("om_1"));
}

#[tokio::test]
async fn insert_mapping_marks_alive_and_routes() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    assert!(!router.session_alive(&key()).await);
    router.insert_mapping(key(), "s7".into()).await;
    assert!(router.session_alive(&key()).await);
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "yo".into(),
            reply_to: None,
        })
        .await;
    match next_out(&mut out_rx).await {
        Out::SendAcp { session_id, .. } => assert_eq!(session_id, "s7"),
        other => panic!("expected SendAcp, got {other:?}"),
    }
}

#[tokio::test]
async fn fail_spawn_ignores_active_and_missing_entries() {
    let map = SessionMap::new();
    // 无条目：no-op 不 panic。
    map.fail_spawn(&key()).await;
    // Active 条目：不动。
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    map.fail_spawn(&key()).await;
    assert_eq!(map.get(&key()).await.unwrap().session_id(), Some("s1"));
    // 不存在 session 的 lookup：None。
    assert!(map.lookup_key_by_session("ghost").await.is_none());
    // remove_by_session 对不存在的 session：no-op。
    map.remove_by_session("ghost").await;
    // activate 无占位 → 插入新映射（warn 路径）。
    let orphan = SessionKey {
        chat_id: "oc_orphan".into(),
        thread_id: None,
    };
    let pending = map.activate(&orphan, "s9".into()).await;
    assert!(pending.is_empty());
    assert_eq!(map.get(&orphan).await.unwrap().session_id(), Some("s9"));
}

#[tokio::test]
async fn help_command_emits_help_text() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "/help".into(),
            reply_to: None,
        })
        .await;
    assert!(matches!(next_out(&mut out_rx).await, Out::HelpText { .. }));
}
