use feishu::events::SessionKey;
use router::MsgIdMap;
use router::state::Mapping;
use router::state::QueuedTurn;
use router::state::SessionMap;

#[tokio::test]
async fn insert_and_lookup() {
    let m = SessionMap::new();
    let k = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    m.insert(k.clone(), Mapping::active("s1")).await.unwrap();
    let got = m.get(&k).await;
    assert_eq!(got.unwrap().session_id(), Some("s1"));
}

#[tokio::test]
async fn dump_and_restore_round_trip() {
    let m = SessionMap::new();
    let k = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    m.insert(k.clone(), Mapping::active("s1")).await.unwrap();

    let json = m.dump_json().await.unwrap();
    let m2 = SessionMap::restore_json(&json).unwrap();
    let got = m2.get(&k).await.unwrap();
    // Restored entries come back DORMANT (spec §3.3e): the child died with
    // the previous daemon, so the mapping must not route as live...
    assert_eq!(got.session_id(), None);
    assert!(matches!(
        got.state,
        router::state::MappingState::Dormant { .. }
    ));
    // ...but the id survives for lazy respawn, and persists again on dump.
    let json2 = m2.dump_json().await.unwrap();
    assert!(json2.contains("\"session_id\":\"s1\""));
}

#[tokio::test]
async fn dormant_first_text_claims_resume_then_queues() {
    let json = r#"{"oc_x":{"session_id":"s-old","last_active_unix":1}}"#;
    let m = SessionMap::restore_json(json).unwrap();
    let k = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    // First text: claims the dormant mapping for lazy respawn (spec §3.3e)
    // and swaps in a Spawning placeholder atomically.
    let r = m.route_text(k.clone(), "hello".into()).await.unwrap();
    assert!(matches!(r, router::state::TextRoute::Resume(ref old) if old == "s-old"));
    // A racing second text queues behind the placeholder — no double respawn.
    let r2 = m.route_text(k.clone(), "again".into()).await.unwrap();
    assert!(matches!(r2, router::state::TextRoute::Enqueued));
    // Activate flips to Active and drains the queue.
    let pending = m.activate(&k, "s-new".into()).await;
    assert_eq!(pending, vec!["again".to_string()]);
    let r3 = m.route_text(k.clone(), "third".into()).await.unwrap();
    assert!(matches!(r3, router::state::TextRoute::Continue(ref sid) if sid == "s-new"));
}

#[tokio::test]
async fn dormant_new_means_fresh_spawn_not_resume() {
    let json = r#"{"oc_x":{"session_id":"s-old","last_active_unix":1}}"#;
    let m = SessionMap::restore_json(json).unwrap();
    let k = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    let r = m.begin_spawn(k.clone()).await.unwrap();
    assert!(matches!(r, router::state::BeginSpawn::ReplacedActive));
    let got = m.get(&k).await.unwrap();
    assert!(matches!(
        got.state,
        router::state::MappingState::Spawning { .. }
    ));
}

#[tokio::test]
async fn route_text_and_activate_touch_last_active() {
    let m = SessionMap::new();
    let k = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    // Backdate the entry so the touch is observable.
    m.insert(k.clone(), Mapping::dormant("s1", 1))
        .await
        .unwrap();
    m.route_text(k.clone(), "hi".into()).await.unwrap();
    let t1 = m.get(&k).await.unwrap().last_active_unix;
    assert!(t1 > 1, "route_text must refresh last_active_unix, got {t1}");
    m.activate(&k, "s2".into()).await;
    let t2 = m.get(&k).await.unwrap().last_active_unix;
    assert!(t2 >= t1);
}

#[tokio::test]
async fn overflow_rejects() {
    let m = SessionMap::with_capacity(2);
    for i in 0..2 {
        m.insert(
            SessionKey {
                chat_id: format!("oc_{i}"),
                thread_id: None,
            },
            Mapping::active(format!("s_{i}")),
        )
        .await
        .unwrap();
    }
    let r = m
        .insert(
            SessionKey {
                chat_id: "oc_3".into(),
                thread_id: None,
            },
            Mapping::active("s_3"),
        )
        .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn msgid_map_record_overwrites_previous_entry() {
    let m = MsgIdMap::default();
    m.record("s1".into(), "om_first".into()).await;
    m.record("s1".into(), "om_second".into()).await;
    assert_eq!(m.get("s1").await.as_deref(), Some("om_second"));
}

#[tokio::test]
async fn queue_fifo_by_default_priority_jumps_front() {
    let m = SessionMap::new();
    let k = SessionKey {
        chat_id: "oc".into(),
        thread_id: None,
    };
    let _ = m.insert(k.clone(), Mapping::active("s1")).await;
    m.enqueue_turn(
        &k,
        QueuedTurn {
            prompt: "first".into(),
            reply_to: None,
            priority: false,
        },
    )
    .await;
    m.enqueue_turn(
        &k,
        QueuedTurn {
            prompt: "second".into(),
            reply_to: None,
            priority: false,
        },
    )
    .await;
    m.enqueue_turn(
        &k,
        QueuedTurn {
            prompt: "btw".into(),
            reply_to: None,
            priority: true,
        },
    )
    .await;
    assert_eq!(m.queue_len(&k).await, 3);
    assert_eq!(m.pop_next_turn(&k).await.unwrap().prompt, "btw"); // priority front
    assert_eq!(m.pop_next_turn(&k).await.unwrap().prompt, "first");
    assert_eq!(m.pop_next_turn(&k).await.unwrap().prompt, "second");
    assert!(m.pop_next_turn(&k).await.is_none());
}

#[tokio::test]
async fn pop_next_turn_cleans_up_empty_entry() {
    let m = SessionMap::new();
    let k = SessionKey {
        chat_id: "oc".into(),
        thread_id: None,
    };
    let _ = m.insert(k.clone(), Mapping::active("s1")).await;
    m.enqueue_turn(
        &k,
        QueuedTurn {
            prompt: "one".into(),
            reply_to: None,
            priority: false,
        },
    )
    .await;
    assert_eq!(m.queue_len(&k).await, 1);
    let popped = m.pop_next_turn(&k).await;
    assert!(popped.is_some());
    assert_eq!(m.queue_len(&k).await, 0);
}

#[tokio::test]
async fn remove_by_key_drops_mapping_and_queue() {
    let m = SessionMap::new();
    let k = SessionKey {
        chat_id: "oc_orphan".into(),
        thread_id: None,
    };
    m.insert(k.clone(), Mapping::spawning()).await.unwrap();
    m.enqueue_turn(
        &k,
        QueuedTurn {
            prompt: "queued".into(),
            reply_to: None,
            priority: false,
        },
    )
    .await;
    assert_eq!(m.queue_len(&k).await, 1);

    m.remove_by_key(&k).await;
    assert!(m.get(&k).await.is_none(), "mapping must be gone");
    assert_eq!(
        m.queue_len(&k).await,
        0,
        "queue must drop alongside the mapping (no zombie prompts)"
    );

    // Idempotent: removing again is a no-op.
    m.remove_by_key(&k).await;
}

#[tokio::test]
async fn msgid_drop_removes_entry() {
    let m = MsgIdMap::default();
    m.record("s1".into(), "om_a".into()).await;
    m.record("s2".into(), "om_b".into()).await;
    m.drop("s1").await;
    assert!(m.get("s1").await.is_none());
    assert_eq!(m.get("s2").await.as_deref(), Some("om_b"));
}
