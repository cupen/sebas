//! Generic-ACP command discovery integration tests
//! (openspec/changes/session-slash-commands tasks 1.3/1.4).
//!
//! These drive the *generic ACP driver* (`AcpDriver`) against the
//! programmable `fake-acp-agent` mock with `--commands "goal:<hint>,compact"`:
//! the mock sends `session/update` notifications carrying
//! `available_commands_update` right after `session/new` / `session/load` and
//! again on every prompt (re-advertisement). The driver must surface each
//! notification as an `AcpEvent::AvailableCommands` stamped with the routing
//! id — no dedup, no dropped second notification (the engine overwrites the
//! old table per event).

use sebas_acp::claude::manager::SessionManager;
use sebas_acp::session::{AcpCommand, AcpEvent};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake_acp() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-acp-agent")
}

/// Build a manager whose sole registered agent is the generic ACP driver
/// bound to the mock binary with the given startup timeout.
fn acp_manager(startup_timeout: Duration) -> SessionManager {
    let mut agents = HashMap::new();
    agents.insert(
        "acp".to_string(),
        sebas_acp::claude::manager::AgentEntry {
            driver: Arc::new(sebas_acp::AcpDriver),
            startup_timeout,
        },
    );
    SessionManager::new("acp".to_string(), agents)
}

fn mock_args(scenario: &str, extra: &[&str]) -> Vec<String> {
    let mut args = vec![
        fake_acp().to_string_lossy().into_owned(),
        scenario.to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    args
}

/// Drain events until the next `AvailableCommands` arrives, returning every
/// event seen along the way (so tests can also assert what was NOT there).
async fn next_commands_event(
    mgr: &SessionManager,
    id: &str,
) -> (Vec<AcpEvent>, Vec<sebas_acp::session::AvailableCommand>) {
    let mut seen = Vec::new();
    for _ in 0..30 {
        let evt = tokio::time::timeout(Duration::from_secs(5), mgr.next_event(id))
            .await
            .expect("timeout waiting for event")
            .expect("event stream closed");
        if let AcpEvent::AvailableCommands { commands, .. } = &evt {
            return (seen, commands.clone());
        }
        seen.push(evt);
    }
    panic!("no AvailableCommands within 30 events");
}

/// session-slash-commands 1.3: the mock's `available_commands_update` after
/// `session/new` surfaces as `AvailableCommands` with name/description/hint
/// mapped and the ROUTING id stamped.
#[tokio::test]
async fn acp_commands_advertisement_reaches_the_driver() {
    let mgr = acp_manager(Duration::from_secs(10));
    let sid = mgr
        .create_session(
            "acp",
            mock_args("load-ok", &["--commands", "goal:<condition>,compact"]),
            None,
            vec![],
            "".into(),
        )
        .await
        .expect("fresh spawn must succeed");

    let (_, commands) = next_commands_event(&mgr, &sid).await;
    assert_eq!(commands.len(), 2, "both advertised entries arrive");
    assert_eq!(commands[0].name, "goal");
    assert_eq!(
        commands[0].hint.as_deref(),
        Some("<condition>"),
        "unstructured input hint maps to the command hint"
    );
    assert_eq!(commands[1].name, "compact");
    assert_eq!(commands[1].hint, None, "no input spec → hint None");

    mgr.kill(&sid).await;
}

/// session-slash-commands 1.4: the second `available_commands_update` (the
/// mock re-advertises on every prompt) also reaches sebas — the driver does
/// not dedup or drop re-advertisement, so the engine can overwrite the old
/// table.
#[tokio::test]
async fn acp_second_commands_update_arrives_on_re_advertisement() {
    let mgr = acp_manager(Duration::from_secs(10));
    let sid = mgr
        .create_session(
            "acp",
            mock_args("load-ok", &["--commands", "goal:<condition>,compact"]),
            None,
            vec![],
            "".into(),
        )
        .await
        .expect("fresh spawn must succeed");

    // First advertisement (post session/new).
    let (_, first) = next_commands_event(&mgr, &sid).await;
    assert_eq!(first.len(), 2);

    // A prompt makes the mock re-advertise before echoing.
    mgr.send(
        &sid,
        AcpCommand::ContinueSession {
            session_id: sid.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send prompt");
    let (_, second) = next_commands_event(&mgr, &sid).await;
    assert_eq!(
        second.len(),
        2,
        "the re-advertised table must arrive in full (overwrite source)"
    );

    mgr.kill(&sid).await;
}

/// Without `--commands` the mock advertises nothing: the driver emits no
/// `AvailableCommands` (echo text flows with no command event interleaved).
#[tokio::test]
async fn acp_without_commands_flag_no_command_event_is_emitted() {
    let mgr = acp_manager(Duration::from_secs(10));
    let sid = mgr
        .create_session("acp", mock_args("load-ok", &[]), None, vec![], "".into())
        .await
        .expect("fresh spawn must succeed");

    mgr.send(
        &sid,
        AcpCommand::ContinueSession {
            session_id: sid.clone(),
            prompt: "hi".into(),
        },
    )
    .await
    .expect("send prompt");

    // Drain up to and including the echo TextDelta; assert nothing along the
    // way was a command advertisement.
    for _ in 0..20 {
        let evt = tokio::time::timeout(Duration::from_secs(5), mgr.next_event(&sid))
            .await
            .expect("timeout waiting for echo")
            .expect("event stream closed");
        match &evt {
            AcpEvent::AvailableCommands { .. } => {
                panic!("no command advertisement was expected, got {evt:?}")
            }
            AcpEvent::TextDelta { delta, .. } if delta.contains("echo:") => return,
            _ => {}
        }
    }
    panic!("echo TextDelta never arrived");
}
