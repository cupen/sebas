use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use std::path::PathBuf;
use std::time::Duration;

#[tokio::test]
async fn fake_claude_emits_finished() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    // `cargo test` runs from the package dir (sebas-acp/), so resolve the
    // workspace-root target/ via CARGO_MANIFEST_DIR's parent.
    let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli");
    let id = mgr
        .create_claude_session(fake.to_str().unwrap(), vec![], None, vec![], "".into())
        .await
        .expect("spawn fake-claude");
    // Send create_session command
    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send create_session");

    // Receive events with timeout. The claude driver now advertises the
    // session command table right after the initialize handshake
    // (session-slash-commands 1.2) — leading AvailableCommands events are
    // part of the contract; so is the frame-observed ModelChanged
    // (workbench-composer-input-polish 2.2: the system init frame's model
    // name overwrites the spawn-time current) — the turn content must follow
    // them.
    let mut saw_content = false;
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
            .await
            .expect("timeout")
            .expect("event");
        match evt {
            AcpEvent::AvailableCommands { .. } | AcpEvent::ModelChanged { .. } => continue,
            AcpEvent::TextDelta { .. } | AcpEvent::Finished { .. } => {
                saw_content = true;
                break;
            }
            other => panic!("unexpected leading event {other:?}"),
        }
    }
    assert!(saw_content, "turn content must arrive after the advertisement");
}
