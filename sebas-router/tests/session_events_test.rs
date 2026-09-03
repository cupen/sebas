//! Session event stream + external snapshot (openspec/changes/add-core-session-channel
//! tasks 1.2/1.3): subscribe → create → status change → remove yields the exact
//! event sequence, and applying events to a snapshot reproduces the router's own
//! state.

use sebas_feishu::events::SessionKey;
use sebas_router::router::SessionEvent;
use sebas_router::state::{Mapping, SessionMap};
use sebas_router::RouterHandle;
use std::collections::HashMap;

fn key(id: &str) -> SessionKey {
    SessionKey {
        chat_id: format!("oc_{id}"),
        thread_id: None,
    }
}

/// Task 1.2: create → status change → remove publishes the exact sequence.
#[tokio::test]
async fn events_follow_create_status_change_remove() {
    let map = SessionMap::new();
    let (router, _rx) = RouterHandle::new(map);
    let mut events = router.subscribe_session_events();

    // create: web_spawn inserts a Spawning placeholder.
    let key = router.web_spawn("hello world".into(), Some("/tmp/p".into()), None).await;
    // status change: Spawning → Active.
    router.activate(&key, "s1".into()).await;
    // remove.
    let outcome = router.web_close_session(key.clone()).await;
    assert_eq!(outcome, sebas_router::router::CloseOutcome::Closed);

    let mut seq = Vec::new();
    while let Ok(ev) = events.try_recv() {
        seq.push(ev);
    }

    assert_eq!(seq.len(), 3, "expected exactly [Created, Updated, Removed], got {seq:?}");
    match &seq[0] {
        SessionEvent::Created { session } => {
            assert_eq!(session.chat_id, key.chat_id);
            assert_eq!(session.status, "spawning");
            assert_eq!(session.project_dir.as_deref(), Some("/tmp/p"));
            assert_eq!(session.session_id, None);
        }
        other => panic!("first event should be Created, got {other:?}"),
    }
    match &seq[1] {
        SessionEvent::Updated { session } => {
            assert_eq!(session.status, "active");
            assert_eq!(session.session_id.as_deref(), Some("s1"));
        }
        other => panic!("second event should be Updated, got {other:?}"),
    }
    match &seq[2] {
        SessionEvent::Removed { chat_id, thread_id } => {
            assert_eq!(chat_id, &key.chat_id);
            assert_eq!(thread_id, &None);
        }
        other => panic!("third event should be Removed, got {other:?}"),
    }
}

/// Task 1.2: emoji phase transition publishes Updated with the new phase.
#[tokio::test]
async fn phase_transition_publishes_updated_with_phase() {
    let map = SessionMap::new();
    let (router, _rx) = RouterHandle::new(map);
    let mut events = router.subscribe_session_events();

    let k = key("b");
    router
        .map
        .insert(k.clone(), Mapping::active("s-b"))
        .await
        .unwrap();
    router.seed_card("s-b".into(), "fix the bug".into()).await;

    // TextDelta does not transition the FSM (SEED → WORKING only on... actually
    // TextDelta moves SEED → WORKING per next_emoji). Drive one TextDelta and
    // assert an Updated event carrying the WORKING phase arrives.
    use sebas_acp::claude::session::AcpEvent;
    router
        .apply_event("s-b", &AcpEvent::TextDelta {
            session_id: "s-b".into(),
            delta: "working on it".into(),
        })
        .await;

    let mut saw_working = false;
    while let Ok(ev) = events.try_recv() {
        if let SessionEvent::Updated { session } = ev
            && session.session_id.as_deref() == Some("s-b")
            && session.phase.as_deref() == Some(sebas_router::card_state::phase::WORKING)
        {
            saw_working = true;
        }
    }
    assert!(saw_working, "expected an Updated event with the WORKING phase");
}

/// Task 1.3: applying published events to a snapshot reproduces the router's
/// own state.
#[tokio::test]
async fn applying_events_to_snapshot_reproduces_router_state() {
    let map = SessionMap::new();
    let (router, _rx) = RouterHandle::new(map);
    let mut events = router.subscribe_session_events();

    // Client-side cache: (chat_id, thread_id) → SessionInfo.
    let mut cache: HashMap<(String, Option<String>), sebas_router::SessionInfo> = HashMap::new();

    let ka = key("a");
    router
        .map
        .insert(ka.clone(), Mapping::dormant("s-a", 42))
        .await
        .unwrap();
    let kb_key = router.web_spawn("spawn me".into(), None, None).await;
    router.activate(&kb_key, "s-b".into()).await;
    let _ = router.web_close_session(ka).await;

    // Fold: snapshot BEFORE the mutations? No — take the snapshot now and fold
    // only the buffered events on top of an empty cache; the result must equal
    // the router's own snapshot.
    let snapshot = router.session_info_snapshot().await;
    while let Ok(ev) = events.try_recv() {
        match ev {
            SessionEvent::Created { session } | SessionEvent::Updated { session } => {
                cache.insert(
                    (session.chat_id.clone(), session.thread_id.clone()),
                    session,
                );
            }
            SessionEvent::Removed {
                chat_id,
                thread_id,
            } => {
                cache.remove(&(chat_id, thread_id));
            }
            SessionEvent::Resync => {}
        }
    }

    assert_eq!(cache.len(), snapshot.len(), "cache {cache:?} vs snapshot {snapshot:?}");
    for info in &snapshot {
        let cached = cache
            .get(&(info.chat_id.clone(), info.thread_id.clone()))
            .expect("snapshot session present in cache");
        assert_eq!(cached, info);
    }
    // Exactly the surviving web session remains, active (web_spawn minted its
    // own `web-{nanos}` key — kb was never given a mapping).
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].chat_id, kb_key.chat_id);
    assert_eq!(snapshot[0].status, "active");
    assert_eq!(snapshot[0].session_id.as_deref(), Some("s-b"));
}

/// Task 1.3 companion: transcript positions are monotonic and `session_turns`
/// returns only entries at or after the requested position.
#[tokio::test]
async fn turns_are_incremental_by_position() {
    use sebas_acp::claude::session::AcpEvent;
    let map = SessionMap::new();
    let (router, _rx) = RouterHandle::new(map);
    let k = key("t");
    router
        .map
        .insert(k.clone(), Mapping::active("s-t"))
        .await
        .unwrap();
    router.seed_card("s-t".into(), "do things".into()).await;

    let delta = |d: &str| AcpEvent::TextDelta {
        session_id: "s-t".into(),
        delta: d.into(),
    };
    router.apply_event("s-t", &delta("one")).await;
    router.apply_event("s-t", &delta("two")).await;
    router.apply_event("s-t", &delta("three")).await;

    let all = router.session_turns(&k, 0).await.unwrap();
    // prompt + three deltas
    assert_eq!(all.len(), 4);
    assert_eq!(all[0].kind, "prompt");
    assert_eq!(all[3].content, "three");

    let after = router.session_turns(&k, 3).await.unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].position, 3);
    assert_eq!(after[0].content, "three");

    // Unknown key → None; known key with no content → empty.
    assert!(router.session_turns(&key("zzz"), 0).await.is_none());
}
