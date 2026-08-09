use feishu::cards::ThinkingDisplay;
use router::router::{Out, RouterHandle};
use router::settings::load_settings;
use router::state::SessionMap;
use feishu::events::SessionKey;
use std::path::PathBuf;

fn key() -> SessionKey {
    SessionKey {
        chat_id: "oc_test".into(),
        thread_id: None,
    }
}

async fn next_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    rx.recv().await.expect("expected Out")
}

/// Per-test tempdir so we never touch the developer's real `~/.sebas/settings.json`.
/// Process ID + atomic counter make the path unique even under parallel `cargo test`.
fn tempdir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "sebas-settings-handler-test-{}-{}",
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[tokio::test]
async fn settings_list_emits_all_keys() {
    let (router, mut rx) = RouterHandle::new(SessionMap::new());
    let path = tempdir().join("settings.json");

    router
        .handle_settings(key(), None, None, &path)
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
    let path = tempdir().join("settings.json");

    router
        .handle_settings(key(), Some("thinking".into()), Some("hide".into()), &path)
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { content, .. } = out else {
        panic!("expected PlainText, got {out:?}");
    };
    assert!(content.contains("hide"), "expected hide in reply: {content}");

    // Verify in-memory config updated.
    let cfg = router.card_config().await;
    assert_eq!(cfg.thinking, ThinkingDisplay::Hide);

    // Verify file written at the tempdir path, not the developer's real one.
    let loaded = load_settings(&path).unwrap();
    assert_eq!(loaded.thinking, ThinkingDisplay::Hide);
}

#[tokio::test]
async fn settings_rejects_invalid_value() {
    let (router, mut rx) = RouterHandle::new(SessionMap::new());
    let path = tempdir().join("settings.json");

    router
        .handle_settings(key(), Some("thinking".into()), Some("disable".into()), &path)
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
