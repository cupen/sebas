//! D1 regression: create_session must issue exactly ONE session/new, and the
//! id it returns must be the id the agent actually works on (updates carry
//! the same id). The pre-fix code sent session/new twice (manual + SDK
//! start_session), splitting the routing id from the working id.

use acp_claude::manager::SessionManager;
use acp_claude::session::{AcpCommand, AcpEvent};
use std::path::PathBuf;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude")
}

fn journal_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "fc-journal-{}-{tag}-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ))
}

fn journal_lines(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .expect("journal exists")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("journal line parses"))
        .collect()
}

fn count_method(lines: &[serde_json::Value], method: &str) -> usize {
    lines
        .iter()
        .filter(|l| l.pointer("/msg/method").and_then(|m| m.as_str()) == Some(method))
        .count()
}

#[tokio::test]
async fn exactly_one_session_new_and_routing_id_matches() {
    let journal = journal_path("single-new");
    let mgr = SessionManager::new();
    let id = mgr
        .create_session(
            fake().to_str().unwrap(),
            vec!["--journal".into(), journal.to_str().unwrap().into()],
            Some("/tmp".into()),
            "".into(),
        )
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
    .expect("send prompt");

    // Drain until Finished.
    for _ in 0..10 {
        let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
            .await
            .expect("event timeout")
            .expect("stream open");
        if matches!(evt, AcpEvent::Finished { .. }) {
            break;
        }
    }

    let lines = journal_lines(&journal);
    assert_eq!(
        count_method(&lines, "session/new"),
        1,
        "expected exactly one session/new, journal: {lines:?}"
    );
    // cwd plumbing: the one session/new must carry the work_dir we passed.
    let cwd = lines
        .iter()
        .find(|l| l.pointer("/msg/method").and_then(|m| m.as_str()) == Some("session/new"))
        .and_then(|l| l.pointer("/msg/params/cwd"))
        .and_then(|c| c.as_str());
    assert_eq!(cwd, Some("/tmp"), "session/new cwd mismatch");
    // Routing integrity: every session/update the agent sent must be tagged
    // with the SAME id create_session returned.
    let update_sids: Vec<&str> = lines
        .iter()
        .filter(|l| l.pointer("/msg/method").and_then(|m| m.as_str()) == Some("session/update"))
        .filter_map(|l| l.pointer("/msg/params/sessionId").and_then(|s| s.as_str()))
        .collect();
    assert!(!update_sids.is_empty(), "agent sent no updates");
    assert!(
        update_sids.iter().all(|s| *s == id.as_str()),
        "updates tagged with a different session id than the routing id {id}: {update_sids:?}"
    );
    let _ = std::fs::remove_file(&journal);
}
