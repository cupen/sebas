//! End-to-end permission chain: prompt "perm" -> agent issues
//! session/request_permission -> AcpEvent::PermissionRequest (carrying the
//! ROUTING session id, not a split second id) -> PermissionReply reaches the
//! agent -> turn completes. Journal asserts the protocol facts.

use acp_claude::manager::SessionManager;
use acp_claude::session::{AcpCommand, AcpEvent, Decision};
use std::path::PathBuf;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude")
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
            vec!["--journal".into(), journal.to_str().unwrap().into()],
            None,
            "".into(),
        )
        .await
        .expect("spawn");
    mgr.send(
        &id,
        AcpCommand::CreateSession {
            session_id: id.clone(),
            prompt: "perm".into(),
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

    // Reply allow_once; the turn must then complete.
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

    let mut finished = false;
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
            .await
            .expect("event timeout")
            .expect("stream open");
        if matches!(evt, AcpEvent::Finished { .. }) {
            finished = true;
            break;
        }
    }
    assert!(finished, "turn did not complete after permission reply");

    // Journal: the permission RESPONSE we sent selected allow_once.
    let raw = std::fs::read_to_string(&journal).expect("journal exists");
    let resp_line = raw
        .lines()
        .find(|l| l.contains("\"perm-1\"") && l.contains("\"result\""))
        .expect("permission response in journal");
    assert!(
        resp_line.contains("allow_once"),
        "expected allow_once in permission response: {resp_line}"
    );
    let _ = std::fs::remove_file(&journal);
}
