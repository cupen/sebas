//! Integration test: spawn fake-stream-claude and read its events.
//!
//! Run with: cargo test -p acp-claude-bridge --test claude_driver -- --nocapture

use acp_claude_bridge::claude::{parse_line, ClaudeDriver, StreamEvent};
use std::process::Command;
use std::time::Duration;
use tokio::time::timeout;

fn fake_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // out of acp-claude-bridge/
    p.push("target/debug/fake-stream-claude");
    if !p.exists() {
        // cargo test runs may build into target/debug/<workspace_root>; fall back to workspace target
        let mut alt = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        alt.push("target/debug/fake-stream-claude");
        if alt.exists() {
            return alt;
        }
        // Build it now
        let status = Command::new("cargo")
            .args(["build", "-p", "acp-claude-bridge", "--bin", "fake-stream-claude"])
            .status()
            .expect("cargo build");
        assert!(status.success(), "fake-stream-claude did not build");
    }
    p
}

#[tokio::test]
async fn reads_init_and_turn_end_from_hello_scenario() {
    let bin = fake_path();
    let mut drv = ClaudeDriver::spawn(bin.to_str().unwrap(), &["hello"])
        .await
        .expect("spawn fake");

    let first = timeout(Duration::from_secs(5), drv.next_event())
        .await
        .expect("timeout on first event")
        .expect("stream closed early");
    assert!(matches!(first, StreamEvent::System { .. }), "got {first:?}");

    let second = timeout(Duration::from_secs(5), drv.next_event())
        .await
        .expect("timeout")
        .expect("closed");
    assert!(matches!(second, StreamEvent::TextDelta { .. }), "got {second:?}");

    let third = timeout(Duration::from_secs(5), drv.next_event())
        .await
        .expect("timeout")
        .expect("closed");
    assert!(matches!(third, StreamEvent::TurnEnd { .. }), "got {third:?}");
}

#[test]
fn parses_a_text_delta_line_directly() {
    let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"x"}}}"#;
    let ev = parse_line(line)
        .unwrap()
        .into_iter()
        .next()
        .expect("expected one event");
    assert_eq!(ev, StreamEvent::TextDelta { text: "x".into() });
}
