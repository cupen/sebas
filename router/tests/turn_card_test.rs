//! Task 5: continue_session emits per-turn SendCard

use acp_claude::session::AcpEvent;
use feishu::events::{FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::{Mapping, SessionMap};

#[tokio::test]
async fn continue_session_emits_per_turn_send_card_with_root_id() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    let key = SessionKey { chat_id: "oc_x".into(), thread_id: None };
    let _ = router.map.insert(key.clone(), Mapping::active("sess-1")).await;
    router.seed_card("sess-1".into(), "first".into()).await;

    // First turn finishes: use apply_event (pure state, no emission).
    let _ = router.apply_event("sess-1", &AcpEvent::Finished { session_id: "sess-1".into() }).await;

    // User sends a 2nd message that quotes-back to om_user_2.
    // continue_session flips DONE->WORKING, emitting [UpdateCard, React],
    // then emits [SendCard (per-turn), SendAcp].
    router.dispatch(FeishuIn::Text {
        key: key.clone(),
        text: "follow-up".into(),
        reply_to: Some("om_user_2".into()),
    }).await;

    // Drain the flip messages first (UpdateCard + React from DONE->WORKING).
    let _ = out_rx.recv().await; // UpdateCard
    let _ = out_rx.recv().await; // React

    // Now drain and assert the per-turn emissions: SendCard + SendAcp.
    let first = out_rx.recv().await.unwrap();
    let second = out_rx.recv().await.unwrap();
    match (&first, &second) {
        (Out::SendCard { root_id: Some(rid), .. }, Out::SendAcp { .. }) => {
            assert_eq!(rid, "om_user_2");
        }
        _ => panic!("expected SendCard(root_id=Some(_)) then SendAcp, got {first:?} then {second:?}"),
    }
}
