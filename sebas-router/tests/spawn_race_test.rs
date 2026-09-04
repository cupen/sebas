//! D8: two texts racing a slow spawn must yield exactly one SpawnAcp; the
//! second is queued and drained by activate(). Dump keeps the MappingDto shape (structured ChannelKey keys now).

use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::{Mapping, MappingState, SessionMap, TextRoute};
use std::time::Duration;

fn key() -> ChannelKey { ChannelKey::feishu("oc_race".into(), None) }

#[tokio::test]
async fn second_text_during_spawn_is_queued_not_spawned() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

    router
        .dispatch(ChannelEvent::Text { key: key(), text: "msg1".into(), reply_target: None })
        .await;
    let first = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(first, Out::SpawnAcp { .. }), "got {first:?}");

    // Spawn still in flight (nobody called activate): the second text queues.
    router
        .dispatch(ChannelEvent::Text { key: key(), text: "msg2".into(), reply_target: None })
        .await;
    let second = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
    assert!(
        second.is_err(),
        "no second Out may be emitted while spawning"
    );

    let pending = map.activate(&key(), "s1".into(), None, None).await;
    assert_eq!(pending, vec!["msg2".to_string()]);
    // Now active: a third text continues the session.
    router
        .dispatch(ChannelEvent::Text { key: key(), text: "msg3".into(), reply_target: None })
        .await;
    // Per-turn flow: a fresh card is posted first, then the prompt is
    // forwarded to the session.
    let third_card = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(third_card, Out::SendCard { .. }),
        "expected per-turn SendCard, got {third_card:?}"
    );
    let third = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match third {
        Out::SendAcp { session_id, .. } => assert_eq!(session_id, "s1"),
        other => panic!("expected SendAcp, got {other:?}"),
    }
}

#[tokio::test]
async fn fail_spawn_removes_placeholder() {
    let map = SessionMap::new();
    map.route_text(key(), "m".into()).await.expect("route_text");
    map.fail_spawn(&key()).await;
    assert!(map.get(&key()).await.is_none());
}

#[tokio::test]
async fn pending_queue_capped_at_16() {
    let map = SessionMap::new();
    map.route_text(key(), "m0".into()).await.unwrap();
    for i in 1..20 {
        let r = map.route_text(key(), format!("m{i}")).await.unwrap();
        assert!(matches!(r, TextRoute::Enqueued));
    }
    let pending = map.activate(&key(), "s1".into(), None, None).await;
    assert_eq!(pending.len(), 16);
    assert_eq!(pending[0], "m1");
    assert_eq!(pending[15], "m16");
}

#[tokio::test]
async fn dump_filters_spawning_and_persists_mapping_dto() {
    let map = SessionMap::new();
    map.route_text(key(), "m".into()).await.unwrap();
    let active_key = ChannelKey::feishu("oc_active", None);
    map.insert(active_key.clone(), Mapping::active("s9"))
        .await
        .unwrap();

    let json = map.dump_json().await.unwrap();
    assert!(!json.contains("oc_race"), "spawning entry leaked into dump");
    assert!(json.contains("oc_active"));
    // MappingDto shape unchanged: {"session_id": ..., "last_active_unix": ...}.
    assert!(json.contains("\"session_id\":\"s9\""));
    assert!(!json.contains("Spawning") && !json.contains("spawning"));

    // Round-trip through restore: entries come back Dormant
    // (openspec/specs/session-lifecycle/spec.md) —
    // dead child, eligible for lazy respawn, not live routing.
    let restored = SessionMap::restore_json(&json).unwrap();
    let m = restored.get(&active_key).await.expect("restored");
    assert!(matches!(m.state, MappingState::Dormant { .. }));
    assert_eq!(m.session_id(), None);
}

#[tokio::test]
async fn rapid_double_new_emits_single_spawn() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

    for _ in 0..2 {
        router
            .dispatch(ChannelEvent::Text { key: key(), text: "/new".into(), reply_target: None })
            .await;
    }

    let first = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(first, Out::SpawnAcp { .. }), "got {first:?}");
    let second = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
    assert!(
        second.is_err(),
        "duplicate /new must not emit a second SpawnAcp"
    );
}
