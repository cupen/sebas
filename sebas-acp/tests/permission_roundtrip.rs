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
    // CARGO_BIN_EXE 随当前构建的 target dir 解析（CARGO_TARGET_DIR 重定向下
    // 仍指本次构建的桩），且桩源与 workspace-root 的 `fake-claude` bin 共享。
    PathBuf::from(env!("CARGO_BIN_EXE_fake-claude-cli"))
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
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let id = mgr
        .create_claude_session(
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

/// fix-webui-qa-round6 1.2（permission-flow「第二轮审批不丢」）：同一会话两轮
/// `perm`——首轮经 PermissionReply 决策落定后，第二轮 `perm` 的 hook_callback
/// 必须再次产生 `AcpEvent::PermissionRequest` 并可经同一 Reply 路径解锁。
/// 回归钉：第二轮 hook_callback 悬空（SDK 不应答 control_request、决策面
/// 空表、回合永久 Waiting）在这里当场爆——本测试只钉 driver 层（无 pump、
/// 无 dispatch），进程级 journey 在 tests/testsuite_e2e_test.rs。
#[tokio::test]
async fn second_perm_turn_parks_again_and_completes() {
    let journal = std::env::temp_dir().join(format!(
        "fc-journal-{}-perm2-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let id = mgr
        .create_claude_session(
            fake().to_str().unwrap(),
            vec![
                "--scenario".into(),
                "empty".into(),
                "--journal".into(),
                journal.to_str().unwrap().into(),
            ],
            None,
            vec![],
            "".into(),
        )
        .await
        .expect("spawn");

    let mut first_request_id: Option<String> = None;
    for turn in 1..=2 {
        mgr.send(
            &id,
            AcpCommand::ContinueSession {
                session_id: id.clone(),
                prompt: "perm".into(),
            },
        )
        .await
        .unwrap_or_else(|e| panic!("turn {turn} prompt: {e}"));

        // 待批登记：PermissionRequest 携带路由 id 与 Bash 工具（`perm` 触发词
        // 的桩行为，优先于 --scenario）。核心断言（fix-webui-qa-round6 1.2）：
        // 第二轮的 request_id 必须与第一轮**不同**——tool_use_id 复用（桩每轮
        // 都是 tc-1）不得让两轮的待批在共享映射/已决墓碑里同 key。
        let mut request_id = None;
        for _ in 0..30 {
            let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
                .await
                .unwrap_or_else(|_| panic!("turn {turn}: timeout waiting for PermissionRequest"))
                .expect("event stream open");
            if let AcpEvent::PermissionRequest {
                session_id,
                request_id: rid,
                tool_name,
                ..
            } = evt
            {
                assert_eq!(session_id, id, "routing id must ride the event");
                assert_eq!(tool_name, "Bash");
                request_id = Some(rid);
                break;
            }
        }
        let request_id = request_id
            .unwrap_or_else(|| panic!("turn {turn}: no PermissionRequest within the event budget"));
        if let Some(first) = &first_request_id {
            assert_ne!(
                &request_id, first,
                "each turn's hook invocation must carry its own request_id"
            );
        } else {
            first_request_id = Some(request_id.clone());
        }

        mgr.send(
            &id,
            AcpCommand::PermissionReply {
                session_id: id.clone(),
                request_id,
                decision: Decision::AllowOnce,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("turn {turn} reply: {e}"));

        // 回合收尾：allow 后 tool_result（perm done）+ Finished。
        let mut saw_tool_end = false;
        let mut finished = false;
        for _ in 0..30 {
            let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
                .await
                .unwrap_or_else(|_| panic!("turn {turn}: timeout waiting for turn close"))
                .expect("event stream open");
            match evt {
                AcpEvent::ToolEnd { result, .. } => {
                    assert!(result.contains("perm done"), "turn {turn} result: {result}");
                    saw_tool_end = true;
                }
                AcpEvent::Finished { .. } => {
                    finished = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_tool_end, "turn {turn}: no ToolEnd after the allow");
        assert!(finished, "turn {turn}: turn did not complete after the reply");
    }
    let _ = std::fs::remove_file(&journal);
}
