//! D5: create_session must give up within startup_timeout when the agent
//! never answers `initialize` — and tear down the half-spawned child instead
//! of leaking it.

use acp_claude::manager::SessionManager;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli")
}

#[tokio::test]
async fn hanging_agent_times_out_and_is_reaped() {
    let mgr = SessionManager::new(Duration::from_millis(400));
    let t0 = Instant::now();
    let res = mgr
        .create_session(
            fake().to_str().unwrap(),
            vec!["--hang-on-init".into()],
            None,
            "".into(),
        )
        .await;
    let elapsed = t0.elapsed();
    let err = res.expect_err("hanging agent must not yield a session");
    assert!(
        err.to_string().contains("timed out"),
        "error should mention timeout: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "timeout took too long: {elapsed:?}"
    );
    // Give the teardown a moment, then assert no fake-claude child lingers.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let out = std::process::Command::new("pgrep")
        .args(["-f", "fake-claude-cli --hang-on-init"])
        .output()
        .expect("pgrep");
    assert!(
        out.stdout.is_empty(),
        "fake-claude child leaked after timeout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
