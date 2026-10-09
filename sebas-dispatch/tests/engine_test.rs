use sebas_acp::claude::session::AcpEvent;
use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::engine::{DispatchHandle, Out};
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
    // Restored state store rows → Dormant mapping; the first text must emit
    // SpawnResume (lazy respawn, openspec/specs/session-lifecycle/spec.md),
    // not SendAcp into the void.
    let map = SessionMap::restore_rows(
        vec![sebas_models::session_map::SessionMapRow {
            chat_id: "feishu".into(),
            thread_id: Some("oc_x".into()),
            session_id: "sess-old".into(),
            last_active_unix: 1,
            project_dir: None,
            acp_session_id: None,
            current_model: None,
            pending_kind: None,
            pending_model: None,
            pending_mode: None,
            desired_mode: sebas_dispatch::engine::ask_mode().as_str().to_string(),
            label: None,
            prompt_preview: None,
            awaiting_first_prompt: false,
        }],
        usize::MAX,
    );
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

// （fix-webui-qa-round2 1.4，D-C3b）投影契约：tool_result 之后到达的
// assistant 正文完整保留在转录里（顺序不乱、不被并入工具条目）。QA 观测
// 的「单工具审批回合丢环后正文」根因在夹具（fake-claude 的 perm 场景缺
// 环后正文帧，已补），本测试把投影半边的合同钉住：TextDelta 一条一段，
// 紧跟 ToolEnd 之后原序落账。
#[tokio::test]
async fn assistant_text_after_tool_result_lands_in_order() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = ChannelKey::new("web", "post-tool-text");
    router
        .map
        .insert(key.clone(), Mapping::active("s-posttool"))
        .await
        .unwrap();
    router.seed_card("s-posttool".to_string(), "perm".into()).await;

    let sid = "s-posttool";
    for evt in [
        AcpEvent::ToolStart {
            session_id: sid.into(),
            tool_name: "Bash".into(),
            args: serde_json::json!({"command": "rm -rf /"}),
            tool_use_id: None,
        },
        AcpEvent::ToolEnd {
            session_id: sid.into(),
            tool_name: "Bash".into(),
            result: "perm done\n".into(),
            tool_use_id: None,
        },
        AcpEvent::TextDelta {
            session_id: sid.into(),
            delta: "perm turn finished".into(),
        },
    ] {
        router.apply_event_to_out(sid.into(), &evt).await;
    }

    let turns = router.session_turns(&key, 0).await.unwrap();
    let kinds: Vec<&str> = turns
        .iter()
        .map(|t| match t.kind {
            sebas_domain::vocabulary::TurnKind::Prompt => "prompt",
            sebas_domain::vocabulary::TurnKind::Content => match t.element_type {
                sebas_domain::vocabulary::TurnElementType::Tool => "tool",
                _ => "markdown",
            },
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["prompt", "tool", "tool", "markdown"],
        "tool request, tool result, and the post-loop text must all land in order"
    );
    assert!(
        turns[3].content.contains("perm turn finished"),
        "the post-loop assistant text is preserved verbatim"
    );
}

// fold-tool-calls-into-process-tree 2.3：事件里的 `tool_use_id` 落进工具
// 转录条目（结果条目与配对调用同 id）；标题沿用既有规则——ToolStart 带
// args 出结构化标题，ToolEnd 无 args 退化为 `✓ {tool}`。
#[tokio::test]
async fn tool_entries_carry_tool_use_id_and_structured_titles() {
    let (router, _rx) = DispatchHandle::new(SessionMap::new());
    let key = ChannelKey::new("web", "tool-id");
    router
        .map
        .insert(key.clone(), Mapping::active("s-toolid"))
        .await
        .unwrap();
    router.seed_card("s-toolid".to_string(), "perm".into()).await;

    let sid = "s-toolid";
    for evt in [
        AcpEvent::ToolStart {
            session_id: sid.into(),
            tool_name: "Read".into(),
            args: serde_json::json!({"file_path": "src/main.rs"}),
            tool_use_id: Some("tc-1".into()),
        },
        AcpEvent::ToolEnd {
            session_id: sid.into(),
            tool_name: "Read".into(),
            result: "contents".into(),
            tool_use_id: Some("tc-1".into()),
        },
    ] {
        router.apply_event_to_out(sid.into(), &evt).await;
    }

    let turns = router.session_turns(&key, 0).await.unwrap();
    let tools: Vec<&sebas_dispatch::TurnEntry> = turns
        .iter()
        .filter(|t| t.element_type == sebas_domain::vocabulary::TurnElementType::Tool)
        .collect();
    assert_eq!(tools.len(), 2, "call + result both land as tool entries");
    // 调用条目：id + 结构化标题（偏好键序提取 file_path）。
    assert_eq!(tools[0].tool_use_id.as_deref(), Some("tc-1"));
    assert_eq!(tools[0].title.as_deref(), Some("Read · src/main.rs"));
    // 结果条目：同一 id；wire 无 args → 标题退化为 `✓ Read`。
    assert_eq!(tools[1].tool_use_id.as_deref(), Some("tc-1"));
    assert_eq!(tools[1].title.as_deref(), Some("✓ Read"));
}

// （fix-webui-qa-round2 2.2，D-B218）session-slash-commands「Command
// submission has a transcript receipt and one dispatch path」的后端半边：
// /compact 提交必须在转写里落一条 prompt 回执条目（此前只有 toast/进度卡、
// 转录零痕迹，命令产出因此与前一条 assistant 段落相邻合并成
// 「hello worldhello world」）。回执先于转发落账——前端按 prompt 条目开新
// 回合，命令产出自然独立成段。
#[tokio::test]
async fn compact_command_leaves_a_transcript_receipt_before_forwarding() {
    use sebas_acp::claude::session::AcpCommand;

    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    let key = ChannelKey::new("web", "compact-receipt");
    router
        .map
        .insert(key.clone(), Mapping::active("s-compact"))
        .await
        .unwrap();

    router
        .dispatch(ChannelEvent::Text {
            key: key.clone(),
            text: "/compact".into(),
            reply_target: None,
        })
        .await;

    // 转发半边（既有行为，不回归）：进度卡在前，ContinueSession 携带
    // /compact 原文。
    let card = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(card, Out::SendCard { .. }),
        "progress card is emitted first, got {card:?}"
    );
    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::SendAcp { session_id, cmd, .. } => {
            assert_eq!(session_id, "s-compact");
            match cmd {
                AcpCommand::ContinueSession { prompt, .. } => {
                    assert_eq!(prompt, "/compact", "the command text is forwarded verbatim");
                }
                other => panic!("expected ContinueSession, got {other:?}"),
            }
        }
        other => panic!("expected SendAcp, got {other:?}"),
    }

    // 回执半边（本轮新合同）：转写里恰好一条 prompt 条目、内容 = 命令原文
    // ——后续命令产出（TextDelta）落在这条 prompt 之后的独立回合。
    let turns = router.session_turns(&key, 0).await.unwrap();
    assert_eq!(turns.len(), 1, "the receipt is the only entry so far");
    assert_eq!(
        turns[0].kind,
        sebas_domain::vocabulary::TurnKind::Prompt,
        "the receipt is a prompt entry (opens the operator turn)"
    );
    assert_eq!(turns[0].content, "/compact", "receipt carries the verbatim command");
}
