//! Task 5: continue_session emits per-turn SendCard
//! Task 7: serialize — enqueue when in-flight, ⏳ reaction
//! Task 8: drain queue when in-flight turn settles
//! Task 8 (fix): terminal error abandons queued turns (do NOT drain on terminal Error)
//! Task 9: /btw command — priority slot in turn queue
//! Task 11: gap-filling — reply_to None is fire-and-forget; explicit 3-turn sequential

use std::time::Duration;

use sebas_acp::claude::session::AcpEvent;
use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::{Mapping, SessionMap};

#[tokio::test]
async fn continue_session_emits_per_turn_send_card_with_root_id() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc_x".into(), None);
    let _ = router
        .map
        .insert(key.clone(), Mapping::active("sess-1"))
        .await;
    router.seed_card("sess-1".into(), "first".into()).await;

    // First turn finishes: use apply_event (pure state, no emission).
    let _ = router
        .apply_event(
            "sess-1",
            &AcpEvent::Finished {
                session_id: "sess-1".into(),
            },
        )
        .await;

    // User sends a 2nd message that quotes-back to om_user_2.
    // continue_session flips DONE->WORKING, emitting [UpdateCard, React],
    // then emits [SendCard (per-turn), SendAcp].
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "follow-up".into(),
            reply_target: Some("om_user_2".into()), })
        .await;

    // Drain the flip messages first. `dispatch` acks the user message with an
    // immediate EYES reaction before processing (upstream ack mechanism), so
    // order is: AckMsg(EYES), UpdateCard, React (DONE->WORKING flip).
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack
    let _ = out_rx.recv().await; // UpdateCard
    let _ = out_rx.recv().await; // React

    // Now drain and assert the per-turn emissions: SendCard + SendAcp.
    let first = out_rx.recv().await.unwrap();
    let second = out_rx.recv().await.unwrap();
    match (&first, &second) {
        (
            Out::SendCard {
                root_id: Some(rid),
                msg_id: Some(mid),
                ..
            },
            Out::SendAcp { .. },
        ) => {
            assert_eq!(rid, "om_user_2");
            // The per-turn card must be recorded under the session so the
            // dispatcher flips MsgIdMap to it (streaming PATCHes this card,
            // not the previous turn's).
            assert_eq!(mid, "sess-1");
        }
        _ => {
            panic!("expected SendCard(root_id=Some(_)) then SendAcp, got {first:?} then {second:?}")
        }
    }
}

#[tokio::test]
async fn terminal_error_clears_queued_turns() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc".into(), None);
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;

    // Mid-flight.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::TextDelta {
                session_id: "s1".into(),
                delta: "x".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await;

    // Queue a turn while in-flight.
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "second".into(),
            reply_target: Some("om2".into()), })
        .await;
    let _ = out_rx.recv().await; // ⏳ react
    assert_eq!(router.map.queue_len(&key).await, 1);

    // Terminal error tears the session down and must drop the queue so the
    // prompt never drains into a future session for the same chat.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Error {
                session_id: "s1".into(),
                message: "dead".into(),
                terminal: true,
            },
        )
        .await;
    assert!(!router.session_alive(&key).await);
    assert_eq!(
        router.map.queue_len(&key).await,
        0,
        "queue must be cleared on teardown"
    );

    // No SendAcp was emitted for the abandoned queued turn.
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await {
        assert!(
            !matches!(msg, Out::SendAcp { .. }),
            "unexpected SendAcp: {msg:?}"
        );
    }
}

#[tokio::test]
async fn continue_while_in_flight_enqueues_no_card_no_sendacp_only_queue_react() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc".into(), None);
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    // First turn is mid-flight (no Finished yet) — emoji stays at SEED but the
    // dispatch path marks it WORKING once SendAcp lands; we simulate by
    // flipping it manually for the test.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::TextDelta {
                session_id: "s1".into(),
                delta: "x".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await; // drain UpdateCard
    let _ = out_rx.recv().await; // drain React(OnIt) from status transition

    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "second".into(),
            reply_target: Some("om_user_2".into()), })
        .await;
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack

    // Expect: only a React with ⏳ — no SendCard, no SendAcp.
    let out = tokio::time::timeout(Duration::from_millis(50), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::React { emoji, .. } => assert_eq!(emoji, "⏳"),
        other => panic!("expected React(⏳), got {other:?}"),
    }
    // Nothing else in flight.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), out_rx.recv())
            .await
            .is_err()
    );
    // Queue contains the queued turn.
    assert_eq!(router.map.queue_len(&key).await, 1);
}

