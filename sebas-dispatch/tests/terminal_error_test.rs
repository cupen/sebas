//! Terminal AcpEvent::Error: the router must remove the session mapping and
//! emit an ❌ UpdateCard. Non-terminal errors keep the existing behaviour.

use sebas_acp::claude::session::AcpEvent;
use sebas_channels::ChannelKey;
use sebas_dispatch::cards::CardConfig;
use sebas_dispatch::engine::{DispatchHandle, Out};
use sebas_dispatch::state::{Mapping, SessionMap};
use std::time::Duration;

/// 收干一小段时间窗内的全部 Out（p3g 起事件可能连带 Out::React，
/// 不能再假设一个事件只产一个 Out）。
async fn drain(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Vec<Out> {
    let mut out = vec![];
    while let Ok(Some(o)) = tokio::time::timeout(Duration::from_millis(60), rx.recv()).await {
        out.push(o);
    }
    out
}

#[tokio::test]
async fn terminal_error_retires_binding_and_marks_card() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s1".into(),
            message: "agent process exited".into(),
            terminal: true,
        })
        .await;

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::UpdateCard { session_id, card } => {
            assert_eq!(session_id, "s1");
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains('❌'), "expected ❌ in terminal card: {s}");
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
    // （fix-webui-qa-defects-round4 1.2）teardown 只清活跃绑定：映射以
    // Dormant 记录形态保留（列表行不消失、转录可寻址），不再是 Active。
    let m = map
        .get(&key)
        .await
        .expect("terminal error must keep the session record");
    assert!(
        m.session_id().is_none() && m.transcript_id() == Some("s1"),
        "live binding must be gone but the record (Dormant) must remain: {m:?}"
    );
    let info = router
        .session_info_for(&key)
        .await
        .expect("retired session stays listed");
    assert_eq!(info.status, "dormant".into(), "retired row reports dormant");
}

#[tokio::test]
async fn non_terminal_error_keeps_mapping() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s1".into(),
            message: "minor".into(),
            terminal: false,
        })
        .await;

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(out, Out::UpdateCard { .. }));
    assert!(
        map.get(&key).await.is_some(),
        "non-terminal error must keep the mapping"
    );
}

#[tokio::test]
async fn terminal_error_preserves_pre_death_transcript() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_x", None);
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    // 显式允许输出 tool result，验证死前 ToolEnd 内容保留（默认 0 会屏蔽）。
    let (router, mut out_rx) = DispatchHandle::new_with_card_config(
        map.clone(),
        CardConfig {
            max_tool_output_chars: 100,
            ..CardConfig::default()
        },
    );

    // 累积若干事件（死前 transcript）。
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::TextDelta {
                session_id: "s1".into(),
                delta: "step1".into(),
            },
        )
        .await;
    let _ = drain(&mut out_rx).await;
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::ToolStart {
                session_id: "s1".into(),
                tool_name: "Bash".into(),
                args: serde_json::json!({}),
            },
        )
        .await;
    let _ = drain(&mut out_rx).await;
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::ToolEnd {
                session_id: "s1".into(),
                tool_name: "Bash".into(),
                result: "step2".into(),
            },
        )
        .await;
    let _ = drain(&mut out_rx).await;

    // terminal Error：死前 transcript 必须保留 + 错误正文。
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s1".into(),
            message: "agent crashed".into(),
            terminal: true,
        })
        .await;

    let outs = drain(&mut out_rx).await;
    let card = outs
        .iter()
        .find_map(|o| match o {
            Out::UpdateCard { session_id, card } if session_id == "s1" => Some(card),
            _ => None,
        })
        .expect("expected terminal UpdateCard");
    let s = serde_json::to_string(card).unwrap();
    assert!(s.contains('❌'), "❌ emoji: {s}");
    assert!(s.contains("step1"), "死前 TextDelta 保留: {s}");
    assert!(s.contains("step2"), "死前 ToolEnd 保留: {s}");
    assert!(s.contains("agent crashed"), "错误正文: {s}");
    // 终态视觉由 card body 表达（❌ 行已 push），不再 Out::React 换 FAILED。
    assert!(
        !outs
            .iter()
            .any(|o| matches!(o, Out::React { emoji, .. } if emoji
                == sebas_dispatch::card_state::phase::FAILED)),
        "terminal 不应发 Out::React FAILED: {outs:?}"
    );
    // （fix-webui-qa-defects-round4 1.2）记录保留：映射以 Dormant 形态存在，
    // 死前 transcript + 错误条目经 session_turns 全部可回看。
    let m = map
        .get(&key)
        .await
        .expect("terminal 必须保留会话记录（Dormant）");
    assert!(m.session_id().is_none(), "活跃绑定必须清掉: {m:?}");
    let turns = router
        .session_turns(&key, 0)
        .await
        .expect("retired record keeps the transcript retrievable");
    let joined = turns
        .iter()
        .map(|t| t.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("step1"), "死前 TextDelta 保留: {joined}");
    assert!(joined.contains("step2"), "死前 ToolEnd 保留: {joined}");
    assert!(joined.contains("agent crashed"), "错误正文: {joined}");
}

