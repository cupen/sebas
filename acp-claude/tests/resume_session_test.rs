//! spec §3.3e lazy-resume coverage against fake-claude:
//! - load-capable agent resumes with the requested id
//! - agent without the capability falls back to session/new
//! - agent whose session/load errors falls back to session/new

use acp_claude::manager::SessionManager;
use acp_claude::session::{AcpCommand, AcpEvent};
use std::path::PathBuf;
use std::time::Duration;

fn fake_claude() -> PathBuf {
    // `cargo test` runs from the package dir (acp-claude/), so resolve the
    // workspace-root target/ via CARGO_MANIFEST_DIR's parent.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude")
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
async fn resume_with_load_capable_agent_keeps_id() {
    let mgr = SessionManager::new(Duration::from_secs(30));
    let outcome = mgr
        .resume_session(
            fake_claude().to_str().unwrap(),
            vec!["--enable-load".into()],
            None,
            "sess-old-1",
        )
        .await
        .expect("resume_session");
    assert!(outcome.resumed, "load-capable agent must resume");
    assert_eq!(outcome.session_id, "sess-old-1");

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
async fn resume_without_capability_falls_back_to_new() {
    let mgr = SessionManager::new(Duration::from_secs(30));
    // Plain fake-claude advertises loadSession:false → the manager must not
    // even send session/load; it falls straight back to session/new.
    let outcome = mgr
        .resume_session(fake_claude().to_str().unwrap(), vec![], None, "sess-gone")
        .await
        .expect("resume_session");
    assert!(!outcome.resumed, "no capability → fallback to session/new");
    assert_eq!(outcome.session_id, "sess-1", "fresh id from session/new");

    mgr.send(
        &outcome.session_id,
        AcpCommand::CreateSession {
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
async fn resume_with_load_error_falls_back_to_new() {
    let mgr = SessionManager::new(Duration::from_secs(30));
    // Capability advertised but session/load errors (session files deleted)
    // → transparent fallback to session/new (spec §3.3e).
    let outcome = mgr
        .resume_session(
            fake_claude().to_str().unwrap(),
            vec!["--load-fails".into()],
            None,
            "sess-deleted",
        )
        .await
        .expect("resume_session");
    assert!(!outcome.resumed, "load error → fallback to session/new");
    assert_eq!(outcome.session_id, "sess-1", "fresh id from session/new");

    mgr.send(
        &outcome.session_id,
        AcpCommand::CreateSession {
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