#[tokio::test]
async fn drain_queue_emits_next_turn_card_and_sendacp_after_finished() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc".into(), None);
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    // Mid-flight.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::TextDelta {
                session_id: "s1".into(),
                delta: "x".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await; // UpdateCard
    let _ = out_rx.recv().await; // React WORKING
    // Queue 2 turns while in-flight.
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "second".into(),
            reply_target: Some("om2".into()), })
        .await;
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack
    let _ = out_rx.recv().await; // ⏳ react
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "third".into(),
            reply_target: Some("om3".into()), })
        .await;
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack
    let _ = out_rx.recv().await; // ⏳ react
    assert_eq!(router.map.queue_len(&key).await, 2);

    // Settle turn 1.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Finished {
                session_id: "s1".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await; // UpdateCard（终态）
    let _ = out_rx.recv().await; // React DONE（997bfe2 恢复终态 reaction）

    // Now turn 2 should drain: SendCard(root_id=om2) + SendAcp("second")
    let first = out_rx.recv().await.unwrap();
    let second = out_rx.recv().await.unwrap();
    match (&first, &second) {
        (
            Out::SendCard {
                root_id: Some(rid), ..
            },
            Out::SendAcp { .. },
        ) => {
            assert_eq!(rid, "om2");
        }
        _ => panic!("expected SendCard(om2) + SendAcp, got {first:?} then {second:?}"),
    }
    assert_eq!(router.map.queue_len(&key).await, 1);

    // Settle turn 2.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Finished {
                session_id: "s1".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await; // UpdateCard（终态）
    let _ = out_rx.recv().await; // React DONE（997bfe2 恢复终态 reaction）
    let third_a = out_rx.recv().await.unwrap();
    let third_b = out_rx.recv().await.unwrap();
    match (&third_a, &third_b) {
        (
            Out::SendCard {
                root_id: Some(rid), ..
            },
            _,
        ) => assert_eq!(rid, "om3"),
        _ => panic!("expected SendCard(om3), got {third_a:?}"),
    }
    assert_eq!(router.map.queue_len(&key).await, 0);
}

#[tokio::test]
async fn terminal_error_abandons_queued_turns() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc".into(), None);
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    // Mid-flight.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::TextDelta {
                session_id: "s1".into(),
                delta: "x".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await;
    // Queue a turn.
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "second".into(),
            reply_target: Some("om2".into()), })
        .await;
    let _ = out_rx.recv().await; // ⏳ react
    assert_eq!(router.map.queue_len(&key).await, 1);
    // Terminal error.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Error {
                session_id: "s1".into(),
                message: "dead".into(),
                terminal: true,
            },
        )
        .await;
    // Drain all messages after terminal error.
    let mut msgs = vec![];
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await {
        msgs.push(msg);
    }
    // Assert no SendAcp was emitted for the abandoned queued turn.
    for msg in &msgs {
        assert!(
            !matches!(msg, Out::SendAcp { .. }),
            "unexpected SendAcp: {:?}",
            msg
        );
    }
    // Session is gone — the queued turn is abandoned.
    assert!(!router.session_alive(&key).await);
}

