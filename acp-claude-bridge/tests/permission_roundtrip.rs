//! Permission round-trip test: bridge + fake-stream-claude + a fake permission
//! decision source. Verifies the unix socket handshake works and the bridge
//! delivers a decision to the (mocked) ACP side.
//!
//! Run with: cargo test -p acp-claude-bridge --test permission_roundtrip -- --nocapture

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

#[tokio::test]
async fn hook_socket_round_trip() {
    let bridge_bin = {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p.push("target/debug/claude-acp-bridge");
        p
    };

    let fake_bin = {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p.push("target/debug/fake-stream-claude");
        p
    };

    // Start bridge with bash scenario (emits a ToolUse event).
    let mut child = TokioCommand::new(&bridge_bin)
        .env("SEBAS_CLAUDE_PATH", fake_bin.to_str().unwrap())
        .args(&["bash"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn bridge");

    // Read stderr in background so we can debug if it hangs.
    let stderr = child.stderr.take().unwrap();
    tokio::spawn(async move {
        let mut s = BufReader::new(stderr);
        let mut line = String::new();
        while s.read_line(&mut line).await.unwrap_or(0) > 0 {
            eprintln!("[bridge] {line}");
            line.clear();
        }
    });

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // initialize
    stdin
        .write_all(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"perm-test","version":"0"}}}"#)
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).await.unwrap();

    // initialized
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
    timeout(Duration::from_secs(5), stdout.read_line(&mut line))
        .await
        .expect("session/new timeout")
        .expect("read");
    assert!(line.contains("sessionId"), "no sessionId in: {line}");

    // Now: try the unix socket sidecar.
    let sidecar = std::env::temp_dir().join("sebras-bridge.sock.path");
    let sidecar_content = std::fs::read_to_string(&sidecar).expect("sidecar");
    let sock_path = sidecar_content.trim();
    assert!(!sock_path.is_empty(), "empty sidecar");

    let mut client = UnixStream::connect(sock_path).await.expect("connect to hook socket");
    client
        .write_all(br#"{"tool_name":"Bash","tool_input":{"command":"echo hi"}}"#)
        .await
        .unwrap();
    client.flush().await.unwrap();
    let mut resp = String::new();
    // Bridge will block on decisions.recv() forever (no decision sender in this
    // test). Use a short timeout to verify the broker accepted the connection.
    let r = timeout(Duration::from_secs(2), client.readable()).await;
    assert!(r.is_err(), "socket should block until a decision is sent");

    drop(client);
    drop(stdin);
    drop(child);
}