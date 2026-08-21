//! sebas-9pz ⑤ regression: a refusal (agent declining the request) must NOT
//! tear the session down. The process is healthy — the next prompt must
//! still be answered. Before the fix, any `Result{is_error:true}` became a
//! terminal error and killed the session.

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

async fn drain_until<F: Fn(&AcpEvent) -> bool>(
    mgr: &SessionManager,
    id: &str,
    pred: F,
) -> AcpEvent {
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(3), mgr.next_event(id))
            .await
            .expect("event timeout")
            .expect("stream open");
        if pred(&evt) {
            return evt;
        }
    }
    panic!("no matching event within 10 reads");
}

#[tokio::test]
async fn refusal_is_non_terminal_and_session_survives() {
    let mgr = SessionManager::new(Duration::from_secs(30));
    let id = mgr
        .create_session(fake().to_str().unwrap(), vec![], None, vec![], "".into())
        .await
        .expect("spawn");

    // First turn: the agent refuses. Expect a NON-terminal error.
    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "refuse".into(),
        },
    )
    .await
    .expect("refusal prompt");
    let evt = drain_until(&mgr, &id, |e| matches!(e, AcpEvent::Error { .. })).await;
    match evt {
        AcpEvent::Error {
            terminal, message, ..
        } => {
            assert!(
                !terminal,
                "refusal must be non-terminal (session survives), got: {message}"
            );
            assert!(
                message.contains("refusal") || message.contains("refused"),
                "expected refusal text, got: {message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }

    // The session must still be usable: a second prompt gets answered.
    mgr.send(
        &id,
        AcpCommand::ContinueSession {
            session_id: id.clone(),
            prompt: "again".into(),
        },
    )
    .await
    .expect("follow-up prompt");
    let evt = drain_until(&mgr, &id, |e| matches!(e, AcpEvent::Finished { .. })).await;
    assert!(
        matches!(evt, AcpEvent::Finished { .. }),
        "session survived the refusal"
    );
}
