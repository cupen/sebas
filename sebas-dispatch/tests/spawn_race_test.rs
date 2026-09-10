//! D8: two texts racing a slow spawn must yield exactly one SpawnAcp; the
//! second is queued and drained by activate(). Dump keeps the MappingDto shape (structured ChannelKey keys now).

use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::engine::{Out, DispatchHandle};
use sebas_dispatch::state::{Mapping, MappingState, SessionMap, TextRoute};
use std::time::Duration;

fn key() -> ChannelKey { ChannelKey::feishu("oc_race", None) }

#[tokio::test]
async fn second_text_during_spawn_is_queued_not_spawned() {
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

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
async fn fail_spawn_keeps_session_visible_as_spawn_failed() {
    // fail-fast-on-startup-errors 3.1：占位不再被移除，而是转为 spawn-failed
    // 终态——会话保持可见、带失败原因，transcript 经合成 id 可读。
    let map = SessionMap::new();
    map.route_text(key(), "m".into()).await.expect("route_text");
    let transcript_id = map
        .fail_spawn(&key(), "agent binary missing")
        .await
        .expect("spawning placeholder must transition");
    let m = map.get(&key()).await.expect("mapping kept");
    assert!(m.session_id().is_none(), "spawn-failed 不是 live 会话");
    assert_eq!(
        m.spawn_failed_reason(),
        Some("agent binary missing"),
        "失败原因随映射保留"
    );
    // transcript_id 与映射内合成 id 一致（engine 用它挂错误条目）。
    match &m.state {
        sebas_dispatch::state::MappingState::SpawnFailed { session_id, .. } => {
            assert_eq!(*session_id, transcript_id);
        }
        other => panic!("expected SpawnFailed, got {other:?}"),
    }
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
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

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
///
/// workbench-agent-wire-fix D1：占位身份是显式 `awaiting_first_prompt`
/// 标记，被 route_text 的 SpawnNew **一次性消费**（消费后到达的文本在
/// spawn-in-flight 窗口内排队，绝不二次 spawn）。因此本测试分两相：
/// 相 1 验证路由判据；相 2 用全新占位走生产路径（`web_send_message`
/// 内部恰好 route_text 一次）验证 WebSpawn 携带 kind/model。
#[tokio::test]
async fn placeholder_first_message_spawns_with_pending_kind_and_model() {
    let map = SessionMap::new();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    let key = web_key("zero-turn");

    let outcome = map
        .begin_spawn_with(key.clone(), Some("opencode".into()), Some("m-free".into()))
        .await
        .unwrap();
    assert!(matches!(outcome, sebas_dispatch::state::BeginSpawn::Fresh));

    // 相 1：路由判据 —— 占位首条消息必须 SpawnNew（不排队），且标记被
    // 消费（spawn-in-flight 窗口内的第二条消息改走 Enqueued）。
    let route = map.route_text(key.clone(), "hello".into()).await.unwrap();
    assert!(
        matches!(route, TextRoute::SpawnNew),
        "placeholder first message must spawn, not enqueue"
    );
    let second = map.route_text(key.clone(), "race".into()).await.unwrap();
    assert!(
        matches!(second, TextRoute::Enqueued),
        "message racing the spawn must queue, not double-spawn"
    );

    // 相 2：生产发射路径 —— 全新占位 + web_send_message（内部一次
    // route_text）必须发出 WebSpawn 且携带记住的 kind/model。
    let key2 = web_key("zero-turn-emit");
    map.begin_spawn_with(key2.clone(), Some("opencode".into()), Some("m-free".into()))
        .await
        .unwrap();
    router.web_send_message(key2.clone(), "hello".into()).await;
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
            assert_eq!(k, key2);
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
    let (router, _out_rx) = DispatchHandle::new(map.clone());

    let outcome = map
        .begin_spawn_with(key.clone(), Some("opencode".into()), None)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        sebas_dispatch::state::BeginSpawn::ReplacedActive
    ));

    let mut m = map.get(&key).await.unwrap();
    assert_eq!(m.pending_kind.as_deref(), Some("opencode"));
    // 读回后 clearing: route_text 消费 placeholder 时翻转为普通 Spawning。
    m.state = MappingState::Spawning {
        pending: Vec::new(),
        awaiting_first_prompt: true,
    };
    map.insert(key.clone(), m).await.unwrap();
    let route = map.route_text(key.clone(), "fresh".into()).await.unwrap();
    assert!(
        matches!(route, TextRoute::SpawnNew),
        "replaced placeholder must spawn on first message, not enqueue"
    );
    let _ = router;
}

