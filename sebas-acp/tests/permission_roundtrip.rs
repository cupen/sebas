//! End-to-end permission chain (post-ACP): prompt → fake CLI tool_use →
//! control hook_callback → driver's PreToolUse callback →
//! AcpEvent::PermissionRequest (carrying the ROUTING session id) →
//! PermissionReply resolves the parked oneshot → hook returns allow →
//! tool_result flows → turn completes. Journal asserts the protocol facts.

use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent, Decision};
use std::path::PathBuf;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli")
}

#[tokio::test]
async fn permission_round_trip() {
    let journal = std::env::temp_dir().join(format!(
        "fc-journal-{}-perm-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mgr = SessionManager::new(Duration::from_secs(30));
    let id = mgr
        .create_session(
            fake().to_str().unwrap(),
            vec![
                "--scenario".into(),
                "bash".into(),
                "--journal".into(),
                journal.to_str().unwrap().into(),
            ],
            None,
            vec![],
            "".into(),
        )
        .await
        .expect("spawn");
    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "please run bash".into(),
        },
    )
    .await
    .expect("prompt perm");

    // Expect PermissionRequest carrying the routing id.
    let mut request_id = None;
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
            .await
            .expect("event timeout")
            .expect("stream open");
        match evt {
            AcpEvent::PermissionRequest {
                session_id,
                request_id: rid,
                tool_name,
                ..
            } => {
                assert_eq!(
                    session_id, id,
                    "permission event session id must equal the routing id"
                );
                assert_eq!(tool_name, "Bash");
                request_id = Some(rid);
                break;
            }
            _ => continue,
        }
    }
    let request_id = request_id.expect("no PermissionRequest within 10 events");

    // Reply allow; the turn must then complete with a ToolEnd + Finished.
    mgr.send(
        &id,
        AcpCommand::PermissionReply {
            session_id: id.clone(),
            request_id,
            decision: Decision::AllowOnce,
        },
    )
    .await
    .expect("reply");

    let mut saw_tool_end = false;
    let mut finished = false;
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
            .await
            .expect("event timeout")
            .expect("stream open");
        match evt {
            AcpEvent::ToolEnd {
                tool_name, result, ..
            } => {
                assert_eq!(tool_name, "Bash");
                assert!(result.contains("hi"), "tool result: {result}");
                saw_tool_end = true;
            }
            AcpEvent::Finished { .. } => {
                finished = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_tool_end, "no ToolEnd after permission allow");
    assert!(finished, "turn did not complete after permission reply");

    // Journal: the hook_callback control_response we sent carried "allow".
    let raw = std::fs::read_to_string(&journal).expect("journal exists");
    let resp_line = raw
        .lines()
        .filter(|l| l.contains("\"dir\":\"in\""))
        .find(|l| l.contains("control_response") && l.contains("permissionDecision"))
        .expect("hook control_response in journal");
    assert!(
        resp_line.contains("allow"),
        "expected allow in hook response: {resp_line}"
    );
    let _ = std::fs::remove_file(&journal);
}
