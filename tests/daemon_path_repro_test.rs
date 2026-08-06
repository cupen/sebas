//! Regression: the production daemon path drives the new-dialect fake CLI
//! through `acp_spawn_and_activate` to an established session. Catches
//! path-resolution, env-propagation and handshake-deadlock regressions on
//! the daemon-side spawn flow. (Post-ACP: the "agent" is now the claude
//! CLI itself — fake-claude speaks its stream-json + control protocol.)

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
async fn daemon_handshake_with_fake_cli_finishes_under_5s() {
    let fake = workspace_target().join("fake-claude");
    assert!(fake.exists(), "missing build artifact {}", fake.display());

    let map = SessionMap::new();
    let (router, _out_rx) = RouterHandle::new(map);
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(15)));

    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };

    let t0 = std::time::Instant::now();
    let (_sid, _pending, _rx) = sebas::run::acp_spawn_and_activate(
        &mgr,
        &router,
        &key,
        "hello",
        fake.to_str().unwrap(),
        vec![],
        Some("/tmp".into()),
    )
    .await
    .expect("spawn fake CLI through production path");

    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "handshake took {elapsed:?}, expected sub-5s",
        elapsed = t0.elapsed(),
    );
}
