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
    assert_eq!(snap.status_emoji, router::card_state::phase::SEED);
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
    assert_eq!(snap.status_emoji, router::card_state::phase::SEED);
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
    assert_eq!(a.status_emoji, router::card_state::phase::SEED);
    assert!(a.body.is_empty());
    let b = CardState::lazy();
    assert_eq!(b.user_prompt, "");
    assert_eq!(b.status_emoji, router::card_state::phase::SEED);
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
        matches!(o2, Out::React { ref emoji, .. } if emoji == router::card_state::phase::WORKING),
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

    // Finished → ✅
    router
        .apply_event_to_out("r1".into(), &AcpEvent::Finished { session_id: "r1".into() })
        .await;
    let o4 = recv(&mut out_rx).await;
    assert!(matches!(o4, Out::UpdateCard { .. }), "先出卡: {o4:?}");
    let o5 = recv(&mut out_rx).await;
    assert!(
        matches!(o5, Out::React { ref emoji, .. } if emoji == router::card_state::phase::DONE),
        "Finished 换 DONE: {o5:?}"
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
        matches!(o2, Out::React { ref emoji, .. } if emoji == router::card_state::phase::FAILED),
        "terminal Error 换 FAILED: {o2:?}"
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
    // 驱动到 DONE（纯状态，无 Out）
    let react = router
        .apply_event("r3", &AcpEvent::Finished { session_id: "r3".into() })
        .await;
    assert_eq!(react, Some(router::card_state::phase::DONE), "apply_event 报告 SEED→DONE 转移");

    // 用户追问：continue 回切 WORKING —— 先刷卡，再换 reaction，最后 SendAcp
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
        matches!(o2, Out::React { ref emoji, .. } if emoji == router::card_state::phase::WORKING),
        "回切 reaction WORKING: {o2:?}"
    );
    let o3 = recv(&mut out_rx).await;
    assert!(matches!(o3, Out::SendCard { root_id: None, .. }), "per-turn card: {o3:?}");
    let o4 = recv(&mut out_rx).await;
    assert!(matches!(o4, Out::SendAcp { .. }), "继续会话: {o4:?}");
    assert_no_more(&mut out_rx).await;
}

// ---- sebas card-flip: permission card click feedback ----

use feishu::cards::render_resolved_permission_card;
use feishu::events::{CardAction, FeishuIn, SessionKey};

