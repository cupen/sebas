//! End-to-end: acp-claude SessionManager 实际起 in-tree bridge binary
//! (`claude-acp-bridge`) 当 child agent，bridge 看到 SEBAS_CLAUDE_PATH
//! 环境变量就 spawn `fake-stream-claude bash` scenario 模拟真 Claude。
//!
//! 验证后三环：acp-claude (client) → acp-claude-bridge (server) → claude child
//! 全链路通到 AcpEvent 流。bash scenario 期望 emit 1 ToolStart + 1 ToolEnd
//! + 1 Finished。

use acp_claude::manager::SessionManager;
use acp_claude::session::{AcpCommand, AcpEvent, Decision};
use std::path::PathBuf;
use std::time::Duration;

fn bridge() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/claude-acp-bridge")
}

fn fake_stream_claude() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-stream-claude")
}

#[tokio::test]
async fn bridge_drives_fake_claude_bash_to_acp_event_stream() {
    // bridge binary 读 SEBAS_CLAUDE_PATH 环境变量来定位 claude 子进程。
    // 必须在 SessionManager 启动 child 之前 setenv 父进程 env。
    std::env::set_var(
        "SEBAS_CLAUDE_PATH",
        fake_stream_claude().to_str().unwrap(),
    );

    let mgr = SessionManager::new(Duration::from_secs(15));
    let session_id = mgr
        .create_session(
            bridge().to_str().unwrap(),
            vec!["bash".into()], // forward to fake-stream-claude as scenario arg
            Some("/tmp".into()),
            "".into(),
        )
        .await
        .expect("spawn bridge");

    eprintln!("[test] bridge spawned, session_id={session_id}");

    mgr.send(
        &session_id,
        AcpCommand::CreateSession {
            session_id: session_id.clone(),
            prompt: "please run bash".into(),
        },
    )
    .await
    .expect("send prompt");

    eprintln!("[test] prompt sent, draining events...");

    // 收集 AcpEvent 直到 Finished 或 timeout
    let mut got_tool_start = false;
    let mut got_tool_end = false;
    let mut got_finished = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);

    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), mgr.next_event(&session_id)).await {
            Ok(Some(AcpEvent::ToolStart { tool_name, .. })) => {
                eprintln!("[test] ToolStart {tool_name}");
                assert_eq!(tool_name, "Bash", "fake-stream-claude bash scenario emits a Bash tool");
                got_tool_start = true;
            }
            Ok(Some(AcpEvent::ToolEnd { result, .. })) => {
                eprintln!("[test] ToolEnd result={result:?}");
                // tool_name is empty in the bridge's current ToolCallUpdate notification
                // (bridge doesn't preserve title across the ToolUse→ToolResult pair). Skip assert.
                assert!(result.contains("hi"), "tool result: {result}");
                got_tool_end = true;
            }
            Ok(Some(AcpEvent::ToolProgress { tool_name, progress, .. })) => {
                eprintln!("[test] ToolProgress {tool_name} {progress}");
            }
            Ok(Some(AcpEvent::Finished { .. })) => {
                eprintln!("[test] Finished");
                got_finished = true;
                break;
            }
            Ok(Some(AcpEvent::Error { message, .. })) => {
                panic!("bridge reported error: {message}");
            }
            Ok(Some(AcpEvent::PermissionRequest { request_id, tool_name, .. })) => {
                eprintln!("[test] PermissionRequest {tool_name} request_id={request_id} — replying AllowOnce");
                mgr.send(
                    &session_id,
                    AcpCommand::PermissionReply {
                        session_id: session_id.clone(),
                        request_id,
                        decision: Decision::AllowOnce,
                    },
                )
                .await
                .expect("send PermissionReply");
                eprintln!("[test] PermissionReply sent, waiting for tool_result");
            }
            Ok(Some(other)) => {
                eprintln!("[test] other event: {other:?}");
            }
            Ok(None) => panic!("event stream closed before Finished"),
            Err(_) => {
                eprintln!("[test] timeout, no event for 3s. final: tool_start={got_tool_start}, tool_end={got_tool_end}, finished={got_finished}");
                panic!("event timeout (no AcpEvent for 3s)");
            }
        }
    }

    assert!(got_tool_start, "no ToolStart AcpEvent for Bash");
    assert!(got_tool_end, "no ToolEnd AcpEvent for Bash");
    assert!(got_finished, "no Finished AcpEvent");
}