// ── fix-webui-qa-defects 5.1（design D5）：is_error 终态合成 error 条目 ─────

/// refusal（非终态 Error + Finished 配对，无任何文本条目）的回合不再
/// 「石沉大海」：transcript 出现携带 generic 分类的 error 条目，消息原文
/// 保留，会话照常存活。
#[tokio::test]
async fn refusal_error_synthesizes_a_classified_transcript_entry() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_refuse", None);
    map.insert(key.clone(), Mapping::active("s-refuse"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

    // refusal result 帧的映射产物：Error{terminal:false} + Finished。
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s-refuse".into(),
            message: "I cannot help with that request.".into(),
            terminal: false,
        })
        .await;
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "s-refuse".into(),
        })
        .await;
    let _ = drain(&mut out_rx).await;

    let turns = router.session_turns(&key, 0).await.expect("mapping exists");
    let err = turns
        .iter()
        .find(|t| t.element_type == "error".into())
        .expect("a refused turn must leave a visible error entry");
    assert!(
        err.content.contains("I cannot help with that request."),
        "the refusal text rides the entry: {err:?}"
    );
    assert_eq!(
        err.failure_class.as_deref(),
        Some(sebas_dispatch::failure_class::GENERIC),
        "agent-turn errors carry the generic failure class"
    );
    assert!(
        map.get(&key).await.is_some(),
        "refusal keeps the session alive"
    );
}

/// SetMode 失败（带「模式未变」标记的非终态 Error）不重复合成 error 条目
/// ——它已由 permission_mode_result 契约条目上报。
#[tokio::test]
async fn mode_unchanged_error_does_not_duplicate_an_error_entry() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_mode_err", None);
    map.insert(key.clone(), Mapping::active("s-mode-err"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());

    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s-mode-err".into(),
            message: "permission mode 未变（模式未变）".into(),
            terminal: false,
        })
        .await;
    let _ = drain(&mut out_rx).await;

    let turns = router.session_turns(&key, 0).await.expect("mapping exists");
    assert!(
        !turns.iter().any(|t| t.element_type == "error".into()),
        "the mode-unchanged marker error must not synthesize a duplicate error entry: {turns:?}"
    );
}

/// 终态 Error 同样合成带分类的条目。条目在拆除臂的 apply_event 内先于
/// 退役写入 turn 存储（round4 后拆除不再抹记录，条目随记录保留可回看）。
#[tokio::test]
async fn terminal_error_also_lands_a_classified_entry() {
    let map = SessionMap::new();
    let key = ChannelKey::feishu("oc_term", None);
    map.insert(key.clone(), Mapping::active("s-term"))
        .await
        .unwrap();
    let (router, _out_rx) = DispatchHandle::new(map.clone());

    // apply_event = 事件落账的公共半边（拆除发生在 apply_event_to_out 的
    // 终态臂）；条目在本事件内先于映射移除写入 turn 存储。
    router
        .apply_event(
            "s-term",
            &AcpEvent::Error {
                session_id: "s-term".into(),
                message: "agent process exited".into(),
                terminal: true,
            },
        )
        .await;

    let turns = router.session_turns(&key, 0).await.expect("mapping still up");
    let err = turns
        .iter()
        .find(|t| t.element_type == "error".into())
        .expect("terminal error must land a transcript entry");
    assert_eq!(err.content, "agent process exited");
    assert_eq!(
        err.failure_class.as_deref(),
        Some(sebas_dispatch::failure_class::GENERIC)
    );
}

// ── fix-webui-qa-defects-round4 1.2/1.3：升级击杀保留会话 ────────────────

