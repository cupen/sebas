//! CardStateMap 存储语义单测（FSM/累积在 card_state_test 的后续测试 + Task 5 覆盖）。

use acp_claude::session::AcpEvent;
use feishu::cards::CardConfig;
use feishu::cards::CardElement;
use router::card_state::{CardState, CardStateMap};
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::time::Duration;

#[tokio::test]
async fn seed_is_idempotent_keeps_accumulated_prompt() {
    let m = CardStateMap::default();
    m.seed("s1".into(), "original".into()).await;
    m.apply("s1", |st| {
        st.body.push(CardElement::Markdown {
            content: "accumulated".into(),
        })
    })
    .await;
    // 重入 seed：保留原 prompt 与 body，不冲掉。
    m.seed("s1".into(), "SHOULD_NOT_WIN".into()).await;
    let snap = m.snapshot("s1").await.expect("seeded");
    assert_eq!(snap.user_prompt, "original");
    assert_eq!(snap.status_emoji, "👀");
    assert_eq!(snap.body.len(), 1);
}

#[tokio::test]
async fn apply_lazy_seeds_with_empty_prompt() {
    let m = CardStateMap::default();
    // 未 seed 直接 apply：lazy 兜底，prompt=""。
    m.apply("s2", |st| {
        st.body.push(CardElement::Markdown {
            content: "early".into(),
        })
    })
    .await;
    let snap = m.snapshot("s2").await.expect("lazy seeded");
    assert_eq!(snap.user_prompt, "");
    assert_eq!(snap.status_emoji, "👀");
    assert_eq!(snap.body.len(), 1);
}

#[tokio::test]
async fn drop_removes_entry() {
    let m = CardStateMap::default();
    m.seed("s3".into(), "hi".into()).await;
    assert!(m.snapshot("s3").await.is_some());
    m.drop("s3").await;
    assert!(m.snapshot("s3").await.is_none());
    // 幂等：drop 不存在的 entry 不 panic。
    m.drop("s3").await;
}

#[tokio::test]
async fn new_and_lazy_constructors() {
    let a = CardState::new("prompt");
    assert_eq!(a.user_prompt, "prompt");
    assert_eq!(a.status_emoji, "👀");
    assert!(a.body.is_empty());
    let b = CardState::lazy();
    assert_eq!(b.user_prompt, "");
    assert_eq!(b.status_emoji, "👀");
}

#[tokio::test]
async fn apply_event_accumulates_without_emitting_out() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router.seed_card("s1".into(), "hi".into()).await;
    // 连发多个流式事件：apply_event 期间无 Out。
    let _ = router
        .apply_event(
            "s1",
            &AcpEvent::TextDelta {
                session_id: "s1".into(),
                delta: "a".into(),
            },
        )
        .await;
    let _ = router
        .apply_event(
            "s1",
            &AcpEvent::ThinkingDelta {
                session_id: "s1".into(),
                delta: "think".into(),
            },
        )
        .await;
    let _ = router
        .apply_event(
            "s1",
            &AcpEvent::ToolEnd {
                session_id: "s1".into(),
                tool_name: "Bash".into(),
                result: "ok".into(),
            },
        )
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), out_rx.recv())
            .await
            .is_err(),
        "apply_event 不得发 Out"
    );
    // flush_card 产 1 张 UpdateCard，正文含全部事件渲染，emoji 🚧。
    router.flush_card("s1").await;
    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::UpdateCard { session_id, card } => {
            assert_eq!(session_id, "s1");
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("a"), "含 TextDelta: {s}");
            assert!(s.contains("think"), "含 ThinkingDelta: {s}");
            assert!(s.contains("Bash"), "含 ToolEnd: {s}");
            assert!(s.contains("🚧"), "emoji 🚧: {s}");
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
}

