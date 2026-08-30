//! Tests for the WebUI session-management surface on `RouterHandle`:
//! `web_close_session` tears down the mapping + (when present) the live
//! child process; `web_set_active` records the focused session.

use sebas_feishu::events::{FeishuIn, SessionKey};
use sebas_router::RouterHandle;
use sebas_router::router::CloseOutcome;
use sebas_router::state::{Mapping, SessionMap};

fn web_key(id: &str) -> SessionKey {
    SessionKey {
        chat_id: format!("web-{id}"),
        thread_id: None,
    }
}

#[tokio::test]
async fn close_unknown_key_returns_not_found() {
    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let out = router.web_close_session(web_key("ghost")).await;
    assert_eq!(out, CloseOutcome::NotFound);
}

#[tokio::test]
async fn close_active_session_drops_mapping_and_clears_active() {
    let map = SessionMap::new();
    let key = web_key("a");
    map.insert(key.clone(), Mapping::active("sid-a"))
        .await
        .unwrap();

    let (router, _rx) = RouterHandle::new(map.clone());
    router.web_set_active(key.clone()).await;
    assert_eq!(
        router.active_session_snapshot().await,
        Some(key.clone()),
        "active pointer should be set before close"
    );

    let out = router.web_close_session(key.clone()).await;
    assert_eq!(out, CloseOutcome::Closed);
    assert!(map.get(&key).await.is_none(), "mapping must drop");
    assert_eq!(
        router.active_session_snapshot().await,
        None,
        "active pointer should clear when the focused session is closed"
    );
}

#[tokio::test]
async fn close_spawning_placeholder_drops_by_key() {
    let map = SessionMap::new();
    let key = web_key("b");
    map.insert(key.clone(), Mapping::spawning()).await.unwrap();

    let (router, _rx) = RouterHandle::new(map.clone());
    let out = router.web_close_session(key.clone()).await;
    assert_eq!(out, CloseOutcome::Closed);
    assert!(map.get(&key).await.is_none());
}

#[tokio::test]
async fn close_dormant_drops_without_killing() {
    let map = SessionMap::new();
    let key = web_key("c");
    map.insert(key.clone(), Mapping::dormant("sid-c", 1))
        .await
        .unwrap();

    let (router, _rx) = RouterHandle::new(map.clone());
    let out = router.web_close_session(key.clone()).await;
    assert_eq!(out, CloseOutcome::Closed);
    assert!(map.get(&key).await.is_none());
}

#[tokio::test]
async fn close_session_clears_reply_target() {
    let map = SessionMap::new();
    let key = web_key("r");
    map.insert(key.clone(), Mapping::active("sid-r"))
        .await
        .unwrap();

    let (router, _rx) = RouterHandle::new(map.clone());
    // 模拟入站消息写入 reply target（话题内 = 话题根消息 message_id）。
    router
        .dispatch(FeishuIn::Text {
            key: key.clone(),
            text: "hello".into(),
            reply_to: Some("om_root".into()),
            chat_type: "private".into(),
            mentions: vec![],
        })
        .await;
    assert_eq!(router.reply_target(&key).await.as_deref(), Some("om_root"));

    let out = router.web_close_session(key.clone()).await;
    assert_eq!(out, CloseOutcome::Closed);
    assert_eq!(
        router.reply_target(&key).await,
        None,
        "close must clear the stale reply target"
    );
}

#[tokio::test]
async fn set_active_is_idempotent_and_overwrites() {
    let (router, _rx) = RouterHandle::new(SessionMap::new());
    let k1 = web_key("1");
    let k2 = web_key("2");

    router.web_set_active(k1.clone()).await;
    router.web_set_active(k1.clone()).await;
    assert_eq!(router.active_session_snapshot().await, Some(k1));

    router.web_set_active(k2.clone()).await;
    assert_eq!(
        router.active_session_snapshot().await,
        Some(k2),
        "second set should overwrite the first"
    );
}

#[tokio::test]
async fn close_unfocused_session_leaves_active_untouched() {
    let map = SessionMap::new();
    let a = web_key("a");
    let b = web_key("b");
    map.insert(a.clone(), Mapping::active("sid-a"))
        .await
        .unwrap();
    map.insert(b.clone(), Mapping::active("sid-b"))
        .await
        .unwrap();

    let (router, _rx) = RouterHandle::new(map.clone());
    router.web_set_active(a.clone()).await;

    let out = router.web_close_session(b.clone()).await;
    assert_eq!(out, CloseOutcome::Closed);
    assert_eq!(
        router.active_session_snapshot().await,
        Some(a.clone()),
        "closing a non-focused session must not touch the active pointer"
    );
}
