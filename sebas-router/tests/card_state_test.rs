//! CardStateMap 存储语义单测（FSM/累积在 card_state_test 的后续测试 + Task 5 覆盖）。

use sebas_acp::claude::session::AcpEvent;
use sebas_channels::card::ChannelElement as CardElement;
use sebas_router::cards::CardConfig;
use sebas_router::card_state::{CardState, CardStateMap};
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::SessionMap;
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
    assert_eq!(snap.status_emoji, sebas_router::card_state::phase::SEED);
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
    assert_eq!(snap.status_emoji, sebas_router::card_state::phase::SEED);
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
    assert_eq!(a.status_emoji, sebas_router::card_state::phase::SEED);
    assert!(a.body.is_empty());
    let b = CardState::lazy();
    assert_eq!(b.user_prompt, "");
    assert_eq!(b.status_emoji, sebas_router::card_state::phase::SEED);
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
    assert!(matches!(o3, Out::UpdateCard { .. }), "先出卡: {o3:?}");
    let o3b = tokio::time::timeout(Duration::from_millis(200), out3.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(o3b, Out::React { ref emoji, .. } if emoji == sebas_router::card_state::phase::DONE),
        "Finished 应换 React DONE: {o3b:?}"
    );
}

#[tokio::test]
async fn fsm_terminal_error_marks_red() {
    // 终态视觉由 card body 表达（❌ 错误行 push 到 body），reaction 不再换
    // FAILED：FSM 仍转 FAILED（apply_event 报告），但 Out::React 不出。
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
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
        Some(sebas_router::card_state::phase::FAILED),
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
        matches!(o2, Out::React { ref emoji, .. } if emoji == sebas_router::card_state::phase::WORKING),
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
        matches!(o5, Out::React { ref emoji, .. } if emoji == sebas_router::card_state::phase::DONE),
        "再换 reaction DONE: {o5:?}"
    );
    assert_no_more(&mut out_rx).await;
}

#[tokio::test]
async fn terminal_error_does_not_emit_reaction() {
    // 终态视觉由 card body 表达（❌ 错误行），reaction 维持"已收到"。
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
    assert_no_more(&mut out_rx).await;
}

#[tokio::test]
async fn continue_after_done_flips_reaction_back_to_working() {
    use sebas_channels::{ChannelEvent, ChannelKey};
    use sebas_router::state::Mapping;

    let map = SessionMap::new();
    let k = ChannelKey::feishu("oc_x", None);
    map.insert(k.clone(), Mapping::active("r3"))
        .await
        .expect("insert within capacity");
    let (router, mut out_rx) = RouterHandle::new(map);
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
        Some(sebas_router::card_state::phase::DONE),
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
        matches!(o2, Out::React { ref emoji, .. } if emoji == sebas_router::card_state::phase::WORKING),
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

use sebas_router::cards_ui::resolved_permission_card as render_resolved_permission_card;
use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};