#[tokio::test]
async fn btw_command_queues_with_priority_ahead_of_existing_fifo() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc".into(), None);
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::TextDelta {
                session_id: "s1".into(),
                delta: "x".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await; // UpdateCard
    let _ = out_rx.recv().await; // React WORKING

    // Queue a normal FIFO turn first.
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "fifo".into(),
            reply_target: Some("omF".into()), })
        .await;
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack
    let _ = out_rx.recv().await; // ⏳

    // Now a /btw turn — must jump to front.
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "/btw btw".into(),
            reply_target: Some("omB".into()), })
        .await;
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack
    let _ = out_rx.recv().await; // ⏳

    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Finished {
                session_id: "s1".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await;
    let _ = out_rx.recv().await; // UpdateCard + React ✅

    // Drain: first SendCard should be the /btw one (omB), not FIFO (omF).
    let first = out_rx.recv().await.unwrap();
    match first {
        Out::SendCard {
            root_id: Some(rid), ..
        } => assert_eq!(rid, "omB"),
        other => panic!("expected SendCard(omB), got {other:?}"),
    }
    let second = out_rx.recv().await.unwrap();
    match second {
        Out::SendAcp { .. } => {}
        other => panic!("expected SendAcp, got {other:?}"),
    }
}

/// Task 11: verify that a message with no reply_to (reply_to: None) emits a
/// SendCard with root_id: None — fire-and-forget per-turn card with no threading.
#[tokio::test]
async fn missing_reply_to_is_fire_and_forget_root_id_none() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc".into(), None);
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;

    // Finish the first turn so the session is DONE.
    let _ = router
        .apply_event(
            "s1",
            &AcpEvent::Finished {
                session_id: "s1".into(),
            },
        )
        .await;

    // User sends a message with NO reply_to (e.g. fresh message, no quote).
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "hello".into(),
            reply_target: None, })
        .await;

    // Drain DONE->WORKING flip (UpdateCard + React).
    let _ = out_rx.recv().await; // UpdateCard
    let _ = out_rx.recv().await; // React

    // The per-turn SendCard must have root_id: None (fire-and-forget).
    let first = out_rx.recv().await.unwrap();
    let second = out_rx.recv().await.unwrap();
    match (&first, &second) {
        (Out::SendCard { root_id: None, .. }, Out::SendAcp { .. }) => {}
        _ => panic!("expected SendCard(root_id=None) then SendAcp, got {first:?} then {second:?}"),
    }
}

/// Task 11: three sequential turns — each dispatch yields a distinct root_id.
#[tokio::test]
async fn three_turns_three_distinct_root_ids() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc".into(), None);
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;

    // ── Turn 1 ──────────────────────────────────────────────────────────────
    // Finish the seeded turn so session is DONE.
    let _ = router
        .apply_event(
            "s1",
            &AcpEvent::Finished {
                session_id: "s1".into(),
            },
        )
        .await;
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "turn 1".into(),
            reply_target: Some("om1".into()), })
        .await;
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack
    let _ = out_rx.recv().await; // UpdateCard (DONE->WORKING flip)
    let _ = out_rx.recv().await; // React
    let (sc1, acp1) = (out_rx.recv().await.unwrap(), out_rx.recv().await.unwrap());
    let root1 = match (&sc1, &acp1) {
        (
            Out::SendCard {
                root_id: Some(r), ..
            },
            Out::SendAcp { .. },
        ) => r.clone(),
        _ => panic!("turn 1: expected SendCard(Some) + SendAcp"),
    };
    assert_eq!(root1, "om1");

    // ── Turn 2 ──────────────────────────────────────────────────────────────
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Finished {
                session_id: "s1".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await; // UpdateCard
    let _ = out_rx.recv().await; // React
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "turn 2".into(),
            reply_target: Some("om2".into()), })
        .await;
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack
    let _ = out_rx.recv().await; // UpdateCard (DONE->WORKING flip)
    let _ = out_rx.recv().await; // React
    let (sc2, acp2) = (out_rx.recv().await.unwrap(), out_rx.recv().await.unwrap());
    let root2 = match (&sc2, &acp2) {
        (
            Out::SendCard {
                root_id: Some(r), ..
            },
            Out::SendAcp { .. },
        ) => r.clone(),
        _ => panic!("turn 2: expected SendCard(Some) + SendAcp"),
    };
    assert_eq!(root2, "om2");
    assert_ne!(root2, root1, "turn 2 root_id must differ from turn 1");

    // ── Turn 3 ──────────────────────────────────────────────────────────────
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Finished {
                session_id: "s1".into(),
            },
        )
        .await;
    let _ = out_rx.recv().await; // UpdateCard
    let _ = out_rx.recv().await; // React
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "turn 3".into(),
            reply_target: Some("om3".into()), })
        .await;
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack
    let _ = out_rx.recv().await; // UpdateCard (DONE->WORKING flip)
    let _ = out_rx.recv().await; // React
    let (sc3, acp3) = (out_rx.recv().await.unwrap(), out_rx.recv().await.unwrap());
    let root3 = match (&sc3, &acp3) {
        (
            Out::SendCard {
                root_id: Some(r), ..
            },
            Out::SendAcp { .. },
        ) => r.clone(),
        _ => panic!("turn 3: expected SendCard(Some) + SendAcp"),
    };
    assert_eq!(root3, "om3");
    assert_ne!(root3, root2, "turn 3 root_id must differ from turn 2");
    assert_ne!(root3, root1, "turn 3 root_id must differ from turn 1");
}

