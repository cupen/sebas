//! 路由分支补测（覆盖率目标 router/ ≥ 90% 的补齐测试）：on_button 死会话/
//! 缺 rid/未知 decision，slash 转发臂，Media 组合 prompt，dispatch_acp_event
//! 全事件类型，MsgIdMap 存取，terminal error 清理。

use acp_claude::session::{AcpCommand, AcpEvent, Decision};
use feishu::events::{CardAction, FeishuIn, SessionKey};
use router::router::{Out, RouterHandle, compose_media_prompt};
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

/// 收干一小段时间窗内的全部 Out（p3g 起 FSM 转移会连带 Out::React，
/// 不能再假设一个事件只产一个 Out）。
async fn drain(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Vec<Out> {
    let mut out = vec![];
    while let Ok(Some(o)) = tokio::time::timeout(Duration::from_millis(60), rx.recv()).await {
        out.push(o);
    }
    out
}

// ---------- ReplyTargetMap 生命周期（F2 修复） ----------

/// spawn_new 必须清掉该 key 的 reply target：新会话不继承上一条入站的
/// 回复目标（话题内 root_id），否则 ReplyTargetMap 随话题数无界增长。
#[tokio::test]
async fn spawn_new_clears_reply_target() {
    let map = SessionMap::new();
    let key = SessionKey {
        chat_id: "oc_topic".into(),
        thread_id: Some("omt_t1".into()),
    };
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

    // 入站话题消息写入 reply target；已映射会话走 Continue，不触发 spawn_new。
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "hello".into(),
            reply_to: Some("om_root".into()),
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    assert_eq!(router.reply_target(&key).await.as_deref(), Some("om_root"));

    // 摘掉映射后下一条消息走 SpawnNew → spawn_new 必须清 reply target。
    map.remove_by_key(&key).await;
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "fresh".into(),
            reply_to: Some("om_root".into()),
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    assert_eq!(
        router.reply_target(&key).await,
        None,
        "spawn_new must clear the stale reply target"
    );

    let _ = drain(&mut out_rx).await;
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
            chat_type: "p2p".into(),
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
            chat_type: "p2p".into(),
        })
        .await;
    assert!(matches!(next_out(&mut out_rx).await, Out::HelpText { .. }));
}

#[tokio::test]
async fn button_cb_unknown_decision_fails_closed_to_deny() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    // In-place flip (commit 658b312) requires a pre-recorded perm_card entry;
    // see tests/permission_flow_test.rs:128 for the recipe.
    router
        .record_perm_card_msg_id(
            "r9".into(),
            key(),
            "om_fake".into(),
            "Bash".into(),
            serde_json::json!({"cmd": "yolo"}),
        )
        .await;
    router
        .dispatch(FeishuIn::ButtonCb {
            key: key(),
            action: CardAction {
                session_id: "s1".into(),
                request_id: Some("r9".into()),
                decision: Some("yolo".into()),
                value: serde_json::Value::Null,
            },
            chat_type: "p2p".into(),
        })
        .await;
    // First Out is the in-place flip (UpdateCardByMsgId); drain until SendAcp.
    let out = loop {
        let got = next_out(&mut out_rx).await;
        if matches!(got, Out::SendAcp { .. }) {
            break got;
        }
    };
    match out {
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
                chat_type: "private".into(),
                mentions: vec![],
            })
            .await;

        if text == "/compact" {
            // /compact now sends a progress card first, then the command
            match next_out(&mut out_rx).await {
                Out::SendCard { .. } => {} // progress card
                other => panic!("expected SendCard for /compact, got {other:?}"),
            }
        }

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
            chat_type: "private".into(),
            mentions: vec![],
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
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    assert!(matches!(next_out(&mut out_rx).await, Out::HelpText { .. }));
}

#[tokio::test]
async fn slash_status_forwards_continue_session() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "/status".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;

    match next_out(&mut out_rx).await {
        Out::SendAcp {
            cmd: AcpCommand::ContinueSession { session_id, prompt },
            ..
        } => {
            assert_eq!(session_id, "s1");
            assert_eq!(prompt, "/status");
        }
        other => panic!("expected ContinueSession(\"/status\"), got {other:?}"),
    }
}

#[tokio::test]
async fn slash_sessions_lists_empty() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "/sessions".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;

    match next_out(&mut out_rx).await {
        Out::PlainText { content, .. } => {
            assert!(content.contains("没有活跃会话"), "content: {content}");
        }
        other => panic!("expected PlainText, got {other:?}"),
    }
}

#[tokio::test]
async fn slash_sessions_lists_active() {
    let map = SessionMap::new();
    map.insert(key(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "/sessions".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;

    match next_out(&mut out_rx).await {
        Out::PlainText { content, .. } => {
            assert!(content.contains("s1"), "s1 missing: {content}");
            assert!(content.contains("active"), "label missing: {content}");
        }
        other => panic!("expected PlainText, got {other:?}"),
    }
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
            reply_to: None,
            chat_type: "private".into(),
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
        let outs = drain(&mut out_rx).await;
        assert!(
            outs.iter().any(|o| matches!(o, Out::UpdateCard { .. })),
            "expected UpdateCard in {outs:?}"
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
    // terminal Error 不再发 Out::React 换 FAILED（❌ 行已 push 到 body）；
    // queue timeout 之后应无更多 Out。
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
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    // Per-turn flow: a fresh card is posted first, then the prompt is
    // forwarded to the session.
    match next_out(&mut out_rx).await {
        Out::SendCard { .. } => {}
        other => panic!("expected per-turn SendCard, got {other:?}"),
    }
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
async fn help_command_emits_help_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "/help".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    let out = next_out(&mut out_rx).await;
    match out {
        Out::SendCard { msg_id, card, .. } => {
            // 应携带帮助卡片哨兵 tag
            assert_eq!(
                msg_id.as_deref(),
                Some("__help_card__"),
                "help card should carry HELP_CARD_TAG"
            );
            // 卡片应包含分组 tab 按钮和命令
            let s = card.to_string();
            assert!(s.contains("💬 会话管理"), "help card missing session tab");
            assert!(s.contains("/new"), "help card missing /new command");
            assert!(s.contains("/cancel"), "help card missing /cancel command");
        }
        other => panic!("expected SendCard (help card), got {other:?}"),
    }
}
