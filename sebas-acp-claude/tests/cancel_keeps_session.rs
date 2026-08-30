//! D4 regression: /cancel cancels the current TURN, not the session.
//! Pre-fix code broke the read loop after sending CancelNotification,
//! dropping the connection and SIGKILL-ing the child.

use sebas_acp_claude::manager::SessionManager;
use sebas_acp_claude::session::{AcpCommand, AcpEvent};
use std::path::PathBuf;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli")
}

async fn drain_until_finished(mgr: &SessionManager, id: &str) {
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(id))
            .await
            .expect("event timeout")
            .expect("stream open");
        if matches!(evt, AcpEvent::Finished { .. }) {
            return;
        }
    }
    panic!("no Finished within 10 events");
}

/// Consume any events produced by the Cancel notification itself.
/// With the old (broken) code the Cancel branch sends a synthetic
/// `Finished` and `break`s the read loop, so we must drain that
/// synthetic event before attempting the follow-up prompt.
/// With the new (fixed) code the Cancel is a pure notification and
/// the agent produces no response when there is no turn in flight,
/// so this function times out and returns.
async fn drain_cancel_fallout(mgr: &SessionManager, id: &str) {
    // Give the background task a chance to process the Cancel.
    tokio::task::yield_now().await;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), mgr.next_event(id)).await {
            Ok(Some(AcpEvent::Finished { .. })) => return, // synthetic Finished consumed
            Ok(Some(_)) => continue,                       // other events
            Ok(None) => panic!("session closed unexpectedly after cancel"),
            Err(_) => return, // timeout — session is idle
        }
    }
}

#[tokio::test]
async fn session_survives_cancel() {
    let mgr = SessionManager::new(Duration::from_secs(30));
    let id = mgr
        .create_session(fake().to_str().unwrap(), vec![], None, vec![], "".into())
        .await
        .expect("spawn");

    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("initial prompt");
    drain_until_finished(&mgr, &id).await;

    // Cancel (no turn in flight — agent ignores it, which is fine).
    mgr.send(
        &id,
        AcpCommand::Cancel {
            session_id: id.clone(),
        },
    )
    .await
    .expect("cancel");

    // Drain any synthetic events the old code might have produced.
    drain_cancel_fallout(&mgr, &id).await;

    // The session must still accept and answer a follow-up prompt.
    mgr.send(
        &id,
        AcpCommand::ContinueSession {
            session_id: id.clone(),
            prompt: "again".into(),
        },
    )
    .await
    .expect("follow-up prompt after cancel");
    drain_until_finished(&mgr, &id).await;
}
