use feishu::cards::ThinkingDisplay;
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use feishu::events::SessionKey;

fn key() -> SessionKey {
    SessionKey {
        chat_id: "oc_test".into(),
        thread_id: None,
    }
}

async fn next_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    rx.recv().await.expect("expected Out")
}

#[tokio::test]
async fn settings_list_emits_all_keys() {
    let (router, mut rx) = RouterHandle::new(SessionMap::new());
    let _ = std::fs::remove_file(router::settings::settings_path());
    router
        .dispatch(feishu::events::FeishuIn::Text {
            key: key(),
            text: "/settings".into(),
            reply_to: None,
        })
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { key: _k, content } = out else {
        panic!("expected PlainText, got {out:?}");
    };
    assert!(content.contains("thinking"), "missing thinking in list: {content}");
    assert!(content.contains("show"), "default thinking not shown: {content}");
}

#[tokio::test]
async fn settings_set_persists_and_updates_router() {
    let (router, mut rx) = RouterHandle::new(SessionMap::new());
    // Clean any leftover from previous runs.
    let _ = std::fs::remove_file(router::settings::settings_path());

    router
        .dispatch(feishu::events::FeishuIn::Text {
            key: key(),
            text: "/settings thinking hide".into(),
            reply_to: None,
        })
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { content, .. } = out else {
        panic!("expected PlainText, got {out:?}");
    };
    assert!(content.contains("hide"), "expected hide in reply: {content}");

    // Verify in-memory config updated.
    let cfg = router.card_config().await;
    assert_eq!(cfg.thinking, ThinkingDisplay::Hide);

    // Verify file written.
    let loaded = router::settings::load_settings(&router::settings::settings_path()).unwrap();
    assert_eq!(loaded.thinking, ThinkingDisplay::Hide);
}

#[tokio::test]
async fn settings_rejects_invalid_value() {
    let (router, mut rx) = RouterHandle::new(SessionMap::new());
    let _ = std::fs::remove_file(router::settings::settings_path());
    router
        .dispatch(feishu::events::FeishuIn::Text {
            key: key(),
            text: "/settings thinking disable".into(),
            reply_to: None,
        })
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { content, .. } = out else {
        panic!("expected PlainText");
    };
    assert!(
        content.contains("可选值") || content.contains("show"),
        "expected validation error, got {content}"
    );
}