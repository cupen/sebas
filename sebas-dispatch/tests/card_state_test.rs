//! CardStateMap 存储语义单测（FSM/累积在 card_state_test 的后续测试 + Task 5 覆盖）。

use sebas_acp::claude::session::AcpEvent;
use sebas_channels::card::ChannelElement as CardElement;
use sebas_dispatch::cards::CardConfig;
use sebas_dispatch::card_state::{CardState, CardStateMap};
use sebas_dispatch::engine::{Out, DispatchHandle};
use sebas_dispatch::state::SessionMap;
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
    assert_eq!(snap.status_emoji, sebas_dispatch::card_state::phase::SEED);
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
    assert_eq!(snap.status_emoji, sebas_dispatch::card_state::phase::SEED);
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
    assert_eq!(a.status_emoji, sebas_dispatch::card_state::phase::SEED);
    assert!(a.body.is_empty());
    let b = CardState::lazy();
    assert_eq!(b.user_prompt, "");
    assert_eq!(b.status_emoji, sebas_dispatch::card_state::phase::SEED);
}

#[tokio::test]
async fn apply_event_accumulates_without_emitting_out() {
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new(map);
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
            &AcpEvent::ToolStart {
                session_id: "s1".into(),
                tool_name: "Bash".into(),
                args: serde_json::json!({}),
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
    // flush_card 产 1 张 UpdateCard，正文含全部事件渲染；
    // 状态 emoji 不再在标题中，而是由 emit_reaction 单独发 React。
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
            // 标题（turn prompt）现在是 user_prompt（adapter 侧派生 topic）。
            assert_eq!(
                card.turn.as_ref().map(|t| t.prompt.as_str()),
                Some("hi"),
                "turn prompt 为 user_prompt 'hi'"
            );
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
}

#[tokio::test]
async fn fsm_eyes_to_construction_to_done() {
    let map = SessionMap::new();
    // 持有接收端到作用域结束：emit 在通道关闭时会 debug_assert
    // （openspec/specs/acp-driver/spec.md "Channel send fail"），
    // 裸 `_` 丢弃接收端会立刻触发 panic。
    let (router, _rx) = DispatchHandle::new(map);
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
    let (router2, mut out2) = DispatchHandle::new(SessionMap::new());
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
            // 状态 emoji 不再进卡：turn prompt 是 user_prompt（adapter side 派生 topic）。
            assert_eq!(
                card.turn.as_ref().map(|t| t.prompt.as_str()),
                Some("p"),
                "turn prompt 为 user_prompt 'p'"
            );
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
    // Finished -> 终态：apply_event_to_out 出 UpdateCard（card body 推 ✅
    // 已完成父面板），随后 Out::React 换 DONE（997bfe2 恢复终态 reaction）。
    let (router3, mut out3) = DispatchHandle::new(SessionMap::new());
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
    assert!(matches!(o3, Out::UpdateCard { .. }), "先出卡: {o3:?}");
    let o3b = tokio::time::timeout(Duration::from_millis(200), out3.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(o3b, Out::React { ref emoji, .. } if emoji == sebas_dispatch::card_state::phase::DONE),
        "Finished 应换 React DONE: {o3b:?}"
    );
}

#[tokio::test]
async fn fsm_terminal_error_marks_red() {
    // 终态视觉由 card body 表达（❌ 错误行 push 到 body），reaction 不再换
    // FAILED：FSM 仍转 FAILED（apply_event 报告），但 Out::React 不出。
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new(map);
    router.seed_card("s4".into(), "p".into()).await;
    // 直接验证 apply_event 报告 FAILED 转移。
    let new_emoji = router
        .apply_event(
            "s4",
            &AcpEvent::Error {
                session_id: "s4".into(),
                message: "dead".into(),
                terminal: true,
            },
        )
        .await;
    assert_eq!(
        new_emoji,
        Some(sebas_dispatch::card_state::phase::FAILED),
        "apply_event 报告 terminal Error -> FAILED"
    );
    // Out 流水线不发射 FAILED reaction（reaction 维持"已收到"）。
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
    assert!(matches!(o, Out::UpdateCard { .. }), "先出卡: {o:?}");
    // 不应再有 FAILED reaction。
    assert!(
        tokio::time::timeout(Duration::from_millis(120), out_rx.recv())
            .await
            .is_err(),
        "terminal 不再发 Out::React FAILED"
    );
}

#[tokio::test]
async fn new_with_card_config_uses_theme() {
    // 自定义 theme_color 流到渲染卡。
    let cfg = CardConfig {
        theme_color: "orange".into(),
        ..CardConfig::default()
    };
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new_with_card_config(map, cfg);
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
            assert_eq!(card.theme, "orange", "theme 流入中立卡");
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
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
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
        matches!(o2, Out::React { ref emoji, .. } if emoji == sebas_dispatch::card_state::phase::WORKING),
        "再换 reaction WORKING: {o2:?}"
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

    // Finished → 终态：内部 FSM 转 DONE，body 推"✅ 已完成"父面板；随后
    // Out::React 换 DONE（997bfe2 恢复终态 reaction）。
    router
        .apply_event_to_out(
            "r1".into(),
            &AcpEvent::Finished {
                session_id: "r1".into(),
            },
        )
        .await;
    let o4 = recv(&mut out_rx).await;
    assert!(matches!(o4, Out::UpdateCard { .. }), "先出卡: {o4:?}");
    let o5 = recv(&mut out_rx).await;
    assert!(
        matches!(o5, Out::React { ref emoji, .. } if emoji == sebas_dispatch::card_state::phase::DONE),
        "再换 reaction DONE: {o5:?}"
    );
    assert_no_more(&mut out_rx).await;
}

#[tokio::test]
async fn terminal_error_does_not_emit_reaction() {
    // 终态视觉由 card body 表达（❌ 错误行），reaction 维持"已收到"。
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
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
    assert_no_more(&mut out_rx).await;
}

#[tokio::test]
async fn continue_after_done_flips_reaction_back_to_working() {
    use sebas_channels::{ChannelEvent, ChannelKey};
    use sebas_dispatch::state::Mapping;

    let map = SessionMap::new();
    let k = ChannelKey::feishu("oc_x", None);
    map.insert(k.clone(), Mapping::active("r3"))
        .await
        .expect("insert within capacity");
    let (router, mut out_rx) = DispatchHandle::new(map);
    router.seed_card("r3".into(), "第一题".into()).await;
    // 驱动到 DONE（纯状态，无 Out）
    let react = router
        .apply_event(
            "r3",
            &AcpEvent::Finished {
                session_id: "r3".into(),
            },
        )
        .await;
    assert_eq!(
        react,
        Some(sebas_dispatch::card_state::phase::DONE),
        "apply_event 报告 SEED→DONE 转移"
    );

    // 用户追问：continue 回切 WORKING —— 先刷卡，再换 reaction，最后 SendAcp
    router
        .dispatch(ChannelEvent::Text {
            key: k,
            text: "第二题".into(),
            reply_target: None,
        })
        .await;

    let o1 = recv(&mut out_rx).await;
    match o1 {
        Out::UpdateCard { card, .. } => {
            // flush_card 走在 emit_turn_card 之前，使用上一轮的 user_prompt。
            assert_eq!(
                card.turn.as_ref().map(|t| t.prompt.as_str()),
                Some("第一题"),
                "本轮 UpdateCard 是上一轮的终态（user_prompt=第一题）"
            );
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
    let o2 = recv(&mut out_rx).await;
    assert!(
        matches!(o2, Out::React { ref emoji, .. } if emoji == sebas_dispatch::card_state::phase::WORKING),
        "回切 reaction WORKING: {o2:?}"
    );
    let o3 = recv(&mut out_rx).await;
    match o3 {
        Out::SendCard { card, root_id, .. } => {
            // emit_turn_card 重新 seed：新轮的 user_prompt 进入 turn chrome。
            assert!(
                root_id.is_none(),
                "per-turn card reply target 由 Out 自己负责: {root_id:?}"
            );
            assert_eq!(
                card.turn.as_ref().map(|t| t.prompt.as_str()),
                Some("第二题"),
                "per-turn card 的 turn prompt 是本轮 user_prompt '第二题'"
            );
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
    let o4 = recv(&mut out_rx).await;
    assert!(matches!(o4, Out::SendAcp { .. }), "继续会话: {o4:?}");
    assert_no_more(&mut out_rx).await;
}

// ---- sebas card-flip: permission card click feedback ----

use sebas_dispatch::cards_ui::resolved_permission_card as render_resolved_permission_card;
use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};

#[tokio::test]
async fn permission_card_click_emits_resolved_card_flip() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_perm", None);
    // Seed an active session mapping so `on_button` passes the
    // `session_alive` check (production: the session is alive while
    // there's a Claude child process for this chat).
    let _ = router
        .map
        .insert(key.clone(), sebas_dispatch::state::Mapping::active("sess-flip"))
        .await;
    // Simulate the dispatch_out step that records the Feishu message_id
    // keyed by request_id (production: after `send_card` returns).
    router
        .record_perm_card_msg_id(
            "req-1".into(),
            key.clone(),
            "om_real".into(),
            "Bash".into(),
            serde_json::json!({"command": "echo hi"}),
        )
        .await;
    // User clicks Allow once on the card.
    router
        .dispatch(ChannelEvent::ButtonCb {
            key: key.clone(),
            action: ChannelAction {
                decision: Some("allow_once".into()),
                session_id: "sess-1".into(),
                request_id: Some("req-1".into()),
                value: serde_json::json!({ "chat_type": "p2p" }),
            },
        })
        .await;
    // First Out: SendAcp carrying PermissionReply — 放行是首要语义，先行
    // 让泊车的 hook 解锁（permission-mode-auto-gate D2 的 ①②③ 顺序）。
    let o1 = recv(&mut out_rx).await;
    match &o1 {
        Out::SendAcp {
            cmd:
                sebas_acp::claude::session::AcpCommand::PermissionReply {
                    request_id,
                    decision,
                    ..
                },
            ..
        } => {
            assert_eq!(request_id, "req-1");
            assert!(matches!(decision, sebas_acp::claude::session::Decision::AllowOnce));
        }
        other => panic!("expected SendAcp PermissionReply, got {other:?}"),
    }
    // Second Out: UpdateCardByMsgId that flips the original card in place.
    let o2 = recv(&mut out_rx).await;
    let msg_id = match &o2 {
        Out::UpdateCardByMsgId { key: k, msg_id, .. } => {
            assert_eq!(k.reference, "oc_perm");
            assert_eq!(msg_id, "om_real");
            msg_id.clone()
        }
        other => panic!("expected UpdateCardByMsgId, got {other:?}"),
    };
    // The card body should carry the resolved label.
    if let Out::UpdateCardByMsgId { card, .. } = &o2 {
        let s = serde_json::to_string(card).unwrap();
        assert!(s.contains("已允许"), "resolved card body: {s}");
    }
    // take_perm_card removed the entry on click — a second click now
    // hits the stale path and emits a fresh "已过期" card instead of
    // trying to update a gone message.
    assert!(router.take_perm_card("req-1").await.is_none());
    let _ = msg_id; // silence unused if pattern changes
}

#[tokio::test]
async fn stale_permission_click_emits_expired_card() {
    // No record_perm_card_msg_id call — simulates the case where the
    // request was already resolved (responder consumed) or never tracked.
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_perm", None);
    // Seed an active session so `on_button` reaches the click path
    // (the stale branch is taken because perm_cards.take returns None,
    // not because the session is dead).
    let _ = router
        .map
        .insert(key.clone(), sebas_dispatch::state::Mapping::active("sess-stale"))
        .await;
    router
        .dispatch(ChannelEvent::ButtonCb {
            key: key.clone(),
            action: ChannelAction {
                decision: Some("allow_once".into()),
                session_id: "sess-1".into(),
                request_id: Some("req-stale".into()),
                value: serde_json::json!({ "chat_type": "p2p" }),
            },
        })
        .await;
    // Stale click should NOT emit UpdateCardByMsgId (nothing to update
    // by message_id) and should NOT emit SendAcp (no responder to call).
    // Instead it emits a fresh SendCard carrying the "已过期" body.
    let got = loop {
        let o = recv(&mut out_rx).await;
        // Drain any react/update noise from unrelated FSM work.
        if matches!(o, Out::SendCard { .. }) {
            break o;
        }
    };
    let Out::SendCard { key: k, card, .. } = got else {
        panic!("expected SendCard for expired");
    };
    assert_eq!(k.reference, "oc_perm");
    let s = serde_json::to_string(&card).unwrap();
    assert!(s.contains("已过期"), "expired card body: {s}");
    assert_no_more(&mut out_rx).await;
}

#[test]
fn render_resolved_card_includes_label() {
    // Sanity: the resolved card body echoes whatever the router hands in.
    let card = render_resolved_permission_card("✅ 已允许（仅此一次）");
    let v = serde_json::to_value(&card).unwrap();
    let s = v.to_string();
    assert!(s.contains("已允许（仅此一次）"), "resolved label: {s}");
}

// ---- permission-mode-auto-gate：「本会话不再询问」= 放行 + SetMode(auto) ----
// （聊天级 allowlist 已退役：签名匹配/grant_all 存储与「命中即放行」路径
// 全部删除，自动放行一律由 driver 层 mode 门控执法。）

use sebas_acp::claude::session::{AcpCommand, Decision};
use serde_json::json;

#[tokio::test]
async fn allow_session_click_replies_then_switches_mode_to_auto() {
    use sebas_acp::claude::session::AcpEvent;
    use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};
    use sebas_dispatch::engine::{Out, DispatchHandle};
    use sebas_dispatch::state::{Mapping, SessionMap};
    use std::time::Duration;

    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

    // First call: a card goes out carrying the request for the click handler.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::PermissionRequest {
                session_id: "s1".into(),
                request_id: "r1".into(),
                tool_name: "Bash".into(),
                args: json!({"command": "ls /tmp"}),
            },
        )
        .await;
    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::SendCard {
            perm_request_id, ..
        } => assert_eq!(perm_request_id.as_deref(), Some("r1")),
        other => panic!("expected SendCard, got {other:?}"),
    }
    // Dispatcher records the msg_id (production: after send_card returns).
    router
        .record_perm_card_msg_id(
            "r1".into(),
            key.clone(),
            "om_1".into(),
            "Bash".into(),
            json!({"command": "ls /tmp"}),
        )
        .await;

    // User clicks 本会话不再询问. Expected Out order pins the semantics:
    // ① PermissionReply(AllowSession) 放行（首要语义，先行让 hook 解锁）
    // ② SendAcp SetMode{auto}（与 webui 中程切换同源的 Out::SendAcp 路径）
    // ③ UpdateCardByMsgId 翻面「✅ 已切换自动模式」
    router
        .dispatch(ChannelEvent::ButtonCb {
            key: key.clone(),
            action: ChannelAction {
                session_id: "s1".into(),
                request_id: Some("r1".into()),
                decision: Some("allow_session".into()),
                value: json!({ "chat_type": "p2p" }),
            },
        })
        .await;

    let mut saw_reply = false;
    let mut saw_set_mode = false;
    let mut saw_flip = false;
    for _ in 0..3 {
        match tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Out::SendAcp {
                session_id,
                cmd: AcpCommand::PermissionReply { decision, .. },
            } => {
                assert_eq!(session_id, "s1");
                assert!(matches!(decision, Decision::AllowSession));
                assert!(!saw_set_mode, "放行必须先于 SetMode（hook 先解锁）");
                saw_reply = true;
            }
            Out::SendAcp {
                session_id,
                cmd: AcpCommand::SetMode { session_id: sid, mode },
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(sid, "s1");
                assert_eq!(mode, "auto");
                assert!(saw_reply, "SetMode 在放行之后");
                saw_set_mode = true;
            }
            Out::UpdateCardByMsgId { card, .. } => {
                let s = serde_json::to_string(&card).unwrap();
                assert!(s.contains("已切换自动模式"), "flip label: {s}");
                saw_flip = true;
            }
            other => panic!("unexpected Out after click: {other:?}"),
        }
    }
    assert!(saw_reply && saw_set_mode && saw_flip);

    // ③ mapping desired_mode=auto 落位（ask 会话点击后的「不再弹卡」语义由
    // driver 层 hook 门控保证——dispatch 只负责把 mode 请求送达执行体）。
    let desired = router
        .map
        .get(&key)
        .await
        .and_then(|m| m.desired_mode.clone());
    assert_eq!(desired.as_deref(), Some("auto"));

    // 在飞记录已登记（driver 事件回执失败时据实翻卡用）。
    assert!(router.auto_mode_switches().take("s1").await.is_some());

    // 其后同会话的游离 SetMode 失败 Error 不再误报（记录已被成功消费）。
    // 注：ModeChanged/Error 经 apply_event_to_out 会顺带 flush 会话卡
    // （Out::UpdateCard，既有行为）——只容忍该噪声，断言无失败翻卡。
    router.apply_event_to_out(
        "s1".into(),
        &AcpEvent::ModeChanged {
            session_id: "s1".into(),
            mode: "auto".into(),
        },
    ).await;
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Error {
                session_id: "s1".into(),
                message: "set mode \"auto\" 被拒绝或未送达（boom），模式未变".into(),
                terminal: false,
            },
        )
        .await;
    loop {
        match tokio::time::timeout(Duration::from_millis(100), out_rx.recv()).await {
            Err(_) => break, // 无更多 Out
            Ok(Some(Out::UpdateCard { .. })) => continue, // 会话卡 flush 噪声
            Ok(Some(other)) => panic!("成功消费后不应有失败翻卡，got {other:?}"),
            Ok(None) => break,
        }
    }
    let turns = router.session_turns(&key, 0).await.unwrap();
    assert!(
        !turns
            .iter()
            .any(|e| e.element_type == "permission_mode_result"),
        "成功路径不写失败契约条目"
    );
}

