//! Router session event source (openspec/changes/add-core-session-channel
//! tasks 1.2 / 1.3): every mapping mutation publishes a `SessionEvent`, and
//! a fresh subscriber can fold the event stream onto the snapshot accessor
//! and land exactly on the router's own state.

use sebas_router::router::{RouterHandle, SessionEvent, SessionSnapshot, SessionState};
use sebas_router::state::SessionMap;
use sebas_feishu::events::SessionKey;

/// Stable string form of a SessionKey for map/set style comparisons.
fn key_string(key: &SessionKey) -> String {
    serde_json::to_value(key).unwrap().as_str().unwrap().to_string()
}

fn fold(events: &[SessionEvent]) -> Vec<SessionSnapshot> {
    let mut state: Vec<SessionSnapshot> = Vec::new();
    for event in events {
        match event {
            SessionEvent::Created { session } | SessionEvent::Updated { session } => {
                if let Some(existing) = state
                    .iter_mut()
                    .find(|s| s.key == session.key)
                {
                    *existing = session.clone();
                } else {
                    state.push(session.clone());
                }
            }
            SessionEvent::Removed { key } => {
                state.retain(|s| s.key != *key);
            }
        }
    }
    state.sort_by_key(|s| std::cmp::Reverse(s.last_active_unix));
    state
}

async fn drain(rx: &mut tokio::sync::broadcast::Receiver<SessionEvent>) -> Vec<SessionEvent> {
    let mut out = Vec::new();
    while let Ok(event) = rx.try_recv() {
        out.push(event);
    }
    out
}

/// Task 1.2: create → status change → remove publishes exactly
/// Created(spawning) → Updated(active) → Removed.
#[tokio::test]
async fn create_then_activate_then_close_publishes_exact_sequence() {
    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let mut events = router.session_events();

    let key = router.web_spawn("build the thing".into(), None).await;
    router.activate(&key, "ses_abc".to_string()).await;
    let outcome = router.web_close_session(key.clone()).await;
    assert_eq!(outcome, sebas_router::router::CloseOutcome::Closed);

    let got = drain(&mut events).await;
    assert_eq!(got.len(), 3, "expected exactly three events, got {got:?}");

    match &got[0] {
        SessionEvent::Created { session } => {
            assert_eq!(session.key, key);
            assert_eq!(session.state, SessionState::Spawning);
            assert_eq!(session.session_id, None);
        }
        other => panic!("first event should be Created, got {other:?}"),
    }
    match &got[1] {
        SessionEvent::Updated { session } => {
            assert_eq!(session.key, key);
            assert_eq!(session.state, SessionState::Active);
            assert_eq!(session.session_id.as_deref(), Some("ses_abc"));
        }
        other => panic!("second event should be Updated, got {other:?}"),
    }
    match &got[2] {
        SessionEvent::Removed { key: removed } => assert_eq!(removed, &key),
        other => panic!("third event should be Removed, got {other:?}"),
    }
}

/// Task 1.3: folding the events onto an empty view reproduces
/// `session_snapshots()` exactly (identity, state, phase, recency, dir).
#[tokio::test]
async fn folded_events_reproduce_the_router_snapshot() {
    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let mut events = router.session_events();

    let web = router
        .web_spawn("project session".into(), Some("/tmp/proj".into()))
        .await;
    let feishu_key = SessionKey {
        chat_id: "oc_fold".into(),
        thread_id: None,
    };
    // Feishu-originated path: text arrives before any mapping exists.
    router
        .dispatch(sebas_feishu::events::FeishuIn::Text {
            key: feishu_key.clone(),
            text: "hello".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;

    let got = drain(&mut events).await;
    let folded = fold(&got);
    let actual = router.session_snapshots().await;

    assert_eq!(folded.len(), actual.len(), "folded {folded:?} vs {actual:?}");
    let mut folded_by_key: Vec<(String, SessionSnapshot)> = folded
        .into_iter()
        .map(|s| (key_string(&s.key), s))
        .collect();
    folded_by_key.sort_by(|a, b| a.0.cmp(&b.0));
    let mut actual_by_key: Vec<(String, SessionSnapshot)> = actual
        .into_iter()
        .map(|s| (key_string(&s.key), s))
        .collect();
    actual_by_key.sort_by(|a, b| a.0.cmp(&b.0));

    for ((fk, fs), (ak, as_)) in folded_by_key.iter().zip(actual_by_key.iter()) {
        assert_eq!(fk, ak, "same key set");
        assert_eq!(fs, as_, "snapshot for {fk} must match the router's view");
    }

    // The web session's Created snapshot already carries its project dir.
    let web_created = got.iter().find_map(|e| match e {
        SessionEvent::Created { session } if session.key == web => Some(session.clone()),
        _ => None,
    });
    assert_eq!(
        web_created.map(|s| s.project_dir),
        Some(Some("/tmp/proj".into())),
        "Created must carry project_dir"
    );
}

/// The removed key must not linger: close drops the row from snapshots, and
/// the event stream is the only notification a subscriber needs.
#[tokio::test]
async fn removal_converges_snapshot_and_stream() {
    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let mut events = router.session_events();

    let keep = router.web_spawn("stays".into(), None).await;
    let gone = router.web_spawn("dies".into(), None).await;
    router.web_close_session(gone.clone()).await;

    let got = drain(&mut events).await;
    let folded = fold(&got);
    assert_eq!(folded.len(), 1, "only the surviving row remains: {folded:?}");
    assert_eq!(folded[0].key, keep);
    assert_eq!(
        router.session_snapshots().await.len(),
        1,
        "router state agrees"
    );
    assert!(!got.iter().any(
        |e| matches!(e, SessionEvent::Removed { key } if *key == keep)
    ));
}
