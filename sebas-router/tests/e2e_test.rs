use sebas_acp::claude::session::AcpEvent;
use sebas_feishu::events::{FeishuIn, SessionKey};
use sebas_router::router::{Out, RouterHandle};
use sebas_router::state::SessionMap;
use std::time::Duration;

const WAIT: Duration = Duration::from_millis(500);

#[tokio::test]
async fn full_round_trip_text_to_events() {
    let map = SessionMap::new();
    let (handle, mut out_rx) = RouterHandle::new(map.clone());
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };

    // 1) Text in → expect SpawnAcp only. The root SendCard is now emitted by
    // the dispatcher (after create_session mints the real session_id), not the
    // router, so it does not appear on this channel.
    handle
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "hello".into(),
            reply_to: None,
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;

    let first = tokio::time::timeout(WAIT, out_rx.recv())
        .await
        .expect("SpawnAcp not received in time")
        .expect("channel closed");
    assert!(
        matches!(first, Out::SpawnAcp { .. }),
        "expected SpawnAcp, got {first:?}"
    );

    // 2) ACP event in → expect UpdateCard
    handle
        .dispatch_acp_event(AcpEvent::TextDelta {
            session_id: "s1".into(),
            delta: "hi back".into(),
        })
        .await;

    let out = tokio::time::timeout(WAIT, out_rx.recv())
        .await
        .expect("UpdateCard not received in time")
        .expect("channel closed");
    assert!(
        matches!(out, Out::UpdateCard { .. }),
        "expected UpdateCard, got {out:?}"
    );
}
