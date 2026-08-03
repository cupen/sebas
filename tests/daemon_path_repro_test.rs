//! Regression: production daemon path can drive the in-tree bridge through
//! `acp_spawn_and_activate` to a finished session, not just the hand-rolled
//! `fake-claude` agent. Catches path-resolution, env-propagation and
//! handshake-deadlock regressions on the daemon-side spawn flow.

use acp_claude::manager::SessionManager;
use feishu::events::SessionKey;
use router::router::RouterHandle;
use router::state::SessionMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn workspace_target() -> PathBuf {
    // sebas integration tests' CARGO_MANIFEST_DIR IS the workspace root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug")
}

#[tokio::test]
async fn daemon_handshake_with_in_tree_bridge_finishes_under_5s() {
    let bridge = workspace_target().join("claude-acp-bridge");
    let fake = workspace_target().join("fake-stream-claude");
    assert!(bridge.exists(), "missing build artifact {}", bridge.display());
    assert!(fake.exists(), "missing build artifact {}", fake.display());

    // Bridge reads SEBAS_CLAUDE_PATH to locate its claude child; default is
    // "claude" on PATH. Override so we drive fake-stream-claude "hello".
    unsafe { std::env::set_var("SEBAS_CLAUDE_PATH", &fake); }

    let map = SessionMap::new();
    let (router, _out_rx) = RouterHandle::new(map);
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(15)));

    let key = SessionKey { chat_id: "oc_x".into(), thread_id: None };

    let t0 = std::time::Instant::now();
    let (_sid, _pending, _rx) = sebas::run::acp_spawn_and_activate(
        &mgr,
        &router,
        &key,
        "hello",
        bridge.to_str().unwrap(),
        vec!["hello".into()],
        Some("/tmp".into()),
    )
    .await
    .expect("spawn bridge through production path");

    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "handshake took {elapsed:?}, expected sub-5s",
        elapsed = t0.elapsed(),
    );
}
