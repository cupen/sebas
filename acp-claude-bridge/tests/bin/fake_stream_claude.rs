#!/usr/bin/env rust
//! Test binary: speaks Claude Code's stream-json protocol on stdio.
//! Ignores stdin (acts like Claude Code would on `--print` mode).

use std::io::{self, BufRead, Write};

fn emit(line: &str) {
    println!("{line}");
    io::stdout().flush().unwrap();
}

fn main() {
    let scenario = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "hello".to_string());

    // Always emit init first so the bridge knows we're alive.
    emit(r#"{"type":"system","subtype":"init","session_id":"fake-1","model":"fake","tools":[{"name":"Bash"},{"name":"Read"}]}"#);

    match scenario.as_str() {
        "hello" => {
            emit(r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello from fake claude"}}}"#);
            emit(r#"{"type":"result","subtype":"success","stop_reason":"end_turn","is_error":false,"duration_ms":1,"result":"","session_id":"fake-1"}"#);
        }
        "bash" => {
            emit(r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"echo hi"}}}}"#);
            emit(r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"hi\n","is_error":false}]}}"#);
            emit(r#"{"type":"result","subtype":"success","stop_reason":"end_turn","is_error":false,"duration_ms":1,"result":"","session_id":"fake-1"}"#);
        }
        "deny" => {
            emit(r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_02","name":"Bash","input":{"command":"rm -rf /"}}}}"#);
            emit(r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_02","content":"denied","is_error":true}]}}"#);
            emit(r#"{"type":"result","subtype":"success","stop_reason":"end_turn","is_error":false,"duration_ms":1,"result":"","session_id":"fake-1"}"#);
        }
        other => {
            eprintln!("unknown scenario: {other}");
            std::process::exit(2);
        }
    }

    // Drain stdin so the parent doesn't block on close.
    let _ = io::stdin().lock().read_line(&mut String::new());
}