#[tokio::test]
async fn fsm_eyes_to_construction_to_done() {
    let map = SessionMap::new();
    // 持有接收端到作用域结束：emit 在通道关闭时会 debug_assert（spec §4.1
    // "Channel send fail"），裸 `_` 丢弃接收端会立刻触发 panic。
    let (router, _rx) = RouterHandle::new(map);
    router.seed_card("s2".into(), "p".into()).await;
    // seed = 👀
    router.flush_card("s2").await; // 不验 Out，只驱动状态机内部（flush 不改 emoji）
    let _ = router
        .apply_event(
            "s2",
            &AcpEvent::TextDelta {
                session_id: "s2".into(),
                delta: "x".into(),
            },
        )
        .await;
    router.flush_card("s2").await;
    // 验证 🚧：用 apply_event_to_out 同步路径产卡断言 emoji
    let (router2, mut out2) = RouterHandle::new(SessionMap::new());
    router2.seed_card("s2".into(), "p".into()).await;
    let _ = router2
        .apply_event(
            "s2",
            &AcpEvent::TextDelta {
                session_id: "s2".into(),
                delta: "x".into(),
            },
        )
        .await;
    router2.flush_card("s2").await;
    let o = tokio::time::timeout(Duration::from_millis(200), out2.recv())
        .await
        .unwrap()
        .unwrap();
    match o {
        Out::UpdateCard { card, .. } => {
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("🚧"));
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
    // Finished -> ✅
    let (router3, mut out3) = RouterHandle::new(SessionMap::new());
    router3.seed_card("s3".into(), "p".into()).await;
    router3
        .apply_event_to_out(
            "s3".into(),
            &AcpEvent::Finished {
                session_id: "s3".into(),
            },
        )
        .await;
    let o3 = tokio::time::timeout(Duration::from_millis(200), out3.recv())
        .await
        .unwrap()
        .unwrap();
    match o3 {
        Out::UpdateCard { card, .. } => {
            let s3 = serde_json::to_string(&card).unwrap();
            assert!(s3.contains("✅"));
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
}

#[tokio::test]
async fn fsm_terminal_error_marks_red() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router.seed_card("s4".into(), "p".into()).await;
    router
        .apply_event_to_out(
            "s4".into(),
            &AcpEvent::Error {
                session_id: "s4".into(),
                message: "dead".into(),
                terminal: true,
            },
        )
        .await;
    let o = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match o {
        Out::UpdateCard { card, .. } => {
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("❌"));
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
}

#[tokio::test]
async fn new_with_card_config_uses_theme() {
    // 自定义 theme_color 流到渲染卡。
    let cfg = CardConfig {
        theme_color: "orange".into(),
        ..CardConfig::default()
    };
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new_with_card_config(map, cfg);
    router.seed_card("s5".into(), "hi".into()).await;
    router
        .apply_event_to_out(
            "s5".into(),
            &AcpEvent::TextDelta {
                session_id: "s5".into(),
                delta: "x".into(),
            },
        )
        .await;
    let o = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match o {
        Out::UpdateCard { card, .. } => {
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("\"template\":\"orange\""));
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
}

// ---- sebas-p3g: root 卡 reaction 状态机（Out::React 发射） ----

async fn recv(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .expect("timed out waiting for Out")
        .expect("channel closed")
}

async fn assert_no_more(rx: &mut tokio::sync::mpsc::Receiver<Out>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(60), rx.recv())
            .await
            .is_err(),
        "不应再有多余 Out"
    );
}

#[tokio::test]
async fn phase_transitions_emit_reactions_card_first() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    router.seed_card("r1".into(), "p".into()).await;

    // 首个流式事件：👀→🚧，先出 UpdateCard，紧跟 React 🚧
    router
        .apply_event_to_out(
            "r1".into(),
            &AcpEvent::ToolStart {
                session_id: "r1".into(),
                tool_name: "Read".into(),
                args: serde_json::json!({"path": "a"}),
            },
        )
        .await;
    let o1 = recv(&mut out_rx).await;
    assert!(matches!(o1, Out::UpdateCard { .. }), "先出卡: {o1:?}");
    let o2 = recv(&mut out_rx).await;
    assert!(
        matches!(o2, Out::React { ref emoji, .. } if emoji == "🚧"),
        "再换 reaction 🚧: {o2:?}"
    );

    // 已 🚧 时的流式事件只出卡，不再发 React
    router
        .apply_event_to_out(
            "r1".into(),
            &AcpEvent::ToolProgress {
                session_id: "r1".into(),
                tool_name: "Read".into(),
                progress: "50%".into(),
            },
        )
        .await;
    let o3 = recv(&mut out_rx).await;
    assert!(matches!(o3, Out::UpdateCard { .. }), "出卡: {o3:?}");
    assert_no_more(&mut out_rx).await;

    // Finished → ✅
    router
        .apply_event_to_out("r1".into(), &AcpEvent::Finished { session_id: "r1".into() })
        .await;
    let o4 = recv(&mut out_rx).await;
    assert!(matches!(o4, Out::UpdateCard { .. }), "先出卡: {o4:?}");
    let o5 = recv(&mut out_rx).await;
    assert!(
        matches!(o5, Out::React { ref emoji, .. } if emoji == "✅"),
        "Finished 换 ✅: {o5:?}"
    );
    assert_no_more(&mut out_rx).await;
}

#[tokio::test]
async fn terminal_error_emits_cross_reaction() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    router.seed_card("r2".into(), "p".into()).await;
    router
        .apply_event_to_out(
            "r2".into(),
            &AcpEvent::Error {
                session_id: "r2".into(),
                message: "boom".into(),
                terminal: true,
            },
        )
        .await;
    let o1 = recv(&mut out_rx).await;
    assert!(matches!(o1, Out::UpdateCard { .. }), "先出卡: {o1:?}");
    let o2 = recv(&mut out_rx).await;
    assert!(
        matches!(o2, Out::React { ref emoji, .. } if emoji == "❌"),
        "terminal Error 换 ❌: {o2:?}"
    );
    assert_no_more(&mut out_rx).await;
}

#[tokio::test]
async fn continue_after_done_flips_reaction_back_to_working() {
    use feishu::events::{FeishuIn, SessionKey};
    use router::state::Mapping;

    let map = SessionMap::new();
    let k = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    map.insert(k.clone(), Mapping::active("r3"))
        .await
        .expect("insert within capacity");
    let (router, mut out_rx) = RouterHandle::new(map);
    router.seed_card("r3".into(), "第一题".into()).await;
    // 驱动到 ✅（纯状态，无 Out）
    let react = router
        .apply_event("r3", &AcpEvent::Finished { session_id: "r3".into() })
        .await;
    assert_eq!(react, Some("✅"), "apply_event 报告 👀→✅ 转移");

    // 用户追问：continue 回切 🚧 —— 先刷卡，再换 reaction，最后 SendAcp
    router
        .dispatch(FeishuIn::Text {
            key: k,
            text: "第二题".into(),
            reply_to: None,
        })
        .await;

    let o1 = recv(&mut out_rx).await;
    match o1 {
        Out::UpdateCard { card, .. } => {
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("🚧"), "回切后卡片状态 🚧: {s}");
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
    let o2 = recv(&mut out_rx).await;
    assert!(
        matches!(o2, Out::React { ref emoji, .. } if emoji == "🚧"),
        "回切 reaction 🚧: {o2:?}"
    );
    let o3 = recv(&mut out_rx).await;
    assert!(matches!(o3, Out::SendAcp { .. }), "继续会话: {o3:?}");
    assert_no_more(&mut out_rx).await;
}
