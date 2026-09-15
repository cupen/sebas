//! Session event stream + external snapshot (openspec/changes/add-core-session-channel
//! tasks 1.2/1.3): subscribe → create → status change → remove yields the exact
//! event sequence, and applying events to a snapshot reproduces the router's own
//! state.

use sebas_channels::ChannelKey;
use sebas_dispatch::DispatchHandle;
use sebas_dispatch::engine::SessionEvent;
use sebas_dispatch::state::{Mapping, SessionMap};
use std::collections::HashMap;

fn key(id: &str) -> ChannelKey {
    ChannelKey::feishu(&format!("oc_{id}"), None)
}

/// Task 1.2: create → status change → remove publishes the exact sequence.
#[tokio::test]
async fn events_follow_create_status_change_remove() {
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let mut events = router.subscribe_session_events();

    // create: web_spawn inserts a Spawning placeholder.
    let key = router
        .web_spawn("hello world".into(), Some("/tmp/p".into()), None, None, None)
        .await;
    // status change: Spawning → Active.
    router.activate(&key, "s1".into(), None, None).await;
    // remove.
    let outcome = router.web_close_session(key.clone()).await;
    assert_eq!(
        outcome,
        sebas_dispatch::engine::CloseOutcome::Closed {
            discarded_pending: 0
        }
    );

    let mut seq = Vec::new();
    while let Ok(ev) = events.try_recv() {
        seq.push(ev);
    }

    assert_eq!(
        seq.len(),
        3,
        "expected exactly [Created, Updated, Removed], got {seq:?}"
    );
    match &seq[0] {
        SessionEvent::Created { session } => {
            assert_eq!(session.channel, key.channel_str());
            assert_eq!(session.key, key.reference);
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
        SessionEvent::Removed { channel, key: k } => {
            assert_eq!(channel, &key.channel_str().to_string());
            assert_eq!(k, &key.reference);
        }
        other => panic!("third event should be Removed, got {other:?}"),
    }
}

/// Task 1.2: emoji phase transition publishes Updated with the new phase.
#[tokio::test]
async fn phase_transition_publishes_updated_with_phase() {
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
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
        .apply_event(
            "s-b",
            &AcpEvent::TextDelta {
                session_id: "s-b".into(),
                delta: "working on it".into(),
            },
        )
        .await;

    let mut saw_working = false;
    while let Ok(ev) = events.try_recv() {
        if let SessionEvent::Updated { session } = ev
            && session.session_id.as_deref() == Some("s-b")
            && session.phase.as_deref() == Some(sebas_dispatch::card_state::phase::WORKING)
        {
            saw_working = true;
        }
    }
    assert!(
        saw_working,
        "expected an Updated event with the WORKING phase"
    );
}

/// Task 1.3: applying published events to a snapshot reproduces the router's
/// own state.
#[tokio::test]
async fn applying_events_to_snapshot_reproduces_router_state() {
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let mut events = router.subscribe_session_events();

    // Client-side cache: (channel, key) → SessionInfo.
    let mut cache: HashMap<(String, String), sebas_dispatch::SessionInfo> = HashMap::new();

    let ka = key("a");
    router
        .map
        .insert(ka.clone(), Mapping::dormant("s-a", 42))
        .await
        .unwrap();
    let kb_key = router.web_spawn("spawn me".into(), None, None, None, None).await;
    router.activate(&kb_key, "s-b".into(), None, None).await;
    let _ = router.web_close_session(ka).await;

    // Fold: snapshot BEFORE the mutations? No — take the snapshot now and fold
    // only the buffered events on top of an empty cache; the result must equal
    // the router's own snapshot.
    let snapshot = router.session_info_snapshot().await;
    while let Ok(ev) = events.try_recv() {
        match ev {
            SessionEvent::Created { session } | SessionEvent::Updated { session } => {
                cache.insert((session.channel.clone(), session.key.clone()), session);
            }
            SessionEvent::Removed { channel, key } => {
                cache.remove(&(channel, key));
            }
            // workbench-turn-queue 5.2：丢弃标注事件不携带可折叠的全量状态，
            // 折叠测试对它无操作（会话本身随 Removed 离开缓存）。
            SessionEvent::PendingDropped { .. } => {}
            SessionEvent::Resync => {}
        }
    }

    assert_eq!(
        cache.len(),
        snapshot.len(),
        "cache {cache:?} vs snapshot {snapshot:?}"
    );
    for info in &snapshot {
        let cached = cache
            .get(&(info.channel.clone(), info.key.clone()))
            .expect("snapshot session present in cache");
        assert_eq!(cached, info);
    }
    // Exactly the surviving web session remains, active (web_spawn minted its
    // own `web-{nanos}` key — kb was never given a mapping).
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].channel, kb_key.channel_str());
    assert_eq!(snapshot[0].key, kb_key.reference);
    assert_eq!(snapshot[0].status, "active");
    assert_eq!(snapshot[0].session_id.as_deref(), Some("s-b"));
}

