//! End-to-end: bridge 接到 session/prompt 后，把 fake-stream-claude 的 hello
//! scenario (text delta + result) 转成一条 session/update 通知 + stopReason=end_turn
//! 响应。
//!
//! Run: cargo test -p acp-claude-bridge --test bridge_prompt_e2e -- --nocapture

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

fn bridge_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // out of acp-claude-bridge/
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
            Err(_) => continue, // 单行 2s 超时但整体未到 deadline → 继续
        }
    }
    panic!("never found {needle:?} within {deadline:?}; last line: {buf:?}");
}

#[tokio::test]
async fn bridge_prompt_emits_text_delta_and_resolves_end_turn() {
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
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"prompt-e2e","version":"0"}}}"#,
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
    let init_line = drive_until_contains(&mut stdout, "agentCapabilities", Duration::from_secs(10)).await;
    assert!(init_line.contains("\"loadSession\":false"), "init: {init_line}");

    // initialized notification
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
    // 抓出 sessionId 用于 session/prompt
    let v: serde_json::Value = serde_json::from_str(&new_line).expect("session/new response json");
    let session_id = v["result"]["sessionId"]
        .as_str()
        .expect("sessionId string")
        .to_string();

    // session/prompt —— 把 sessionId 注入到 params.sessionId
    let prompt_payload = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"hi"}}]}}}}"#
    );
    stdin.write_all(prompt_payload.as_bytes()).await.unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    // 期望：1 条 session/update 通知（agent_message_chunk 含 "hello from fake claude"）
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
    assert_eq!(
        nv["params"]["update"]["content"]["text"], "hello from fake claude",
        "chunk text"
    );
    assert_eq!(nv["params"]["sessionId"], session_id, "sessionId tagged");

    // 期望：最终 id=3 的 session/prompt 响应，stopReason=end_turn
    // 因 stdout 同时混着通知和响应，按 id 字段匹配
    let resp_line = drive_until_contains(
        &mut stdout,
        "\"id\":3",
        Duration::from_secs(10),
    )
    .await;
    let rv: serde_json::Value = serde_json::from_str(&resp_line).expect("response json");
    assert_eq!(rv["id"], 3, "id: {resp_line}");
    assert_eq!(
        rv["result"]["stopReason"], "end_turn",
        "stopReason: {resp_line}"
    );

    drop(stdin);
    drop(child);
}
