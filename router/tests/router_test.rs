use router::state::{SessionMap, Mapping};
use router::router::{RouterHandle, Out};
use feishu::events::{FeishuIn, SessionKey};
use acp_claude::session::AcpEvent;
use std::time::Duration;

#[tokio::test]
async fn new_text_creates_session_and_emits_initial_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map.clone());
    let key = SessionKey { chat_id: "oc_x".into(), thread_id: None };

    tokio::spawn(async move {
        let _ = router
            .dispatch(FeishuIn::Text {
                key: key.clone(),
                text: "hello".into(),
                reply_to: None,
            })
            .await;
    });

    let first = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await.unwrap().unwrap();
    // First event is some "send_card" or "spawn acp" — we assert shape loosely:
    assert!(matches!(first, Out::SendCard { .. } | Out::SpawnAcp { .. }));
}

#[tokio::test]
async fn existing_session_dispatches_continue() {
    let map = SessionMap::new();
    let k = SessionKey { chat_id: "oc_x".into(), thread_id: None };
    map.insert(k.clone(), Mapping { session_id: "existing".into(), last_active_unix: 1 }).await;

    let (router, mut out_rx) = RouterHandle::new(map.clone());
    tokio::spawn(async move {
        let _ = router.dispatch(FeishuIn::Text {
            key: k.clone(),
            text: "more".into(),
            reply_to: None,
        }).await;
    });

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await.unwrap().unwrap();
    match out {
        Out::SendAcp { session_id, .. } => assert_eq!(session_id, "existing"),
        other => panic!("expected SendAcp, got {other:?}"),
    }
}

#[tokio::test]
async fn apply_event_to_out_renders_update_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

    let evt = AcpEvent::TextDelta {
        session_id: "s1".into(),
        delta: "hi".into(),
    };
    router.apply_event_to_out("s1".into(), &evt).await;

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await.unwrap().unwrap();
    match out {
        Out::UpdateCard { session_id, card } => {
            assert_eq!(session_id, "s1");
            assert!(card.is_object());
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
}
