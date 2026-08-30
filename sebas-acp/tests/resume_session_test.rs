//! lazy-resume coverage (post-ACP rewrite; openspec/specs/acp-driver/spec.md):
//! - resuming a live conversation id keeps the id and answers prompts
//! - resume rejection (conversation files gone) transparently falls back to
//!   a fresh session with a NEW id, `resumed == false` (sebas-dk8.4), and
//!   the fallback is FAST (stderr watch, not the startup-timeout path)

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

/// Drain events until a Finished arrives (or timeout); returns the deltas seen.
async fn drain_until_finished(mgr: &SessionManager, id: &str) -> Vec<String> {
    let mut deltas = Vec::new();
    for _ in 0..8 {
        let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(id))
            .await
            .expect("timeout waiting for event")
            .expect("event stream closed");
        match evt {
            AcpEvent::TextDelta { delta, .. } => deltas.push(delta),
            AcpEvent::Finished { .. } => return deltas,
            _ => {}
        }
    }
    panic!("no Finished within 8 events");
}

#[tokio::test]
async fn resume_keeps_old_id_and_answers() {
    let mgr = SessionManager::new(Duration::from_secs(30));
    // The fake accepts --resume <id> and echoes it as its session_id.
    let outcome = mgr
        .resume_session(fake().to_str().unwrap(), vec![], None, vec![], "sess-old-1")
        .await
        .expect("resume_session");
    assert!(outcome.resumed, "native resume path must report resumed");
    assert_eq!(outcome.session_id, "sess-old-1", "routing id = resumed id");

    // The resumed session answers prompts under the OLD id.
    mgr.send(
        &outcome.session_id,
        AcpCommand::ContinueSession {
            session_id: outcome.session_id.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send prompt");
    let deltas = drain_until_finished(&mgr, &outcome.session_id).await;
    assert_eq!(deltas.concat(), "hello world");

    mgr.kill(&outcome.session_id).await;
}

#[tokio::test]
async fn resume_rejected_falls_back_to_fresh_session() {
    // Generous startup timeout: the fallback must fire via the driver's
    // stderr watch ("No conversation found"), NOT by timing out — the 10s
    // outer guard fails the test if we ever regress to the hang path.
    let mgr = SessionManager::new(Duration::from_secs(30));
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        mgr.resume_session(
            fake().to_str().unwrap(),
            vec!["--resume-fails".into()],
            None,
            vec![],
            "sess-deleted",
        ),
    )
    .await
    .expect("fallback must be fast (stderr watch), not a startup-timeout hang")
    .expect("rejected resume falls back instead of erroring");

    assert!(!outcome.resumed, "fallback reports resumed=false");
    assert_ne!(
        outcome.session_id, "sess-deleted",
        "fallback mints a fresh routing id"
    );

    // The fresh session is fully functional under the new id.
    mgr.send(
        &outcome.session_id,
        AcpCommand::ContinueSession {
            session_id: outcome.session_id.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send prompt");
    let deltas = drain_until_finished(&mgr, &outcome.session_id).await;
    assert_eq!(deltas.concat(), "hello world");

    mgr.kill(&outcome.session_id).await;
}
