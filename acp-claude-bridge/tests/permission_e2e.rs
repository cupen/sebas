//! End-to-end regression: hello scenario 跑通，证明 Task 2 的 pump loop
//! 改造没破 hello 路径。真权限通路覆盖见 Task 1 unit tests + 手动集成。
//!
//! Run: cargo test -p acp-claude-bridge --test permission_e2e -- --nocapture

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

fn bridge_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.push("target/debug/claude-acp-bridge");
    p
}

fn fake_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.push("target/debug/fake-stream-claude");
    p
}

async fn drive_until_contains<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
    needle: &str,
    deadline: Duration,
) -> String {
    let mut buf = String::new();
    let started = std::time::Instant::now();
    while started.elapsed() < deadline {
        buf.clear();
        let fut = reader.read_line(&mut buf);
        match timeout(Duration::from_secs(2), fut).await {
            Ok(Ok(0)) => break,
            Ok(Ok(_)) => {
                if buf.contains(needle) {
                    return buf;
                }
            }
            Ok(Err(e)) => panic!("read error: {e}"),
            Err(_) => continue,
        }
    }
    panic!("never found {needle:?} within {deadline:?}; last line: {buf:?}");
}

#[tokio::test]
async fn hello_path_survives_tool_use_branch_refactor() {
    let mut child = TokioCommand::new(bridge_path())
        .env("SEBAS_CLAUDE_PATH", fake_path().to_str().unwrap())
        .args(&["hello"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn bridge");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // initialize
    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"hello-regression","version":"0"}}}"#,
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
    let init_line = drive_until_contains(&mut stdout, "agentCapabilities", Duration::from_secs(10)).await;
    assert!(init_line.contains("\"loadSession\":false"), "init: {init_line}");

    // initialized
    stdin
        .write_all(br#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#)
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    // session/new
    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#,
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
    let new_line = drive_until_contains(&mut stdout, "sessionId", Duration::from_secs(10)).await;
    let v: serde_json::Value = serde_json::from_str(&new_line).expect("session/new json");
    let session_id = v["result"]["sessionId"]
        .as_str()
        .expect("sessionId string")
        .to_string();

    // session/prompt
    let prompt_payload = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"hi"}}]}}}}"#
    );
    stdin.write_all(prompt_payload.as_bytes()).await.unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    // 期望：text delta notification（hello scenario 不 emit ToolUse，所以走原 translator 路径）
    let notif_line = drive_until_contains(
        &mut stdout,
        "hello from fake claude",
        Duration::from_secs(10),
    )
    .await;
    let nv: serde_json::Value = serde_json::from_str(&notif_line).expect("notification json");
    assert_eq!(nv["method"], "session/update", "notif: {notif_line}");
    assert_eq!(
        nv["params"]["update"]["sessionUpdate"], "agent_message_chunk",
        "update kind"
    );
    assert_eq!(nv["params"]["sessionId"], session_id, "sessionId tagged");

    // 期望：响应 stopReason=end_turn
    let resp_line = drive_until_contains(&mut stdout, "\"id\":3", Duration::from_secs(10)).await;
    let rv: serde_json::Value = serde_json::from_str(&resp_line).expect("response json");
    assert_eq!(rv["id"], 3, "id: {resp_line}");
    assert_eq!(
        rv["result"]["stopReason"], "end_turn",
        "stopReason: {resp_line}"
    );

    drop(stdin);
    drop(child);
}