#[tokio::test]
async fn permission_card_click_emits_resolved_card_flip() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey {
        chat_id: "oc_perm".into(),
        thread_id: None,
    };
    // Seed an active session mapping so `on_button` passes the
    // `session_alive` check (production: the session is alive while
    // there's a Claude child process for this chat).
    router
        .map
        .insert(key.clone(), router::state::Mapping::active("sess-flip"))
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
        .dispatch(FeishuIn::ButtonCb {
            key: key.clone(),
            action: CardAction {
                decision: Some("allow_once".into()),
                session_id: "sess-1".into(),
                request_id: Some("req-1".into()),
                value: serde_json::json!({}),
            },
        })
        .await;
    // First Out: UpdateCardByMsgId that flips the original card in place.
    let o1 = recv(&mut out_rx).await;
    let msg_id = match &o1 {
        Out::UpdateCardByMsgId { key: k, msg_id, .. } => {
            assert_eq!(k.chat_id, "oc_perm");
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
            cmd: acp_claude::session::AcpCommand::PermissionReply { request_id, decision, .. },
            ..
        } => {
            assert_eq!(request_id, "req-1");
            assert!(matches!(
                decision,
                acp_claude::session::Decision::AllowOnce
            ));
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
    let key = SessionKey {
        chat_id: "oc_perm".into(),
        thread_id: None,
    };
    // Seed an active session so `on_button` reaches the click path
    // (the stale branch is taken because perm_cards.take returns None,
    // not because the session is dead).
    router
        .map
        .insert(key.clone(), router::state::Mapping::active("sess-stale"))
        .await;
    router
        .dispatch(FeishuIn::ButtonCb {
            key: key.clone(),
            action: CardAction {
                decision: Some("allow_once".into()),
                session_id: "sess-1".into(),
                request_id: Some("req-stale".into()),
                value: serde_json::json!({}),
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
    assert_eq!(k.chat_id, "oc_perm");
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

use acp_claude::session::{AcpCommand, Decision};
use router::router::tool_signature;
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
    let a = tool_signature("Bash", &json!({"command": "ls /tmp", "description": "list /tmp"}));
    let b = tool_signature("Bash", &json!({"description": "list /tmp", "command": "ls /tmp"}));
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
    let a = tool_signature("Bash", &json!({"command": "ls", "env": {"PATH": "/usr/bin", "HOME": "/root"}}));
    let b = tool_signature("Bash", &json!({"env": {"HOME": "/root", "PATH": "/usr/bin"}, "command": "ls"}));
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
    use router::router::RouterHandle;
    use router::state::SessionMap;
    use feishu::events::SessionKey;

    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    // Initial state: not allowed.
    assert!(
        !router.allowlist()
            
            .is_allowed(&key, "Bash", &json!({"command": "ls"}))
            .await
    );
    // Grant.
    router.allowlist()
        
        .grant(&key, "Bash", &json!({"command": "ls"}))
        .await;
    // Now allowed.
    assert!(
        router.allowlist()
            
            .is_allowed(&key, "Bash", &json!({"command": "ls"}))
            .await
    );
    // Different args → not allowed.
    assert!(
        !router.allowlist()
            
            .is_allowed(&key, "Bash", &json!({"command": "rm -rf /"}))
            .await
    );
    // Different chat → not allowed.
    let other_key = SessionKey {
        chat_id: "oc_y".into(),
        thread_id: None,
    };
    assert!(
        !router.allowlist()
            
            .is_allowed(&other_key, "Bash", &json!({"command": "ls"}))
            .await
    );
}

#[tokio::test]
async fn allowlist_clear_drops_everything_for_chat() {
    use router::router::RouterHandle;
    use router::state::SessionMap;
    use feishu::events::SessionKey;

    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    router.allowlist().grant(&key, "Bash", &json!({"command": "ls"})).await;
    router.allowlist().grant(&key, "Read", &json!({"path": "/etc"})).await;
    assert!(router.allowlist().is_allowed(&key, "Bash", &json!({"command": "ls"})).await);
    // Clear wipes the whole entry (no leak across sessions).
    router.allowlist().clear(&key).await;
    assert!(!router.allowlist().is_allowed(&key, "Bash", &json!({"command": "ls"})).await);
    assert!(!router.allowlist().is_allowed(&key, "Read", &json!({"path": "/etc"})).await);
}

#[tokio::test]
async fn permission_request_after_grant_auto_approves_without_card() {
    use router::router::RouterHandle;
    use router::state::SessionMap;
    use feishu::events::SessionKey;

    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    let session_id = "sess-1".to_string();
    // Seed the session map so apply_event_to_out can resolve the key.
    router
        .map
        .insert(key.clone(), router::state::Mapping::active(session_id.clone()))
        .await;

    // Pre-grant: the (Bash, ls) call is on the allowlist.
    router.allowlist()
        
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
        key: feishu::events::SessionKey {
            chat_id: "oc_test".into(),
            thread_id: None,
        },
        card: serde_json::json!({"type": "card"}),
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
        key: feishu::events::SessionKey {
            chat_id: "oc_test".into(),
            thread_id: None,
        },
        card: serde_json::json!({"type": "card"}),
        msg_id: None,
        perm_request_id: None,
        perm_meta: None,
        root_id: None,
    };
    let s = format!("{:?}", out);
    assert!(s.contains("root_id"), "Debug output should contain root_id: {s}");
}

#[tokio::test]
async fn permission_request_without_grant_still_renders_card() {
    // Sanity counterpart to the auto-approve test: a fresh (Bash, ls)
    // call when nothing is on the allowlist must still show the card so
    // the user can decide.
    use router::router::RouterHandle;
    use router::state::SessionMap;
    use feishu::events::SessionKey;

    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    let session_id = "sess-1".to_string();
    router
        .map
        .insert(key.clone(), router::state::Mapping::active(session_id.clone()))
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
        Out::SendCard { key: k, perm_request_id, .. } => {
            assert_eq!(k.chat_id, "oc_x");
            assert_eq!(perm_request_id.as_deref(), Some("req-fresh"));
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
}