/// Task 1.3 companion: transcript positions are monotonic and `session_turns`
/// returns only entries at or after the requested position.
#[tokio::test]
async fn turns_are_incremental_by_position() {
    use sebas_acp::claude::session::AcpEvent;
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
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

/// rail-declutter-unread 1.1：`SessionInfo.msg_count` 按可见回复段口径投影
/// ——相邻 markdown delta 合并成段；thinking/tool/prompt 不计数；被打断后
/// 的正文另起一段。徽标口径（段）与 seam 口径（轮）在此分别钉住。
#[tokio::test]
async fn session_info_projects_visible_reply_segment_count() {
    use sebas_acp::claude::session::AcpEvent;
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let k = key("cnt");
    router
        .map
        .insert(k.clone(), Mapping::active("s-cnt"))
        .await
        .unwrap();

    let count_of = || async {
        router
            .session_info_for(&k)
            .await
            .expect("mapping exists")
            .msg_count
    };

    // 尚无内容：0。
    assert_eq!(count_of().await, 0);

    // 一段流式回复：3 个 delta = 1 段。
    let delta = |d: &str| AcpEvent::TextDelta {
        session_id: "s-cnt".into(),
        delta: d.into(),
    };
    router.seed_card("s-cnt".into(), "do it".into()).await;
    router.apply_event("s-cnt", &delta("one ")).await;
    router.apply_event("s-cnt", &delta("two ")).await;
    router.apply_event("s-cnt", &delta("three.")).await;
    assert_eq!(count_of().await, 1, "adjacent deltas merge into one segment");

    // 工具噪声不计数。
    router
        .apply_event(
            "s-cnt",
            &AcpEvent::ToolStart {
                session_id: "s-cnt".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({ "path": "/tmp/x" }),
            },
        )
        .await;
    assert_eq!(count_of().await, 1, "tool calls must not increment");

    // thinking 不计数。
    router
        .apply_event(
            "s-cnt",
            &AcpEvent::ThinkingDelta {
                session_id: "s-cnt".into(),
                delta: "hmm".into(),
            },
        )
        .await;
    assert_eq!(count_of().await, 1, "thinking must not increment");

    // 工具/thinking 打断后的正文另起一段。
    router.apply_event("s-cnt", &delta("and done.")).await;
    assert_eq!(count_of().await, 2, "a broken run opens a new segment");
}

/// workbench-conversation-view 1.3（design D2）：ToolStart/ToolEnd 两条 push
/// 点写 `element_type = "tool"`（kind 仍是 agent 侧 content），turn-content
/// 检索结果里工具与正文可区分——不再靠内容前缀当契约。
#[tokio::test]
async fn tool_events_are_labelled_tool_in_turn_content() {
    use sebas_acp::claude::session::AcpEvent;
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let k = key("tool");
    router
        .map
        .insert(k.clone(), Mapping::active("s-tool"))
        .await
        .unwrap();
    router.seed_card("s-tool".into(), "use a tool".into()).await;

    router
        .apply_event(
            "s-tool",
            &AcpEvent::TextDelta {
                session_id: "s-tool".into(),
                delta: "let me check.".into(),
            },
        )
        .await;
    router
        .apply_event(
            "s-tool",
            &AcpEvent::ToolStart {
                session_id: "s-tool".into(),
                tool_name: "read_file".into(),
                args: serde_json::json!({ "path": "/tmp/x" }),
            },
        )
        .await;
    router
        .apply_event(
            "s-tool",
            &AcpEvent::ToolEnd {
                session_id: "s-tool".into(),
                tool_name: "read_file".into(),
                result: "file body".into(),
            },
        )
        .await;
    router
        .apply_event(
            "s-tool",
            &AcpEvent::TextDelta {
                session_id: "s-tool".into(),
                delta: "done.".into(),
            },
        )
        .await;

    let turns = router.session_turns(&k, 0).await.unwrap();
    // prompt + text + ToolStart + ToolEnd + text
    assert_eq!(turns.len(), 5);
    let tools: Vec<_> = turns.iter().filter(|e| e.element_type == "tool").collect();
    assert_eq!(
        tools.len(),
        2,
        "ToolStart and ToolEnd both label tool: {turns:?}"
    );
    for t in &tools {
        assert_eq!(t.kind, "content", "tool entries are agent-side content");
        assert!(
            t.content.contains("read_file"),
            "tool content stays readable markdown: {}",
            t.content
        );
    }
    let prose: Vec<_> = turns
        .iter()
        .filter(|e| e.element_type == "markdown" && e.kind == "content")
        .collect();
    assert_eq!(prose.len(), 2, "prose stays markdown: {turns:?}");
    // Tool entries are distinguishable from prose without sniffing content.
    assert_ne!(tools[0].position, prose[0].position);
}

/// session-slash-commands 2.1：`AvailableCommands` 事件物化进会话快照
/// （apply_event 写映射 → session_info_for 暴露 `available_commands`）；二次
/// 通知（重新广告）**覆盖**旧表而非追加；空表同样覆盖（撤回广告如实呈现）。
#[tokio::test]
async fn available_commands_materializes_into_snapshot_and_reread_overwrites() {
    use sebas_acp::claude::session::AcpEvent;
    use sebas_acp::AvailableCommand;

    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let k = key("slash");
    router
        .map
        .insert(k.clone(), Mapping::active("s-slash"))
        .await
        .unwrap();

    let cmd = |name: &str, hint: Option<&str>| AvailableCommand {
        name: name.into(),
        description: format!("desc of {name}"),
        hint: hint.map(str::to_string),
    };
    let commands_of = || async {
        router
            .session_info_for(&k)
            .await
            .expect("mapping exists")
            .available_commands
    };

    // 尚未广告：空表（无命令面板的诚实退化形态）。
    assert!(commands_of().await.is_empty());

    // 第一次广告：快照出现命令表。
    router
        .apply_event(
            "s-slash",
            &AcpEvent::AvailableCommands {
                session_id: "s-slash".into(),
                commands: vec![cmd("goal", Some("<condition>"))],
            },
        )
        .await;
    let first = commands_of().await;
    assert_eq!(first.len(), 1, "advertised table lands in the snapshot");
    assert_eq!(first[0].name, "goal");
    assert_eq!(first[0].hint.as_deref(), Some("<condition>"));

    // 二次通知：全量覆盖旧表（不是追加）。
    router
        .apply_event(
            "s-slash",
            &AcpEvent::AvailableCommands {
                session_id: "s-slash".into(),
                commands: vec![cmd("compact", None), cmd("review", None)],
            },
        )
        .await;
    let second = commands_of().await;
    assert_eq!(
        second.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["compact", "review"],
        "re-advertisement must OVERWRITE the old table, got {second:?}"
    );

    // 撤回广告（空表）：同样覆盖——快照如实回到「无命令面板」。
    router
        .apply_event(
            "s-slash",
            &AcpEvent::AvailableCommands {
                session_id: "s-slash".into(),
                commands: Vec::new(),
            },
        )
        .await;
    assert!(commands_of().await.is_empty(), "empty table overwrites too");

    // 快照序列化：空表时键不上 wire（旧消费端兼容）。
    let info = router.session_info_for(&k).await.unwrap();
    let json = serde_json::to_string(&info).unwrap();
    assert!(!json.contains("available_commands"), "{json}");
}

/// session-slash-commands 2.2（webui 透出的核心断言面在 SessionInfo 本身，
/// 这里钉住 Updated 事件携带新表——WS `session.updated` 的载荷数据源）。
#[tokio::test]
async fn available_commands_rides_the_updated_event() {
    use sebas_acp::claude::session::AcpEvent;

    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let mut events = router.subscribe_session_events();
    let k = key("slash-ev");
    router
        .map
        .insert(k.clone(), Mapping::active("s-slash-ev"))
        .await
        .unwrap();

    // 即时路径（dispatch_acp_event → apply_event_to_out 的 `_` 臂）驱动
    // 物化：与 pump 路径（上一用例的 apply_event）殊途同归。
    router
        .dispatch_acp_event(AcpEvent::AvailableCommands {
            session_id: "s-slash-ev".into(),
            commands: vec![
                sebas_acp::AvailableCommand {
                    name: "goal".into(),
                    description: "Set a goal".into(),
                    hint: Some("<condition>".into()),
                },
                sebas_acp::AvailableCommand {
                    name: "compact".into(),
                    description: "Clear context".into(),
                    hint: None,
                },
            ],
        })
        .await;

    let mut saw_updated_with_commands = false;
    while let Ok(ev) = events.try_recv() {
        if let sebas_dispatch::engine::SessionEvent::Updated { session } = ev
            && session.available_commands.len() == 2
            && session.available_commands[0].name == "goal"
        {
            saw_updated_with_commands = true;
        }
    }
    assert!(
        saw_updated_with_commands,
        "an Updated event must carry the refreshed command table"
    );
}
