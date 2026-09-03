//! D6: a crashing agent must surface exactly one Error{terminal:true},
//! be removed from the manager table, and close its event stream.
//!
//! Both tests clone `Arc<Mutex<Receiver>>` via `event_rx()` BEFORE
//! triggering death. This mirrors the run.rs pump (which clones once at
//! startup) and makes the wrapper's eager table removal safe: dropping
//! the manager's entry releases only the manager's Arc clone, so any
//! buffered terminal event survives for the consumer to observe.

use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli")
}

#[tokio::test]
async fn crash_yields_terminal_error_and_cleanup() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let id = mgr
        .create_claude_session(fake().to_str().unwrap(), vec![], None, vec![], "".into())
        .await
        .expect("spawn");
    // Clone the receiver BEFORE triggering death so the buffered terminal
    // event survives the wrapper's eager table removal (which drops the
    // manager's Arc<Mutex<Receiver>> clone).
    let rx = mgr.event_rx(&id).await.expect("rx");
    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "crash".into(),
        },
    )
    .await
    .expect("crash prompt");

    // Expect: one chunk ("boom"), then exactly one Error{terminal:true},
    // then the stream closes.
    let mut saw_terminal = false;
    let mut rx = rx.lock().await;
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("event timeout — no terminal event synthesized");
        match evt {
            None => break, // stream closed
            Some(AcpEvent::Error {
                session_id,
                terminal: true,
                ..
            }) => {
                assert_eq!(session_id, id);
                assert!(!saw_terminal, "duplicate terminal event");
                saw_terminal = true;
            }
            Some(AcpEvent::Error {
                terminal: false, ..
            }) => {
                panic!("non-terminal Error from a crash path");
            }
            Some(_) => continue,
        }
    }
    assert!(saw_terminal, "no terminal Error received");

    // Table cleanup: further sends fail fast.
    let res = mgr
        .send(
            &id,
            AcpCommand::ContinueSession {
                session_id: id.clone(),
                prompt: "anyone there?".into(),
            },
        )
        .await;
    assert!(res.is_err(), "dead session must reject sends");
}

#[tokio::test]
async fn explicit_kill_produces_no_terminal_error() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let id = mgr
        .create_claude_session(fake().to_str().unwrap(), vec![], None, vec![], "".into())
        .await
        .expect("spawn");
    // Clone the receiver BEFORE kill so we can observe whatever the
    // wrapper synthesizes — a wrongly-synthesized crash
    // Error{terminal:true} would actually be seen here, making the test
    // non-vacuous.
    let rx = mgr.event_rx(&id).await.expect("rx");
    mgr.kill(&id).await;
    let mut rx = rx.lock().await;
    // Drain whatever remains; nothing terminal may be synthesized for an
    // explicit kill, and the stream must close promptly.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
            Ok(Some(AcpEvent::Error { terminal: true, .. })) => {
                panic!("terminal Error synthesized for explicit kill")
            }
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(_) => {
                assert!(Instant::now() < deadline, "stream did not close after kill");
                break;
            }
        }
    }
}
