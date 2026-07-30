//! Terminal AcpEvent::Error: the router must remove the session mapping and
//! emit an ❌ UpdateCard. Non-terminal errors keep the existing behaviour.

use acp_claude::session::AcpEvent;
use feishu::events::SessionKey;
use router::router::{Out, RouterHandle};
use router::state::{Mapping, SessionMap};
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
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

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
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

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
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    map.insert(key.clone(), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

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
    // p3g：terminal 还应把 root 卡 reaction 换成 ❌
    assert!(
        outs.iter()
            .any(|o| matches!(o, Out::React { emoji, .. } if emoji == "❌")),
        "terminal 换 ❌ reaction: {outs:?}"
    );
    assert!(map.get(&key).await.is_none(), "terminal 必清 mapping");
}