/// 升级击杀（driver 终态 Error）后会话必须仍可回看：列表快照含该行
/// （dormant 态）、转录经 session API 可取、受影响回合以携带升级原因的
/// error 条目收尾、无静默移除事件。
#[tokio::test]
async fn escalation_kill_keeps_the_session_browsable() {
    let map = SessionMap::new();
    let key = ChannelKey::new("web", "web-hang");
    map.insert(key.clone(), Mapping::active("s-hang"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    let mut events = router.subscribe_session_events();

    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s-hang".into(),
            message: "agent hung (no activity for 5m; 3 cancels failed)".into(),
            terminal: true,
        })
        .await;
    let _ = drain(&mut out_rx).await;

    // 列表仍含该会话，dormant 态（不再是 active/spawning）。
    let infos = router.session_info_snapshot().await;
    let row = infos
        .iter()
        .find(|i| i.key == key.reference)
        .expect("escalation kill must keep the session listed");
    assert_eq!(row.status, "dormant".into());

    // 详情（transcript）可回看，末尾是携带升级原因的 error 条目。
    let turns = router
        .session_turns(&key, 0)
        .await
        .expect("transcript must remain retrievable");
    let err = turns
        .iter()
        .find(|t| t.element_type == "error".into())
        .expect("the killed turn must be finalized with a visible error entry");
    assert!(
        err.content.contains("agent hung"),
        "the escalation cause rides the entry: {err:?}"
    );

    // 无静默移除：到达的生命周期事件里没有 Removed。
    let mut saw_removed = false;
    let mut saw_updated = false;
    while let Ok(ev) = events.try_recv() {
        match ev {
            sebas_dispatch::engine::SessionEvent::Removed { .. } => saw_removed = true,
            sebas_dispatch::engine::SessionEvent::Updated { session } => {
                if session.status == "dormant".into() {
                    saw_updated = true;
                }
            }
            _ => {}
        }
    }
    assert!(!saw_removed, "record removal must not be announced");
    assert!(saw_updated, "retirement must refresh the row via Updated");
}

/// （1.3）升级终止时排队提交按 pending-queue 语义如实释放并上报
/// not-executed，不随记录消失或滞留在退役行上。
#[tokio::test]
async fn escalation_kill_releases_queued_submissions_with_reporting() {
    use sebas_dispatch::state::QueuedTurn;
    let map = SessionMap::new();
    let key = ChannelKey::new("web", "web-hang-q");
    map.insert(key.clone(), Mapping::active("s-hang-q"))
        .await
        .unwrap();
    map.enqueue_turn(&key, QueuedTurn::new("second message", None, false))
        .await;
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    let mut events = router.subscribe_session_events();

    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s-hang-q".into(),
            message: "agent hung".into(),
            terminal: true,
        })
        .await;
    let _ = drain(&mut out_rx).await;

    // 逐条上报：PendingDropped 携带被释放的提交原文。
    let mut dropped_texts = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let sebas_dispatch::engine::SessionEvent::PendingDropped { dropped, .. } = ev {
            dropped_texts.extend(dropped.into_iter().map(|d| d.text));
        }
    }
    assert_eq!(
        dropped_texts,
        vec!["second message".to_string()],
        "the queued submission must be reported as not executed"
    );
    // 释放后退役行上不再滞留 pending。
    assert!(
        router.session_pending(&key).await.is_empty(),
        "retired row must not keep dropped submissions"
    );
    // 记录本身仍在。
    assert!(
        router.session_info_for(&key).await.is_some(),
        "the session record survives the release"
    );
}

/// （1.2 命名连续性）卡态丢弃前命名来源迁进映射：升级后行名仍来自
/// 首 prompt 预览，不退化为短 id。
#[tokio::test]
async fn escalation_kill_keeps_the_row_named_by_the_first_prompt() {
    let map = SessionMap::new();
    let key = ChannelKey::new("web", "web-hang-name");
    map.insert(key.clone(), Mapping::active("s-hang-name"))
        .await
        .unwrap();
    let (router, mut out_rx) = DispatchHandle::new(map.clone());
    router.seed_card("s-hang-name".into(), "跑一下命令".into()).await;

    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s-hang-name".into(),
            message: "agent hung".into(),
            terminal: true,
        })
        .await;
    let _ = drain(&mut out_rx).await;

    let info = router
        .session_info_for(&key)
        .await
        .expect("record stays listed");
    assert_eq!(
        info.user_prompt.as_deref(),
        Some("跑一下命令"),
        "the row name source must survive card-state teardown"
    );
}
