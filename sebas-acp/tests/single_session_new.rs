//! D1 regression (post-ACP rewrite): create_session must deliver exactly ONE
//! initialize handshake and the initial prompt must arrive as exactly ONE
//! user message; the routing id the manager returns must be the id the fake
//! CLI works on (every frame it emits carries the same session_id).

use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use std::path::PathBuf;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli")
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

#[tokio::test]
async fn exactly_one_initialize_one_user_message_and_routing_id_matches() {
    let journal = journal_path("single-new");
    // 工作目录用 tempdir：/tmp 是 Unix 硬编码，Windows 下会被解析成
    // 当前盘符的 C:\tmp，导致 cwd 断言必然失败（child cwd mismatch）。
    let work_dir = tempfile::tempdir().expect("work dir");
    let work_dir_str = work_dir.path().to_string_lossy().into_owned();
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let id = mgr
        .create_claude_session(
            fake().to_str().unwrap(),
            vec!["--journal".into(), journal.to_str().unwrap().into()],
            Some(work_dir_str.clone()),
            vec![],
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
    let inbound: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|l| l.get("dir").and_then(|d| d.as_str()) == Some("in"))
        .filter_map(|l| l.get("msg"))
        .collect();

    // Exactly one initialize control_request…
    let inits = inbound
        .iter()
        .filter(|m| {
            m.get("type").and_then(|t| t.as_str()) == Some("control_request")
                && m.pointer("/request/subtype").and_then(|s| s.as_str()) == Some("initialize")
        })
        .count();
    assert_eq!(
        inits, 1,
        "expected exactly one initialize, journal: {lines:?}"
    );

    // …and exactly one user message (the prompt, delivered once).
    let user_msgs = inbound
        .iter()
        .filter(|m| m.get("type").and_then(|t| t.as_str()) == Some("user"))
        .count();
    assert_eq!(
        user_msgs, 1,
        "prompt must be delivered exactly once: {lines:?}"
    );

    // cwd plumbing: the fake reports its process cwd in the init frame.
    let cwd = lines
        .iter()
        .filter(|l| l.get("dir").and_then(|d| d.as_str()) == Some("out"))
        .filter_map(|l| l.get("msg"))
        .find(|m| {
            m.get("type").and_then(|t| t.as_str()) == Some("system")
                && m.get("subtype").and_then(|s| s.as_str()) == Some("init")
        })
        .and_then(|m| m.get("cwd"))
        .and_then(|c| c.as_str())
        .map(str::to_owned);
    assert_eq!(
        cwd.as_deref(),
        Some(work_dir_str.as_str()),
        "child cwd mismatch"
    );

    // Routing integrity: every assistant frame the agent sent must be tagged
    // with the SAME id create_session returned (the fake echoes --session-id).
    let frame_sids: Vec<&str> = lines
        .iter()
        .filter(|l| l.get("dir").and_then(|d| d.as_str()) == Some("out"))
        .filter_map(|l| l.get("msg"))
        .filter(|m| m.get("type").and_then(|t| t.as_str()) == Some("assistant"))
        .filter_map(|m| m.get("session_id").and_then(|s| s.as_str()))
        .collect();
    assert!(!frame_sids.is_empty(), "agent sent no assistant frames");
    assert!(
        frame_sids.iter().all(|s| *s == id.as_str()),
        "frames tagged with a different session id than the routing id {id}: {frame_sids:?}"
    );
    let _ = std::fs::remove_file(&journal);
}
