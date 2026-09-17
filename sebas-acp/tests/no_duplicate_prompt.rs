//! Regression test for the duplicate-prompt bug.
//!
//! After `create_session` + one `mgr.send(CreateSession { prompt })` the
//! agent must receive the user's prompt **exactly once**, and the
//! resulting event stream must contain the canonical "hello world" reply
//! in two `TextDelta` chunks (not four). This guards against the
//! pre-fix behaviour where `create_session` itself pushed the initial
//! prompt and `mgr.send(CreateSession)` pushed it again, producing
//! four text chunks for a single user message.

use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use std::path::PathBuf;
use std::time::Duration;

#[tokio::test]
async fn one_prompt_yields_exactly_three_events() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli");
    let id = mgr
        .create_claude_session(fake.to_str().unwrap(), vec![], None, vec![], "".into())
        .await
        .expect("spawn fake-claude");

    // The single, explicit prompt send. `create_session` above must not
    // have already sent it.
    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send create_session");

    // Drain the event stream with a 2s budget per call. The claude driver
    // advertises the (empty — the stub runs without `--advertise-commands`)
    // command table right after connect (session-slash-commands 1.2), and
    // the frame-observed ModelChanged (workbench-composer-input-polish 2.2:
    // init frame's model name) leads the turn — drain those first. The next
    // three events must be, in order: TextDelta "hello ", TextDelta "world",
    // Finished — and nothing else. If the prompt had been sent twice we
    // would see four TextDelta events (two "hello " pairs) before the
    // Finished.
    loop {
        let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
            .await
            .expect("timeout on leading event")
            .expect("leading event");
        match evt {
            AcpEvent::AvailableCommands { commands, .. } => {
                assert!(
                    commands.is_empty(),
                    "the bare stub advertises no commands, got {commands:?}"
                );
            }
            // init 帧观察出的 ModelChanged 是合法先导事件（2.2）。
            AcpEvent::ModelChanged { .. } => {}
            other => {
                assert!(
                    matches!(&other, AcpEvent::TextDelta { delta, .. } if delta == "hello "),
                    "first turn event must be TextDelta(\"hello \"), got {other:?}",
                );
                break;
            }
        }
    }
    let evt2 = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
        .await
        .expect("timeout on event 2")
        .expect("event 2");
    let evt3 = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
        .await
        .expect("timeout on event 3")
        .expect("event 3");

    assert!(
        matches!(&evt2, AcpEvent::TextDelta { delta, .. } if delta == "world"),
        "second event must be TextDelta(\"world\") — a duplicate-prompt bug would surface as a second \"hello \" here, got {evt2:?}",
    );
    assert!(
        matches!(evt3, AcpEvent::Finished { .. }),
        "third event must be Finished, got {evt3:?}",
    );

    // The session is kept alive after Finished (so the caller can
    // dispatch follow-up commands like /compact), so the stream does
    // not close. But no further TextDelta must ever arrive from a
    // single user prompt: if the prompt were sent twice we would
    // see a fourth event — either another TextDelta or another
    // Finished — within a short window. A 500ms quiet period is
    // plenty for the agent to re-reply if the duplicate path
    // is still live.
    let mut extra_text_count = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), mgr.next_event(&id)).await {
            Ok(Some(AcpEvent::TextDelta { .. })) => extra_text_count += 1,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    assert_eq!(
        extra_text_count, 0,
        "expected zero further TextDelta events — the prompt was sent more than once",
    );
}