#[tokio::test]
async fn allow_session_click_failure_keeps_allow_and_reports_honestly() {
    use sebas_acp::claude::session::AcpEvent;
    use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};
    use sebas_dispatch::engine::{Out, DispatchHandle};
    use sebas_dispatch::state::{Mapping, SessionMap};
    use std::time::Duration;

    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    router
        .record_perm_card_msg_id(
            "r1".into(),
            key.clone(),
            "om_1".into(),
            "Bash".into(),
            json!({"command": "ls /tmp"}),
        )
        .await;

    // 点击（SetMode 已发出，随后执行体拒绝）。
    router
        .dispatch(ChannelEvent::ButtonCb {
            key: key.clone(),
            action: ChannelAction {
                session_id: "s1".into(),
                request_id: Some("r1".into()),
                decision: Some("allow_session".into()),
                value: json!({}),
            },
        })
        .await;
    // 排掉点击三连（reply + SetMode + 翻面）。
    for _ in 0..3 {
        tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
            .await
            .unwrap()
            .unwrap();
    }

    // 执行体拒绝：非终态 Error 带驱动「模式未变」标记。
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Error {
                session_id: "s1".into(),
                message: "set mode \"auto\" 被拒绝或未送达（rejected），模式未变".into(),
                terminal: false,
            },
        )
        .await;

    // 失败如实上报（事件契约）：turn 流 permission_mode_result 条目。
    let turns = router.session_turns(&key, 0).await.unwrap();
    let entry = turns
        .iter()
        .find(|e| e.element_type == "permission_mode_result")
        .expect("失败必须写事件契约条目");
    let payload: serde_json::Value = serde_json::from_str(&entry.content).unwrap();
    assert_eq!(payload["request_id"], "r1");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["mode"], "auto");
    assert!(payload["detail"].as_str().unwrap().contains("rejected"));

    // 失败如实翻卡（卡归本进程跟踪时）：orange 主题 + 放行仍在的诚实文案。
    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::UpdateCardByMsgId { key: k, card, .. } => {
            assert_eq!(k, key);
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("当前调用已放行"), "放行不回滚须如实呈现: {s}");
            assert!(s.contains("自动模式切换失败"), "失败文案: {s}");
            assert!(s.contains("orange"), "失败态主题: {s}");
        }
        other => panic!("expected failure flip, got {other:?}"),
    }
    // 取走即消费：重复 Error 不重复上报（容忍既有会话卡 flush 噪声）。
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Error {
                session_id: "s1".into(),
                message: "set mode \"auto\" 被拒绝或未送达（rejected），模式未变".into(),
                terminal: false,
            },
        )
        .await;
    loop {
        match tokio::time::timeout(Duration::from_millis(100), out_rx.recv()).await {
            Err(_) => break,
            Ok(Some(Out::UpdateCard { .. })) => continue,
            Ok(Some(other)) => panic!("重复失败事件不应二次翻卡，got {other:?}"),
            Ok(None) => break,
        }
    }
    let turns_after = router.session_turns(&key, 0).await.unwrap();
    assert_eq!(
        turns_after
            .iter()
            .filter(|e| e.element_type == "permission_mode_result")
            .count(),
        1,
        "重复失败事件不写第二条契约条目"
    );
}