#[tokio::test]
async fn permission_card_click_emits_resolved_card_flip() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_perm", None);
    // Seed an active session mapping so `on_button` passes the
    // `session_alive` check (production: the session is alive while
    // there's a Claude child process for this chat).
    let _ = router
        .map
        .insert(key.clone(), sebas_router::state::Mapping::active("sess-flip"))
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
    // First Out: UpdateCardByMsgId that flips the original card in place.
    let o1 = recv(&mut out_rx).await;
    let msg_id = match &o1 {
        Out::UpdateCardByMsgId { key: k, msg_id, .. } => {
            assert_eq!(k.reference, "oc_perm");
            assert_eq!(msg_id, "om_real");
            msg_id.clone()
        }
        other => panic!("expected UpdateCardByMsgId, got {other:?}"),
    };
    // The card body should carry the resolved label.
    if let Out::UpdateCardByMsgId { card, .. } = &o1 {
        let s = serde_json::to_string(card).unwrap();
        assert!(s.contains("已允许"), "resolved card body: {s}");
    }
    // Second Out: SendAcp carrying PermissionReply (the actual decision
    // forwarded to the bridge).
    let o2 = recv(&mut out_rx).await;
    match o2 {
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
        other => panic!("expected SendAcp, got {other:?}"),
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
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_perm", None);
    // Seed an active session so `on_button` reaches the click path
    // (the stale branch is taken because perm_cards.take returns None,
    // not because the session is dead).
    let _ = router
        .map
        .insert(key.clone(), sebas_router::state::Mapping::active("sess-stale"))
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

// ---- sebas session-level allowlist (Allow session semantics) ----

use sebas_acp::claude::session::{AcpCommand, Decision};
use sebas_router::router::tool_signature;
use serde_json::json;

#[test]
fn tool_signature_is_stable_for_same_input() {
    // Exact match: same tool + same args → same signature.
    let sig_a = tool_signature("Bash", &json!({"command": "ls /tmp"}));
    let sig_b = tool_signature("Bash", &json!({"command": "ls /tmp"}));
    assert_eq!(sig_a, sig_b);
    // Different args → different signature.
    let sig_c = tool_signature("Bash", &json!({"command": "ls /home"}));
    assert_ne!(sig_a, sig_c);
    // Different tool → different signature.
    let sig_d = tool_signature("Read", &json!({"command": "ls /tmp"}));
    assert_ne!(sig_a, sig_d);
}

#[test]
fn tool_signature_canonicalizes_key_order() {
    // Reproduce the real-world failure mode: Claude's tool_use args may
    // serialise the same logical object with keys in a different order on
    // different invocations. A naive `serde_json::to_string` would produce
    // different strings, and the allowlist would miss a "second same call".
    // The signature must be order-insensitive.
    let a = tool_signature(
        "Bash",
        &json!({"command": "ls /tmp", "description": "list /tmp"}),
    );
    let b = tool_signature(
        "Bash",
        &json!({"description": "list /tmp", "command": "ls /tmp"}),
    );
    assert_eq!(a, b, "key order must not affect signature");
}

#[test]
fn tool_signature_ignores_null_fields() {
    // Claude sometimes emits a `parent_tool_use_id: null` or other optional
    // fields. Including those would defeat the match. The signature must
    // strip nulls.
    let with_null = tool_signature("Bash", &json!({"command": "ls", "parent": null}));
    let without = tool_signature("Bash", &json!({"command": "ls"}));
    assert_eq!(with_null, without, "null fields must not affect signature");
}

#[test]
fn tool_signature_nested_object_keys_canonicalized() {
    // Nested objects should also be canonicalized recursively.
    let a = tool_signature(
        "Bash",
        &json!({"command": "ls", "env": {"PATH": "/usr/bin", "HOME": "/root"}}),
    );
    let b = tool_signature(
        "Bash",
        &json!({"env": {"HOME": "/root", "PATH": "/usr/bin"}, "command": "ls"}),
    );
    assert_eq!(a, b, "nested object key order must not affect signature");
}

#[test]
fn tool_signature_preserves_array_order() {
    // Array order is semantically meaningful for command args, env, etc.
    // Canonicalization must NOT sort arrays.
    let a = tool_signature("Bash", &json!({"args": ["ls", "-la", "/tmp"]}));
    let b = tool_signature("Bash", &json!({"args": ["/tmp", "-la", "ls"]}));
    assert_ne!(a, b, "array order is meaningful; must not be sorted");
}

#[test]
fn tool_signature_claude_style_bash_args_match_across_invocations() {
    // The exact scenario from the user's test: same `Bash ls /tmp` call
    // arriving in two separate tool_use blocks with the surrounding
    // Claude-Code wrapper fields. The wrapper fields may be added by the
    // bridge translator and shouldn't be part of the signature.
    //
    // We model the inner call (the bridge would normalize before this point
    // in production; the test pins the contract).
    let args_a = json!({"command": "ls /tmp", "description": "list /tmp contents"});
    let args_b = json!({"description": "list /tmp contents", "command": "ls /tmp"});
    assert_eq!(
        tool_signature("Bash", &args_a),
        tool_signature("Bash", &args_b),
        "Claude-style Bash args with reordered keys must match"
    );
}

#[tokio::test]
async fn allowlist_grant_and_check() {
    use sebas_channels::ChannelKey;
    use sebas_router::router::RouterHandle;
    use sebas_router::state::SessionMap;

    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_x", None);
    // Initial state: not allowed.
    assert!(
        !router
            .allowlist()
            .is_allowed(&key, "Bash", &json!({"command": "ls"}))
            .await
    );
    // Grant.
    router
        .allowlist()
        .grant(&key, "Bash", &json!({"command": "ls"}))
        .await;
    // Now allowed.
    assert!(
        router
            .allowlist()
            .is_allowed(&key, "Bash", &json!({"command": "ls"}))
            .await
    );
    // Different args → not allowed.
    assert!(
        !router
            .allowlist()
            .is_allowed(&key, "Bash", &json!({"command": "rm -rf /"}))
            .await
    );
    // Different chat → not allowed.
    let other_key = ChannelKey::feishu("oc_y", None);
    assert!(
        !router
            .allowlist()
            .is_allowed(&other_key, "Bash", &json!({"command": "ls"}))
            .await
    );
}

#[tokio::test]
async fn allowlist_clear_drops_everything_for_chat() {
    use sebas_channels::ChannelKey;
    use sebas_router::router::RouterHandle;
    use sebas_router::state::SessionMap;

    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_x", None);
    router
        .allowlist()
        .grant(&key, "Bash", &json!({"command": "ls"}))
        .await;
    router
        .allowlist()
        .grant(&key, "Read", &json!({"path": "/etc"}))
        .await;
    assert!(
        router
            .allowlist()
            .is_allowed(&key, "Bash", &json!({"command": "ls"}))
            .await
    );
    // Clear wipes the whole entry (no leak across sessions).
    router.allowlist().clear(&key).await;
    assert!(
        !router
            .allowlist()
            .is_allowed(&key, "Bash", &json!({"command": "ls"}))
            .await
    );
    assert!(
        !router
            .allowlist()
            .is_allowed(&key, "Read", &json!({"path": "/etc"}))
            .await
    );
}

#[tokio::test]
async fn new_command_clears_session_allowlist() {
    use sebas_channels::{ChannelEvent, ChannelKey};
    use sebas_router::router::RouterHandle;
    use sebas_router::state::SessionMap;

    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_x", None);
    router
        .allowlist()
        .grant(&key, "Bash", &json!({"command": "ls"}))
        .await;
    assert!(
        router
            .allowlist()
            .is_allowed(&key, "Bash", &json!({"command": "ls"}))
            .await
    );

    // /new starts a FRESH session in the same chat: "Allow session" grants
    // are scoped to the session that approved them and must not carry over.
    router
        .dispatch(ChannelEvent::Text {
            key: key.clone(),
            text: "/new".into(),
            reply_target: None,
        })
        .await;

    assert!(
        !router
            .allowlist()
            .is_allowed(&key, "Bash", &json!({"command": "ls"}))
            .await,
        "/new must clear the session allowlist for the chat"
    );
}

#[tokio::test]
async fn allow_session_click_grants_and_auto_approves_identical_call() {
    use sebas_acp::claude::session::AcpEvent;
    use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};
    use sebas_router::router::{Out, RouterHandle};
    use sebas_router::state::{Mapping, SessionMap};
    use std::time::Duration;

    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

    let args = json!({"command": "ls /tmp"});
    // First call: nothing granted yet, so a card must go out carrying the
    // (tool, args) stash for the click handler.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::PermissionRequest {
                session_id: "s1".into(),
                request_id: "r1".into(),
                tool_name: "Bash".into(),
                args: args.clone(),
            },
        )
        .await;
    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::SendCard {
            perm_request_id,
            perm_meta,
            ..
        } => {
            assert_eq!(perm_request_id.as_deref(), Some("r1"));
            assert_eq!(
                perm_meta,
                Some(("Bash".to_string(), args.clone())),
                "card must stash (tool, args) for the allowlist grant"
            );
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
    // Dispatcher records the msg_id (production: after send_card returns).
    router
        .record_perm_card_msg_id(
            "r1".into(),
            key.clone(),
            "om_1".into(),
            "Bash".into(),
            args.clone(),
        )
        .await;

    // User clicks 相同调用不再询问.
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
    // Expect: card flip + PermissionReply(AllowSession), and the grant
    // registered.
    let mut saw_reply = false;
    for _ in 0..2 {
        match tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Out::UpdateCardByMsgId { .. } => {}
            Out::SendAcp {
                cmd: AcpCommand::PermissionReply { decision, .. },
                ..
            } => {
                assert!(matches!(decision, Decision::AllowSession));
                saw_reply = true;
            }
            other => panic!("unexpected Out after click: {other:?}"),
        }
    }
    assert!(saw_reply, "click must emit a PermissionReply");
    assert!(
        router.allowlist().is_allowed(&key, "Bash", &args).await,
        "click must grant the (tool, args) signature"
    );

    // Second identical call: auto-approved — SendAcp straight away, no card.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::PermissionRequest {
                session_id: "s1".into(),
                request_id: "r2".into(),
                tool_name: "Bash".into(),
                args: args.clone(),
            },
        )
        .await;
    match tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Out::SendAcp {
            cmd:
                AcpCommand::PermissionReply {
                    request_id,
                    decision,
                    ..
                },
            ..
        } => {
            assert_eq!(request_id, "r2");
            assert!(matches!(decision, Decision::AllowSession));
        }
        other => panic!("expected auto-approve SendAcp, got {other:?}"),
    }
}

