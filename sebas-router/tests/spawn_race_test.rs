//! D8: two texts racing a slow spawn must yield exactly one SpawnAcp; the
//! second is queued and drained by activate(). Dump keeps the MappingDto shape (structured ChannelKey keys now).

use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::{Mapping, MappingState, SessionMap, TextRoute};
use std::time::Duration;

fn key() -> ChannelKey { ChannelKey::feishu("oc_race", None) }

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

/// web 通道 key（0-turn 占位会话走 web 通道）。
fn web_key(id: &str) -> ChannelKey {
    ChannelKey::new("web", format!("web-{id}"))
}

/// P2: 0-turn 占位（`begin_spawn_with` 记住 kind/model）首条消息应触发
/// `Out::WebSpawn` 且携带 pending kind/model —— 不发空 prompt、不排队。
#[tokio::test]
async fn placeholder_first_message_spawns_with_pending_kind_and_model() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    let key = web_key("zero-turn");

    let outcome = map
        .begin_spawn_with(key.clone(), Some("opencode".into()), Some("m-free".into()))
        .await
        .unwrap();
    assert!(matches!(outcome, sebas_router::state::BeginSpawn::Fresh));

    // First message: route_text must yield SpawnNew (not Enqueued) and the
    // router must emit Out::WebSpawn with the remembered kind/model.
    let route = map.route_text(key.clone(), "hello".into()).await.unwrap();
    assert!(
        matches!(route, TextRoute::SpawnNew),
        "placeholder first message must spawn, not enqueue"
    );

    router
        .web_send_message(key.clone(), "hello".into())
        .await;
    let out = tokio::time::timeout(Duration::from_millis(300), out_rx.recv())
        .await
        .expect("WebSpawn must be emitted")
        .expect("channel open");
    match out {
        Out::WebSpawn {
            key: k,
            prompt,
            project_dir,
            kind,
            model,
        } => {
            assert_eq!(k, key);
            assert_eq!(prompt, "hello");
            assert_eq!(project_dir, None);
            assert_eq!(kind.as_deref(), Some("opencode"));
            assert_eq!(model.as_deref(), Some("m-free"));
        }
        other => panic!("expected Out::WebSpawn, got {other:?}"),
    }
}

/// P2: 0-turn 占位替换已激活会话时保留 kind/model，首条消息仍触发 spawn。
#[tokio::test]
async fn placeholder_replaces_active_and_keeps_pending_kind() {
    let map = SessionMap::new();
    let key = web_key("replace");
    map.insert(key.clone(), Mapping::active("sid-old"))
        .await
        .unwrap();
    let (router, _out_rx) = RouterHandle::new(map.clone());

    let outcome = map
        .begin_spawn_with(key.clone(), Some("opencode".into()), None)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        sebas_router::state::BeginSpawn::ReplacedActive
    ));

    let mut m = map.get(&key).await.unwrap();
    assert_eq!(m.pending_kind.as_deref(), Some("opencode"));
    // 读回后 clearing: route_text 消费 placeholder 时翻转为普通 Spawning。
    m.state = MappingState::Spawning { pending: Vec::new() };
    map.insert(key.clone(), m).await.unwrap();
    let route = map.route_text(key.clone(), "fresh".into()).await.unwrap();
    assert!(
        matches!(route, TextRoute::SpawnNew),
        "replaced placeholder must spawn on first message, not enqueue"
    );
    let _ = router;
}