#[tokio::test]
async fn later_requests_still_render_cards_after_allow_session() {
    // allowlist 路径不复存在：点击「本会话不再询问」后，dispatch 侧不再
    // 自动放行任何请求——后续 PermissionRequest 照常出卡（自动放行由
    // driver 层 mode 门控在 hook 里执法，根本不产生请求）。
    use sebas_acp::claude::session::AcpEvent;
    use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};
    use sebas_dispatch::engine::{Out, DispatchHandle};
    use sebas_dispatch::state::{Mapping, SessionMap};
    use std::time::Duration;

    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    router
        .record_perm_card_msg_id(
            "r1".into(),
            key.clone(),
            "om_1".into(),
            "Bash".into(),
            json!({"command": "ls /tmp"}),
        )
        .await;

    router
        .dispatch(ChannelEvent::ButtonCb {
            key: key.clone(),
            action: ChannelAction {
                session_id: "s1".into(),
                request_id: Some("r1".into()),
                decision: Some("allow_session".into()),
                value: json!({}),
            },
        })
        .await;
    // Drain 点击三连。
    for _ in 0..3 {
        tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
            .await
            .unwrap()
            .unwrap();
    }

    // 第二个请求（不同工具不同参数）：必须出卡，绝不静默放行。
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::PermissionRequest {
                session_id: "s1".into(),
                request_id: "r2".into(),
                tool_name: "Write".into(),
                args: json!({"path": "/etc/hostname", "content": "x"}),
            },
        )
        .await;
    match tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Out::SendCard {
            perm_request_id, ..
        } => assert_eq!(perm_request_id.as_deref(), Some("r2")),
        other => panic!("expected a card for r2 (no dispatch-side auto-approve), got {other:?}"),
    }
    // 不产生任何自动放行 reply。
    assert!(
        tokio::time::timeout(Duration::from_millis(100), out_rx.recv())
            .await
            .is_err(),
        "dispatch 侧不再自动放行"
    );
}


