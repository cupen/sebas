//! Task 5: continue_session emits per-turn SendCard
//! Task 7: serialize — enqueue when in-flight, ⏳ reaction
//! Task 8: drain queue when in-flight turn settles
//! Task 8 (fix): terminal error abandons queued turns (do NOT drain on terminal Error)

use std::time::Duration;

use acp_claude::session::AcpEvent;
use feishu::events::{FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::{Mapping, SessionMap};

#[tokio::test]
async fn continue_session_emits_per_turn_send_card_with_root_id() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc_x".into(), thread_id: None };
    let _ = router.map.insert(key.clone(), Mapping::active("sess-1")).await;
    router.seed_card("sess-1".into(), "first".into()).await;

    // First turn finishes: use apply_event (pure state, no emission).
    let _ = router.apply_event("sess-1", &AcpEvent::Finished { session_id: "sess-1".into() }).await;

    // User sends a 2nd message that quotes-back to om_user_2.
    // continue_session flips DONE->WORKING, emitting [UpdateCard, React],
    // then emits [SendCard (per-turn), SendAcp].
    router.dispatch(FeishuIn::Text {
        key: key.clone(),
        text: "follow-up".into(),
        reply_to: Some("om_user_2".into()),
    }).await;

    // Drain the flip messages first (UpdateCard + React from DONE->WORKING).
    let _ = out_rx.recv().await; // UpdateCard
    let _ = out_rx.recv().await; // React

    // Now drain and assert the per-turn emissions: SendCard + SendAcp.
    let first = out_rx.recv().await.unwrap();
    let second = out_rx.recv().await.unwrap();
    match (&first, &second) {
        (Out::SendCard { root_id: Some(rid), .. }, Out::SendAcp { .. }) => {
            assert_eq!(rid, "om_user_2");
        }
        _ => panic!("expected SendCard(root_id=Some(_)) then SendAcp, got {first:?} then {second:?}"),
    }
}

#[tokio::test]
async fn continue_while_in_flight_enqueues_no_card_no_sendacp_only_queue_react() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc".into(), thread_id: None };
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    // First turn is mid-flight (no Finished yet) — emoji stays at SEED but the
    // dispatch path marks it WORKING once SendAcp lands; we simulate by
    // flipping it manually for the test.
    router.apply_event_to_out("s1".into(), &AcpEvent::TextDelta {
        session_id: "s1".into(),
        delta: "x".into(),
    }).await;
    let _ = out_rx.recv().await; // drain UpdateCard
    let _ = out_rx.recv().await; // drain React(OnIt) from status transition

    router.dispatch(FeishuIn::Text {
        key: key.clone(),
        text: "second".into(),
        reply_to: Some("om_user_2".into()),
    }).await;

    // Expect: only a React with ⏳ — no SendCard, no SendAcp.
    let out = tokio::time::timeout(Duration::from_millis(50), out_rx.recv())
        .await.unwrap().unwrap();
    match out {
        Out::React { emoji, .. } => assert_eq!(emoji, "⏳"),
        other => panic!("expected React(⏳), got {other:?}"),
    }
    // Nothing else in flight.
    assert!(tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await.is_err());
    // Queue contains the queued turn.
    assert_eq!(router.map.queue_len(&key).await, 1);
}

#[tokio::test]
async fn drain_queue_emits_next_turn_card_and_sendacp_after_finished() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc".into(), thread_id: None };
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    // Mid-flight.
    router.apply_event_to_out("s1".into(), &AcpEvent::TextDelta {
        session_id: "s1".into(), delta: "x".into(),
    }).await; let _ = out_rx.recv().await;
    // Queue 2 turns while in-flight.
    router.dispatch(FeishuIn::Text {
        key: key.clone(), text: "second".into(), reply_to: Some("om2".into()),
    }).await; let _ = out_rx.recv().await; // ⏳ react
    router.dispatch(FeishuIn::Text {
        key: key.clone(), text: "third".into(), reply_to: Some("om3".into()),
    }).await; let _ = out_rx.recv().await; // ⏳ react
    assert_eq!(router.map.queue_len(&key).await, 2);

    // Settle turn 1.
    router.apply_event_to_out("s1".into(), &AcpEvent::Finished { session_id: "s1".into() }).await;
    let _ = out_rx.recv().await; // UpdateCard (✅)
    let _ = out_rx.recv().await; // React ✅

    // Now turn 2 should drain: SendCard(root_id=om2) + SendAcp("second")
    let first = out_rx.recv().await.unwrap();
    let second = out_rx.recv().await.unwrap();
    match (&first, &second) {
        (Out::SendCard { root_id: Some(rid), .. }, Out::SendAcp { .. }) => {
            assert_eq!(rid, "om2");
        }
        _ => panic!("expected SendCard(om2) + SendAcp, got {first:?} then {second:?}"),
    }
    assert_eq!(router.map.queue_len(&key).await, 1);

    // Settle turn 2.
    router.apply_event_to_out("s1".into(), &AcpEvent::Finished { session_id: "s1".into() }).await;
    let _ = out_rx.recv().await; let _ = out_rx.recv().await; // UpdateCard + React
    let third_a = out_rx.recv().await.unwrap();
    let third_b = out_rx.recv().await.unwrap();
    match (&third_a, &third_b) {
        (Out::SendCard { root_id: Some(rid), .. }, _) => assert_eq!(rid, "om3"),
        _ => panic!("expected SendCard(om3), got {third_a:?}"),
    }
    assert_eq!(router.map.queue_len(&key).await, 0);
}

#[tokio::test]
async fn terminal_error_abandons_queued_turns() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc".into(), thread_id: None };
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    // Mid-flight.
    router.apply_event_to_out("s1".into(), &AcpEvent::TextDelta {
        session_id: "s1".into(), delta: "x".into(),
    }).await;
    let _ = out_rx.recv().await;
    // Queue a turn.
    router.dispatch(FeishuIn::Text {
        key: key.clone(), text: "second".into(), reply_to: Some("om2".into()),
    }).await;
    let _ = out_rx.recv().await; // ⏳ react
    assert_eq!(router.map.queue_len(&key).await, 1);
    // Terminal error.
    router.apply_event_to_out("s1".into(), &AcpEvent::Error {
        session_id: "s1".into(), message: "dead".into(), terminal: true,
    }).await;
    // Drain all messages after terminal error.
    let mut msgs = vec![];
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await {
        msgs.push(msg);
    }
    // Assert no SendAcp was emitted for the abandoned queued turn.
    for msg in &msgs {
        assert!(!matches!(msg, Out::SendAcp { .. }), "unexpected SendAcp: {:?}", msg);
    }
    // Session is gone — the queued turn is abandoned.
    assert!(!router.session_alive(&key).await);
}