/// Task 11 (reviewer gap-fill): after a 2nd turn's SendCard is emitted, the
/// MsgIdMap flips to the new msg_id. A streaming event after that must produce
/// an UpdateCard that the dispatcher will resolve via MsgIdMap -> msg_id_2
/// (the new turn's card), not msg_id_1 (the previous turn's card, which stays frozen).
#[tokio::test]
async fn streaming_update_after_second_turn_targets_current_card() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = ChannelKey::feishu("oc".into(), None);
    let _ = router.map.insert(key.clone(), Mapping::active("s1")).await;
    router.seed_card("s1".into(), "first".into()).await;

    // Settle turn 1.
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::Finished {
                session_id: "s1".into(),
            },
        )
        .await;
    // Drain 2 (UpdateCard + React from the settled state)
    let _ = out_rx.recv().await;
    let _ = out_rx.recv().await;

    // Manually record msg_id for the first turn (simulating dispatcher having POSTed the seed card).
    router
        .record_root_msg_id("s1".into(), "om_msg_1".into())
        .await;

    // User sends turn 2.
    router
        .dispatch(ChannelEvent::Text { 
            key: key.clone(),
            text: "follow-up".into(),
            reply_target: Some("om_user_2".into()), })
        .await;
    // Drain 5 messages from turn 2: AckMsg(EYES) + UpdateCard (DONE→WORKING
    // flip) + React + SendCard + SendAcp
    let _ = out_rx.recv().await; // AckMsg(EYES) — immediate receipt ack
    let _ = out_rx.recv().await;
    let _ = out_rx.recv().await;
    let send_card_msg = out_rx.recv().await.unwrap();
    let _ = out_rx.recv().await;
    // Confirm turn 2 emitted SendCard with root_id=om_user_2.
    match send_card_msg {
        Out::SendCard {
            root_id: Some(rid), ..
        } => assert_eq!(rid, "om_user_2"),
        other => panic!("expected SendCard(root_id=om_user_2), got {other:?}"),
    }

    // Simulate dispatcher POSTing turn 2's card and recording its msg_id (msg_id_2).
    router
        .record_root_msg_id("s1".into(), "om_msg_2".into())
        .await;

    // Streaming event for turn 2 (TextDelta).
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::TextDelta {
                session_id: "s1".into(),
                delta: "streaming chunk".into(),
            },
        )
        .await;
    // Drain 2: UpdateCard (with the new body content) + React if emoji changed.
    let update_msg = out_rx.recv().await.unwrap();
    match update_msg {
        Out::UpdateCard { session_id, .. } => {
            // The router emits by session_id; the dispatcher resolves via
            // MsgIdMap (which now points to msg_id_2). Assert the MsgIdMap
            // is indeed pointing at msg_id_2.
            assert_eq!(
                router.root_msg_id("s1").await.as_deref(),
                Some("om_msg_2"),
                "MsgIdMap should point at the most recent (2nd turn) msg_id"
            );
            assert_eq!(session_id, "s1");
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
}