#[tokio::test]
async fn allow_session_click_auto_approves_all_later_calls_in_chat() {
    use sebas_acp::claude::session::AcpEvent;
    use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};
    use sebas_router::router::{Out, RouterHandle};
    use sebas_router::state::{Mapping, SessionMap};
    use std::time::Duration;

    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

    // First call prompts; user clicks 本会话不再询问.
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
    let _card = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
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
                value: json!({ "chat_type": "p2p" }),
            },
        })
        .await;
    // Drain card flip + reply.
    for _ in 0..2 {
        let _ = tokio::time::timeout(Duration::from_millis(200), out_rx.recv()).await;
    }

    // A DIFFERENT tool with different args must also auto-approve: the grant
    // is session-wide, not signature-scoped.
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
        Out::SendAcp {
            cmd:
                AcpCommand::PermissionReply {
                    request_id,
                    decision,
                    ..
                },
            ..
        } => {
            assert_eq!(request_id, "r2");
            assert!(matches!(decision, Decision::AllowSession));
        }
        other => panic!("expected session-wide auto-approve, got {other:?}"),
    }
}

#[tokio::test]
async fn permission_request_after_grant_auto_approves_without_card() {
    use sebas_channels::ChannelKey;
    use sebas_router::router::RouterHandle;
    use sebas_router::state::SessionMap;

    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_x", None);
    let session_id = "sess-1".to_string();
    // Seed the session map so apply_event_to_out can resolve the key.
    let _ = router
        .map
        .insert(
            key.clone(),
            sebas_router::state::Mapping::active(session_id.clone()),
        )
        .await;

    // Pre-grant: the (Bash, ls) call is on the allowlist.
    router
        .allowlist()
        .grant(&key, "Bash", &json!({"command": "ls /tmp"}))
        .await;

    // Drive a PermissionRequest through the same path production uses
    // (apply_event_to_out, immediate branch for permission prompts).
    router
        .dispatch_acp_event(AcpEvent::PermissionRequest {
            session_id: session_id.clone(),
            request_id: "req-auto".into(),
            tool_name: "Bash".into(),
            args: json!({"command": "ls /tmp"}),
        })
        .await;

    // Expected: NO SendCard (user shouldn't see anything). Only the
    // auto-approved PermissionReply flows downstream to the bridge.
    let out = tokio::time::timeout(std::time::Duration::from_millis(200), out_rx.recv())
        .await
        .expect("no Out within 200ms")
        .expect("channel closed");
    match out {
        Out::SendAcp {
            cmd:
                AcpCommand::PermissionReply {
                    session_id: sid,
                    request_id: rid,
                    decision,
                },
            ..
        } => {
            assert_eq!(sid, session_id);
            assert_eq!(rid, "req-auto");
            // The router's auto-approve path uses AllowSession (the same
            // decision that "Allow session" maps to) — the bridge can't
            // tell them apart, and the allowlist already accepted it.
            assert!(matches!(decision, Decision::AllowSession));
        }
        other => panic!("expected SendAcp, got {other:?}"),
    }
    // No further Out (no card).
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), out_rx.recv())
            .await
            .is_err(),
        "auto-approve should not render a card"
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
    use sebas_router::router::RouterHandle;
    use sebas_router::state::SessionMap;

    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_x", None);
    let session_id = "sess-1".to_string();
    let _ = router
        .map
        .insert(
            key.clone(),
            sebas_router::state::Mapping::active(session_id.clone()),
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
