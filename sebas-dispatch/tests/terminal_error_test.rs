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
async fn terminal_error_removes_mapping_and_marks_card() {
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
    assert!(
        map.get(&key).await.is_none(),
        "terminal error must remove the session mapping"
    );
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
    assert!(map.get(&key).await.is_none(), "terminal 必清 mapping");
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
        .find(|t| t.element_type == "error")
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
        !turns.iter().any(|t| t.element_type == "error"),
        "the mode-unchanged marker error must not synthesize a duplicate error entry: {turns:?}"
    );
}

/// 终态 Error 同样合成带分类的条目。映射在同一事件的拆除臂中被移除
/// （终态会话整体消失是既有契约），故经 apply_event 在拆除前断言条目。
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
        .find(|t| t.element_type == "error")
        .expect("terminal error must land a transcript entry");
    assert_eq!(err.content, "agent process exited");
    assert_eq!(
        err.failure_class.as_deref(),
        Some(sebas_dispatch::failure_class::GENERIC)
    );
}
