//! sebas-9pz ① regression: a child that is ALIVE but silent (no frames for
//! `SEBAS_HANG_TIMEOUT_SECS`) must be detected as hung and the session torn
//! down with a terminal error — not left running forever.
//!
//! The fake child enters an infinite silent loop on the "hang" prompt. The
//! liveness probe (`set_permission_mode`) still answers (process alive), so
//! only the hang detector can fire.

use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use std::path::PathBuf;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli")
}

#[tokio::test]
async fn silent_child_is_detected_as_hung_and_torn_down() {
    // 1s hang threshold so the test doesn't wait 5 minutes. Test-only env
    // mutation is single-threaded (tokio current-thread not required, but
    // the default test binary runs tests concurrently — this is the one
    // place that touches process env, and SEBAS_* is namespaced enough to
    // not collide with other tests).
    // SAFETY: no other thread reads this var concurrently in this test.
    unsafe { std::env::set_var("SEBAS_HANG_TIMEOUT_SECS", "1") };

    let mgr = SessionManager::new(Duration::from_secs(30));
    let id = mgr
        .create_session(
            fake().to_str().unwrap(),
            // `--ignore-interrupt` makes the fake ack cancels but keep going,
            // so the FULL escalation chain (interrupt×3 → disconnect → 5s →
            // drop) must run instead of the interrupt killing it on the first
            // shot.
            vec!["--ignore-interrupt".into()],
            None,
            vec![],
            "".into(),
        )
        .await
        .expect("spawn");

    // A prompt that makes the fake child stop responding forever.
    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "hang".into(),
        },
    )
    .await
    .expect("prompt");

    // The driver must eventually surface a terminal error (hang detected).
    // The full escalation chain takes ≈ 1s (detect) + 3×2s (interrupt grace)
    // + 5s (SIGKILL grace) ≈ 12s, during which the driver emits NO events
    // (the terminal error arrives only at the end). So we cannot use a short
    // per-read timeout — read with a total 30s deadline instead.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "hung child was not torn down within 30s"
        );
        let evt = mgr.next_event(&id).await.expect("stream open");
        match evt {
            AcpEvent::Error {
                terminal: true,
                message,
                ..
            } => {
                assert!(
                    message.contains("hung") || message.contains("unresponsive"),
                    "expected hang message, got: {message}"
                );
                break;
            }
            _ => continue,
        }
    }

    // After the terminal error the manager should have reaped the entry.
    // SAFETY: same single-threaded rationale as the set_var above.
    unsafe { std::env::remove_var("SEBAS_HANG_TIMEOUT_SECS") };
}