/// fail-fast-on-startup-errors 3.1：spawn 失败即时内显——会话保留为
/// spawn-failed（不再 Removed），transcript 出现带原因的 error 事件，
/// 事件流发布的是 Updated 而非 Removed。
#[tokio::test]
async fn fail_spawn_surfaces_error_inline_and_keeps_session_visible() {
    let map = SessionMap::new();
    let (router, _out_rx) = DispatchHandle::new(map.clone());

    // webui 建会话：占位插入（Created）。
    let key = router.web_spawn("hello".into(), None, None, None).await;

    // spawn 失败：dispatch 以失败原因回调。
    router.fail_spawn(&key, "agent binary missing").await;

    // 会话仍在列表里，状态诚实标记 spawn-failed。
    let infos = router.session_info_snapshot().await;
    let info = infos.iter().find(|i| i.key == key.reference).expect("session kept");
    assert_eq!(info.status, "spawn-failed");
    assert!(info.session_id.is_none());

    // transcript 经合成 id 可读：一条带原因的 error 事件。
    let turns = router.session_turns(&key, 0).await.expect("turns readable");
    let errors: Vec<_> = turns.iter().filter(|e| e.kind == "error").collect();
    assert_eq!(errors.len(), 1, "exactly one error entry: {turns:?}");
    assert!(errors[0].element_type == "error");
    assert!(
        errors[0].content.contains("spawn failed") && errors[0].content.contains("agent binary missing"),
        "error carries the reason: {}",
        errors[0].content
    );

    // 事件流的首次呈现是 Updated（会话保持存在），不是 Removed。
    let mut events = router.subscribe_session_events();
    router.fail_spawn(&key, "second failure must be a no-op").await;
    let mut saw_removed = false;
    while let Ok(ev) = events.try_recv() {
        if matches!(ev, sebas_dispatch::SessionEvent::Removed { .. }) {
            saw_removed = true;
        }
    }
    assert!(!saw_removed, "spawn failure must not publish Removed");
}

/// workbench-agent-wire-fix D1：占位标记（与 kind/model 解耦）——
/// kind/model 全空的 0-turn 占位，首条消息也必须 SpawnNew。
/// 这是「输入框发不出消息」根因的回归锁：旧判据
/// `pending_kind.is_some() || pending_model.is_some()` 对默认 agent 的
/// 占位恒为 false，首条消息被误排队且无人 drain。
#[tokio::test]
async fn placeholder_without_kind_or_model_still_spawns_on_first_message() {
    let map = SessionMap::new();
    let key = web_key("bare-placeholder");

    // 占位创建：kind/model 均为 None（默认 agent 路径，rail「+」同款）。
    map.begin_spawn_with(key.clone(), None, None).await.unwrap();

    let route = map.route_text(key.clone(), "hello".into()).await.unwrap();
    assert!(
        matches!(route, TextRoute::SpawnNew),
        "placeholder identity is the explicit marker, not the optional kind/model fields"
    );
}

/// workbench-agent-wire-fix spec「placeholder marker survives restart」：
/// 0-turn 占位落盘后重启（restore），仍以占位身份出现，首条消息照常
/// SpawnNew；普通 spawn-in-flight 条目依旧不入盘。
#[tokio::test]
async fn placeholder_marker_survives_dump_restore_round_trip() {
    let map = SessionMap::new();
    let ph = web_key("ph-roundtrip");
    let in_flight = web_key("inflight-roundtrip");
    let active_key = ChannelKey::feishu("oc_rt", None);

    // 占位（带 kind/model/project_dir 的完整形状）。
    map.begin_spawn_with(ph.clone(), Some("codex".into()), Some("m1".into()))
        .await
        .unwrap();
    map.set_project_dir(&ph, Some("/tmp/wf".into())).await;
    // 普通 spawn-in-flight（首条消息已触发 spawn）——不入盘。
    map.route_text(in_flight.clone(), "go".into()).await.unwrap();
    // Active 对照组。
    map.insert(active_key.clone(), Mapping::active("s-rt"))
        .await
        .unwrap();

    let json = map.dump_json().await.unwrap();
    assert!(
        json.contains("ph-roundtrip") && !json.contains("inflight-roundtrip"),
        "placeholder persists; in-flight spawn stays filtered"
    );

    let restored = SessionMap::restore_json(&json).unwrap();
    let m = restored.get(&ph).await.expect("placeholder restored");
    match &m.state {
        MappingState::Spawning { awaiting_first_prompt, .. } => {
            assert!(awaiting_first_prompt, "placeholder identity survives restart");
        }
        other => panic!("expected Spawning placeholder, got {other:?}"),
    }
    assert_eq!(m.pending_kind.as_deref(), Some("codex"));
    assert_eq!(m.pending_model.as_deref(), Some("m1"));
    assert_eq!(m.project_dir.as_deref(), Some("/tmp/wf"));

    // 重启后的首条消息照常触发 spawn。
    let route = restored.route_text(ph.clone(), "hello".into()).await.unwrap();
    assert!(matches!(route, TextRoute::SpawnNew));

    // 对照组：active 条目照常回 Dormant；in-flight 条目不存在。
    let a = restored.get(&active_key).await.expect("active restored");
    assert!(matches!(a.state, MappingState::Dormant { .. }));
    assert!(restored.get(&in_flight).await.is_none(), "in-flight spawn is not persisted");
}
