#!/usr/bin/env rust
//! Test binary: speaks Claude Code's stream-json protocol on stdio.
//! Ignores stdin (acts like Claude Code would on `--print` mode).
//!
//! argv[1] = scenario (hello / bash / deny)
//! argv[2..] = optional flags:
//!   --slow-ms <N>   sleep N ms between text_delta and result so callers
//!                   using the prod 150ms-debounced pump can observe the
//!                   transient 🚧 state before Finished consumes it.

use std::io::{self, BufRead, Write};
use std::time::Duration;

fn emit(line: &str) {
    println!("{line}");
    io::stdout().flush().unwrap();
}

fn parse_flag(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).cloned();
        }
        i += 1;
    }
    None
}

fn run_scenario(scenario: &str, slow_ms: u64, turn: usize) {
    let suffix = if turn > 1 {
        format!(" (turn #{turn})")
    } else {
        String::new()
    };
    match scenario {
        "hello" => {
            emit(&format!(
                r#"{{"type":"stream_event","event":{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"hello from fake claude{suffix}"}}}}}}"#
            ));
            if slow_ms > 0 {
                std::thread::sleep(Duration::from_millis(slow_ms));
            }
            emit(r#"{"type":"result","subtype":"success","stop_reason":"end_turn","is_error":false,"duration_ms":1,"result":"","session_id":"fake-1"}"#);
        }
        "bash" => {
            emit(&format!(
                r#"{{"type":"stream_event","event":{{"type":"content_block_start","index":1,"content_block":{{"type":"tool_use","id":"toolu_01","name":"Bash","input":{{"command":"echo hi{suffix}"}}}}}}}}"#
            ));
            if slow_ms > 0 {
                std::thread::sleep(Duration::from_millis(slow_ms));
            }
            emit(r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"hi\n","is_error":false}]}}"#);
            emit(r#"{"type":"result","subtype":"success","stop_reason":"end_turn","is_error":false,"duration_ms":1,"result":"","session_id":"fake-1"}"#);
        }
        "deny" => {
            emit(r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_02","name":"Bash","input":{"command":"rm -rf /"}}}}"#);
            if slow_ms > 0 {
                std::thread::sleep(Duration::from_millis(slow_ms));
            }
            emit(r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_02","content":"denied","is_error":true}]}}"#);
            emit(r#"{"type":"result","subtype":"success","stop_reason":"end_turn","is_error":false,"duration_ms":1,"result":"","session_id":"fake-1"}"#);
        }
        other => {
            eprintln!("unknown scenario: {other}");
            std::process::exit(2);
        }
    }
}

fn main() {
    let all_args: Vec<String> = std::env::args().collect();
    let scenario = all_args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "hello".to_string());
    let slow_ms = parse_flag(&all_args[2..], "--slow-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let loop_mode = all_args[2..].iter().any(|a| a == "--loop");

    // Always emit init first so the bridge knows we're alive.
    emit(r#"{"type":"system","subtype":"init","session_id":"fake-1","model":"fake","tools":[{"name":"Bash"},{"name":"Read"}]}"#);

    if !loop_mode {
        run_scenario(&scenario, slow_ms, 1);
        // Drain stdin so the parent doesn't block on close.
        let _ = io::stdin().lock().read_line(&mut String::new());
        return;
    }

    // --loop: emit scenario outputs for the initial session/new (turn 1)
    // and then for every subsequent user message read from stdin.
    run_scenario(&scenario, slow_ms, 1);

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    while let Some(Ok(line)) = lines.next() {
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("user") {
            // Each user prompt → one scenario worth of output.
            // Loop index starts at 2 for the first user message.
            // (We don't track an index — just emit forever.)
            run_scenario(&scenario, slow_ms, /*turn*/ 0);
        }
    }
}