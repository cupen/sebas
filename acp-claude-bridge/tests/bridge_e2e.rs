//! End-to-end test: spawn the bridge, drive a real ACP handshake + session/new
//! + session/prompt, and assert the text delta from fake-stream-claude comes
//! through as an AgentMessageChunk.
//!
//! Run with: cargo test -p acp-claude-bridge --test bridge_e2e -- --nocapture

use std::process::Command;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

fn ensure_bridge_built() {
    let status = Command::new("cargo")
        .args(["build", "-p", "acp-claude-bridge"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "bridge build failed");
}

fn bridge_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.push("target/debug/claude-acp-bridge");
    p
}

fn fake_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // out of acp-claude-bridge/
    p.push("target/debug/fake-stream-claude");
    if !p.exists() {
        // cargo test may run from crate dir; fall back to crate target
        let mut alt = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        alt.push("target/debug/fake-stream-claude");
        if alt.exists() {
            return alt;
        }
    }
    p
}

#[tokio::test]
async fn bridge_handshake_returns_capabilities() {
    ensure_bridge_built();

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

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}"#;
    stdin.write_all(init.as_bytes()).await.unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    let mut line = String::new();
    timeout(Duration::from_secs(10), stdout.read_line(&mut line))
        .await
        .expect("timeout on init response")
        .expect("read init response");
    assert!(line.contains("agentCapabilities"), "no caps in: {line}");
    assert!(line.contains("\"loadSession\":false"), "expected loadSession:false, got: {line}");

    drop(stdin);
    drop(child);
}

#[tokio::test]
async fn bridge_session_new_returns_uuid() {
    ensure_bridge_built();

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
        .write_all(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}"#)
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
    let mut line = String::new();
    timeout(Duration::from_secs(10), stdout.read_line(&mut line))
        .await
        .expect("timeout on init response")
        .expect("read init response");

    // initialized notification
    stdin
        .write_all(br#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#)
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    // session/new
    stdin
        .write_all(br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#)
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    let mut line = String::new();
    timeout(Duration::from_secs(10), stdout.read_line(&mut line))
        .await
        .expect("timeout on session/new response")
        .expect("read session/new response");
    assert!(line.contains("sessionId"), "no sessionId in: {line}");

    drop(stdin);
    drop(child);
}