//! spec §3.3e lazy-resume coverage (post-ACP rewrite):
//! - resuming a live conversation id keeps the id and answers prompts
//! - resuming with a CLI that rejects the resume errors the spawn
//!   (Phase 1 semantics: NO transparent fallback — that is Phase 3,
//!   sebas-dk8.4; the old "capability"/"load error → session/new" tests
//!   tested ACP-session/load fallback machinery that no longer exists)

use acp_claude::manager::SessionManager;
use acp_claude::session::{AcpCommand, AcpEvent};
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
        .resume_session(fake().to_str().unwrap(), vec![], None, "sess-old-1")
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
async fn resume_rejected_by_cli_errors_spawn() {
    let mgr = SessionManager::new(Duration::from_millis(800));
    // --resume-fails: the fake exits(1) immediately, modelling claude's
    // "No conversation found" startup failure. The spawn must surface an
    // error (Phase 1: no silent fresh-session fallback).
    let res = mgr
        .resume_session(
            fake().to_str().unwrap(),
            vec!["--resume-fails".into()],
            None,
            "sess-deleted",
        )
        .await;
    assert!(res.is_err(), "rejected resume must error, got {res:?}");
}
