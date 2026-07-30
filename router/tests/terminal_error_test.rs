//! Terminal AcpEvent::Error: the router must remove the session mapping and
//! emit an ❌ UpdateCard. Non-terminal errors keep the existing behaviour.

use acp_claude::session::AcpEvent;
use router::router::{Out, RouterHandle};
use router::state::{Mapping, SessionMap};
use std::time::Duration;
use feishu::events::SessionKey;

#[tokio::test]
async fn terminal_error_removes_mapping_and_marks_card() {
    let map = SessionMap::new();
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    map.insert(
        key.clone(),
        Mapping {
            session_id: "s1".into(),
            last_active_unix: 1,
        },
    )
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
    map.insert(
        key.clone(),
        Mapping {
            session_id: "s1".into(),
            last_active_unix: 1,
        },
    )
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