// ---- sebas-per-turn: Out::SendCard carries root_id (Task 2) ----

#[test]
fn out_send_card_carries_root_id() {
    let out = Out::SendCard {
        key: sebas_channels::ChannelKey::feishu("oc_test", None),
        card: sebas_channels::ChannelCard::new("t", "blue"),
        msg_id: None,
        perm_request_id: None,
        perm_meta: None,
        root_id: Some("om_user_msg".into()),
    };
    let s = format!("{:?}", out);
    assert!(
        s.contains("root_id"),
        "Debug output should contain root_id: {s}"
    );
    assert!(
        s.contains("om_user_msg"),
        "Debug output should contain the root_id value: {s}"
    );
}

#[test]
fn out_send_card_root_id_none_round_trips() {
    let out = Out::SendCard {
        key: sebas_channels::ChannelKey::feishu("oc_test", None),
        card: sebas_channels::ChannelCard::new("t", "blue"),
        msg_id: None,
        perm_request_id: None,
        perm_meta: None,
        root_id: None,
    };
    let s = format!("{:?}", out);
    assert!(
        s.contains("root_id"),
        "Debug output should contain root_id: {s}"
    );
}

#[tokio::test]
async fn permission_request_without_grant_still_renders_card() {
    // Sanity counterpart to the auto-approve test: a fresh (Bash, ls)
    // call when nothing is on the allowlist must still show the card so
    // the user can decide.
    use sebas_channels::ChannelKey;
    use sebas_dispatch::engine::DispatchHandle;
    use sebas_dispatch::state::SessionMap;

    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_x", None);
    let session_id = "sess-1".to_string();
    let _ = router
        .map
        .insert(
            key.clone(),
            sebas_dispatch::state::Mapping::active(session_id.clone()),
        )
        .await;

    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: session_id.clone(),
            request_id: "req-fresh".into(),
            tool_name: "Bash".into(),
            args: json!({"command": "ls /tmp"}),
        })
        .await;

    let out = tokio::time::timeout(std::time::Duration::from_millis(200), out_rx.recv())
        .await
        .expect("Out within 200ms")
        .expect("channel closed");
    match out {
        Out::SendCard {
            key: k,
            perm_request_id,
            ..
        } => {
            assert_eq!(k.reference, "oc_x");
            assert_eq!(perm_request_id.as_deref(), Some("req-fresh"));
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
}
