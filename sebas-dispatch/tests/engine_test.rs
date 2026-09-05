use sebas_acp::claude::session::AcpEvent;
use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::engine::{Out, DispatchHandle};
use sebas_dispatch::state::{Mapping, SessionMap};
use std::time::Duration;

#[tokio::test]
async fn new_text_creates_session_and_emits_initial_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    let key = ChannelKey::feishu("oc_x", None);

    tokio::spawn(async move {
        let _ = router
            .dispatch(ChannelEvent::Text {
                key: key.clone(),
                text: "hello".into(),
                reply_target: None,
            })
            .await;
    });

    let first = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    // First event is some "send_card" or "spawn acp" — we assert shape loosely:
    assert!(matches!(first, Out::SendCard { .. } | Out::SpawnAcp { .. }));
}

#[tokio::test]
async fn existing_session_dispatches_continue() {
    let map = SessionMap::new();
    let k = ChannelKey::feishu("oc_x", None);
    map.insert(k.clone(), Mapping::active("existing"))
        .await
        .unwrap();

    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    tokio::spawn(async move {
        let _ = router
            .dispatch(ChannelEvent::Text {
                key: k.clone(),
                text: "more".into(),
                reply_target: None,
            })
            .await;
    });

    // Per-turn flow: a fresh card is posted first, then the prompt is
    // forwarded to the session.
    let card = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(card, Out::SendCard { .. }),
        "expected per-turn SendCard, got {card:?}"
    );
    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::SendAcp { session_id, .. } => assert_eq!(session_id, "existing"),
        other => panic!("expected SendAcp, got {other:?}"),
    }
}

#[tokio::test]
async fn dormant_mapping_emits_spawn_resume() {
    // Restored state file → Dormant mapping; the first text must emit
    // SpawnResume (lazy respawn, openspec/specs/session-lifecycle/spec.md),
    // not SendAcp into the void.
    let json = r#"{"oc_x":{"session_id":"sess-old","last_active_unix":1}}"#;
    let map = SessionMap::restore_json(json).unwrap();
    let k = ChannelKey::feishu("oc_x", None);

    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    tokio::spawn(async move {
        let _ = router
            .dispatch(ChannelEvent::Text {
                key: k.clone(),
                text: "继续".into(),
                reply_target: None,
            })
            .await;
    });

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::SpawnResume {
            session_id, prompt, ..
        } => {
            assert_eq!(session_id, "sess-old");
            assert_eq!(prompt, "继续");
        }
        other => panic!("expected SpawnResume, got {other:?}"),
    }
}

#[tokio::test]
async fn apply_event_to_out_renders_update_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

    let evt = AcpEvent::TextDelta {
        session_id: "s1".into(),
        delta: "hi".into(),
    };
    router.apply_event_to_out("s1".into(), &evt).await;

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::UpdateCard { session_id, card } => {
            assert_eq!(session_id, "s1");
            assert!(!card.elements.is_empty());
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
}
