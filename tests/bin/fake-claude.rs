//! Fake claude CLI for acp-claude's post-ACP engine (sebas-dk8.2).
//! Speaks the Claude Code stream-json + control-protocol dialect that
//! cc-agent-sdk drives:
//!   stdin  ← control_request initialize / user messages / control_response
//!   stdout → system init, assistant/user frames, result, control frames
//!
//! Fidelity contract (mirrors the real CLI, verified by wire fixtures in
//! spikes/cc-agent-sdk-spike/out/wire-frames.jsonl):
//! - `{cli} --version` prints a version line and exits 0 (SDK checks this).
//! - `control_request{subtype:initialize}` gets a `control_response` with
//!   `response.subtype == "success"`.
//! - The FIRST `user` message is preceded by exactly one `system/init` frame
//!   carrying the --session-id (or --resume) value from argv and our cwd.
//! - A tool_use turn sends `control_request{subtype:hook_callback}` and
//!   BLOCKS until the matching `control_response` arrives; `allow` runs the
//!   tool (tool_result success), `deny` fails it (tool_result is_error).
//! - `control_request{subtype:interrupt}` ends the turn with an error result
//!   and the process EXITS (the real CLI is unusable after interrupt).
//!
//! Flags (argv, not env — env races under cargo test parallelism):
//!   fake-claude-cli [scenario] [--loop] [--slow-ms N] [--hang-on-init]
//!                   [--delay-init-ms N] [--journal PATH] [--resume-fails]
//!   scenario: hello (default) | bash | deny | thinking
//!   --resume-fails: exit(1) with "No conversation found" on stderr, but ONLY
//!   when argv carries --resume — a fresh --session-id spawn still works
//!   (mirrors the real CLI; exercises the manager's fresh-session fallback).
//!
//! Content-triggered behaviors (for regression tests; take precedence over
//! the argv scenario):
//! - user text containing "crash" → emit one "boom" text frame then
//!   exit(1), modelling a mid-session process crash (D6).
//! - user text == "perm" → Bash tool_use + hook_callback; allow →
//!   tool_result "perm done", deny → tool_result is_error.
//! - user text == "stream" → 5 text frames with a 250ms pause before the
//!   result frame (exercises the 150ms-debounce pump's transient states).

use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

struct Flags {
    scenario: String,
    loop_mode: bool,
    slow_ms: u64,
    hang_on_init: bool,
    delay_init_ms: u64,
    journal: Option<String>,
    resume_fails: bool,
    /// True when argv carried `--resume <id>` (as opposed to `--session-id`)
    /// — resume rejection only applies to actual resume attempts, so the
    /// manager's fresh-session fallback still spawns fine.
    resume_used: bool,
    session_id: String,
}

const SCENARIOS: &[&str] = &["hello", "bash", "deny", "thinking"];

/// Flags that consume the NEXT argv token as their value (the SDK passes
/// many; anything not listed here and starting with `--` is treated as a
/// boolean switch and ignored). Positional tokens only become the scenario
/// if they name a known scenario — SDK-injected positionals must not be
/// mistaken for it.
const VALUE_FLAGS: &[&str] = &[
    "--slow-ms",
    "--delay-init-ms",
    "--journal",
    "--session-id",
    "--resume",
    "--output-format",
    "--input-format",
    "--permission-prompt-tool",
    "--model",
    "--fallback-model",
    "--permission-mode",
    "--setting-sources",
    "--mcp-config",
    "--append-system-prompt",
    "--system-prompt",
    "--max-turns",
    "--cwd",
    "--scenario",
];

fn parse_flags() -> Flags {
    let mut f = Flags {
        scenario: "hello".into(),
        loop_mode: false,
        slow_ms: 0,
        hang_on_init: false,
        delay_init_ms: 0,
        journal: None,
        resume_fails: false,
        resume_used: false,
        session_id: "fake-1".into(),
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--loop" => f.loop_mode = true,
            "--hang-on-init" => f.hang_on_init = true,
            "--resume-fails" => f.resume_fails = true,
            "--delay-new-ms" => {
                // Compat alias for the ACP-era flag: slow session/new ≈
                // slow initialize handshake in the new dialect.
                f.delay_init_ms = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--enable-load" => {} // no-op: resume always "works" in the new dialect
            "--load-fails" => f.resume_fails = true,
            "--slow-ms" => {
                f.slow_ms = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--delay-init-ms" => {
                f.delay_init_ms = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--journal" => {
                f.journal = args.get(i + 1).cloned();
                i += 1;
            }
            "--scenario" => {
                // Keyed form: survives the SDK's extra_args map (positionals
                // cannot be expressed there).
                if let Some(v) = args.get(i + 1) {
                    f.scenario = v.clone();
                }
                i += 1;
            }
            "--session-id" => {
                if let Some(v) = args.get(i + 1) {
                    f.session_id = v.clone();
                }
                i += 1;
            }
            "--resume" => {
                f.resume_used = true;
                if let Some(v) = args.get(i + 1) {
                    f.session_id = v.clone();
                }
                i += 1;
            }
            s if VALUE_FLAGS.contains(&s) => {
                i += 1; // consume the value, ignore it
            }
            s if SCENARIOS.contains(&s) => f.scenario = s.to_string(),
            _ => {} // boolean switch or unknown positional: ignore
        }
        i += 1;
    }
    f
}

struct Io {
    out: io::StdoutLock<'static>,
    journal: Option<std::fs::File>,
}

impl Io {
    fn emit(&mut self, v: &Value) {
        let line = serde_json::to_string(v).unwrap();
        writeln!(self.out, "{line}").unwrap();
        self.out.flush().unwrap();
        self.journal_write("out", v);
    }
    fn journal_write(&mut self, dir: &str, v: &Value) {
        if let Some(j) = self.journal.as_mut() {
            let line = serde_json::to_string(&json!({"dir": dir, "msg": v})).unwrap();
            let _ = writeln!(j, "{line}");
            let _ = j.flush();
        }
    }
}

fn main() {
    // The SDK runs `{cli} --version` before spawning (transport check).
    if std::env::args().any(|a| a == "--version") {
        println!("2.1.206 (fake-claude-cli)");
        return;
    }
    let flags = parse_flags();
    // Like the real CLI, only an actual `--resume` of an unknown id is
    // rejected; a fresh `--session-id` spawn with the same flags works.
    if flags.resume_fails && flags.resume_used {
        eprintln!("Error: No conversation found with session ID");
        std::process::exit(1);
    }

    let journal = flags.journal.as_ref().map(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .expect("open journal")
    });
    let mut io = Io {
        out: Box::leak(Box::new(io::stdout())).lock(),
        journal,
    };
    // Diagnostic: record the full argv so tests can assert flag plumbing.
    io.journal_write(
        "meta",
        &json!({"argv": std::env::args().skip(1).collect::<Vec<_>>()}),
    );

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines().map_while(Result::ok);
    let mut init_sent = false;
    let mut hook_counter: u64 = 0;

    while let Some(line) = lines.next() {
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        io.journal_write("in", &v);
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "control_request" => {
                let req_id = v
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let subtype = v
                    .pointer("/request/subtype")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match subtype {
                    "initialize" => {
                        if flags.hang_on_init {
                            continue; // never answer → caller's startup timeout fires
                        }
                        if flags.delay_init_ms > 0 {
                            std::thread::sleep(std::time::Duration::from_millis(
                                flags.delay_init_ms,
                            ));
                        }
                        io.emit(&json!({
                            "type": "control_response",
                            "response": {"subtype": "success", "request_id": req_id, "response": {}}
                        }));
                    }
                    "interrupt" => {
                        // Real CLI: ack the control request, end the turn with
                        // an error result, then EXIT (post-interrupt client
                        // is unusable — spike §S6).
                        io.emit(&json!({
                            "type": "control_response",
                            "response": {"subtype": "success", "request_id": req_id, "response": {}}
                        }));
                        io.emit(&result_frame(
                            &flags.session_id,
                            "error_during_execution",
                            true,
                        ));
                        io.out.flush().unwrap();
                        std::process::exit(1);
                    }
                    _ => {
                        // set_model / set_permission_mode / ... : ack and ignore.
                        io.emit(&json!({
                            "type": "control_response",
                            "response": {"subtype": "success", "request_id": req_id, "response": {}}
                        }));
                    }
                }
            }
            "user" => {
                let text = user_text(&v);
                if text.contains("crash") {
                    // D6: mid-session process crash — one last frame, then die.
                    io.emit(&assistant_text(&flags.session_id, "boom"));
                    io.out.flush().unwrap();
                    std::process::exit(1);
                }
                if !init_sent {
                    init_sent = true;
                    let cwd = std::env::current_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    io.emit(&json!({
                        "type": "system", "subtype": "init",
                        "session_id": flags.session_id, "model": "fake",
                        "cwd": cwd, "tools": ["Bash", "Read"],
                    }));
                }
                if text == "perm" {
                    perm_turn(&flags, &mut io, &mut lines, &mut hook_counter);
                } else if text == "stream" {
                    stream_turn(&flags, &mut io);
                } else {
                    run_scenario(&flags, &mut io, &mut lines, &mut hook_counter);
                }
                // Like the real CLI in streaming mode, stay alive for further
                // user messages until stdin closes (multi-turn).
            }
            _ => {} // control_response to nothing we asked: ignore
        }
    }
}

/// "perm" prompt: Bash(rm -rf /) gated by the hook; the tool_result reflects
/// the decision, then the turn ends.
fn perm_turn(
    flags: &Flags,
    io: &mut Io,
    lines: &mut dyn Iterator<Item = String>,
    hook_counter: &mut u64,
) {
    let sid = &flags.session_id;
    io.emit(&json!({
        "type": "assistant",
        "session_id": sid,
        "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "tc-1", "name": "Bash", "input": {"command": "rm -rf /"}}
        ]}
    }));
    *hook_counter += 1;
    let req_id = format!("fake-hook-{}", *hook_counter);
    io.emit(&json!({
        "type": "control_request",
        "request_id": req_id,
        "request": {
            "subtype": "hook_callback",
            "callback_id": "hook_0",
            "tool_use_id": "tc-1",
            "input": {
                "hook_event_name": "PreToolUse",
                "session_id": sid,
                "tool_name": "Bash",
                "tool_input": {"command": "rm -rf /"},
                "cwd": "/tmp",
                "transcript_path": "/tmp/fake.jsonl"
            }
        }
    }));
    let decision = wait_hook_decision(lines, &req_id, io);
    if decision == "allow" {
        io.emit(&tool_result_frame(sid, "tc-1", "perm done\n", false));
    } else {
        io.emit(&tool_result_frame(sid, "tc-1", "denied by fake", true));
    }
    io.emit(&result_frame(sid, "success", false));
}

/// "stream" prompt: 5 text chunks, then a pause so the debounced pump can
/// flush a transient mid-turn card before Finished (mirrors the ACP-era
/// fake's "stream" trigger).
fn stream_turn(flags: &Flags, io: &mut Io) {
    let sid = &flags.session_id;
    for i in 0..5 {
        io.emit(&assistant_text(sid, &format!("chunk{i} ")));
    }
    // 800ms: the 150ms debounce tick must flush a transient 🚧 card well
    // before the result frame; SDK startup (version probe + spawn) adds
    // ~100ms of latency, so smaller margins flake under parallel test load.
    std::thread::sleep(std::time::Duration::from_millis(800));
    io.emit(&result_frame(sid, "success", false));
}

fn run_scenario(
    flags: &Flags,
    io: &mut Io,
    lines: &mut dyn Iterator<Item = String>,
    hook_counter: &mut u64,
) {
    let sid = &flags.session_id;
    let settle = || {
        // Slow-down knob: sleep BETWEEN the content frames and the result
        // frame so a debounced consumer observes the transient 🚧 state.
        if flags.slow_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(flags.slow_ms));
        }
    };
    match flags.scenario.as_str() {
        "hello" => {
            io.emit(&assistant_text(sid, "hello "));
            io.emit(&assistant_text(sid, "world"));
            settle();
            io.emit(&result_frame(sid, "success", false));
        }
        "thinking" => {
            io.emit(&json!({
                "type": "assistant",
                "session_id": sid,
                "message": {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "hmm"}
                ]}
            }));
            io.emit(&assistant_text(sid, "thought out loud"));
            settle();
            io.emit(&result_frame(sid, "success", false));
        }
        "bash" | "deny" => {
            let (tool_id, cmd) = if flags.scenario == "bash" {
                ("toolu_01", "echo hi")
            } else {
                ("toolu_02", "rm -rf /")
            };
            io.emit(&json!({
                "type": "assistant",
                "session_id": sid,
                "message": {"role": "assistant", "content": [
                    {"type": "tool_use", "id": tool_id, "name": "Bash", "input": {"command": cmd}}
                ]}
            }));
            // Permission gate: ask the SDK side via hook_callback and block.
            *hook_counter += 1;
            let req_id = format!("fake-hook-{}", *hook_counter);
            io.emit(&json!({
                "type": "control_request",
                "request_id": req_id,
                "request": {
                    "subtype": "hook_callback",
                    "callback_id": "hook_0",
                    "tool_use_id": tool_id,
                    "input": {
                        "hook_event_name": "PreToolUse",
                        "session_id": sid,
                        "tool_name": "Bash",
                        "tool_input": {"command": cmd},
                        "cwd": "/tmp",
                        "transcript_path": "/tmp/fake.jsonl"
                    }
                }
            }));
            let decision = wait_hook_decision(lines, &req_id, io);
            if decision == "allow" && flags.scenario == "bash" {
                io.emit(&tool_result_frame(sid, tool_id, "hi\n", false));
            } else {
                io.emit(&tool_result_frame(sid, tool_id, "denied by fake", true));
            }
            settle();
            io.emit(&result_frame(sid, "success", false));
        }
        other => {
            eprintln!("unknown scenario: {other}");
            std::process::exit(2);
        }
    }
}

/// Read stdin until the control_response for our hook_callback arrives;
/// extract permissionDecision (default deny — fail closed like the bridge).
/// Any control_request seen while waiting (e.g. the driver's watchdog
/// `set_model` probe) is acked inline so it doesn't consume our response
/// or starve the probe of its answer.
#[allow(clippy::while_let_on_iterator)] // &mut dyn Iterator: no by_ref (Sized)
fn wait_hook_decision(
    lines: &mut dyn Iterator<Item = String>,
    req_id: &str,
    io: &mut Io,
) -> String {
    while let Some(line) = lines.next() {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        io.journal_write("in", &v);
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        if ty == "control_request" {
            let other_id = v.get("request_id").and_then(Value::as_str).unwrap_or("");
            let subtype = v
                .pointer("/request/subtype")
                .and_then(Value::as_str)
                .unwrap_or("");
            if subtype == "interrupt" {
                // Not expected mid-hook, but honor the contract.
                io.emit(&json!({
                    "type": "control_response",
                    "response": {"subtype": "success", "request_id": other_id, "response": {}}
                }));
                io.out.flush().unwrap();
                std::process::exit(1);
            }
            io.emit(&json!({
                "type": "control_response",
                "response": {"subtype": "success", "request_id": other_id, "response": {}}
            }));
            continue;
        }
        if ty == "control_response"
            && v.pointer("/response/request_id").and_then(Value::as_str) == Some(req_id)
        {
            return v
                .pointer("/response/response/hookSpecificOutput/permissionDecision")
                .and_then(Value::as_str)
                .unwrap_or("deny")
                .to_string();
        }
    }
    "deny".into()
}

fn user_text(v: &Value) -> String {
    match v.pointer("/message/content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn assistant_text(sid: &str, text: &str) -> Value {
    json!({
        "type": "assistant",
        "session_id": sid,
        "message": {"role": "assistant", "content": [
            {"type": "text", "text": text}
        ]}
    })
}

fn tool_result_frame(sid: &str, tool_id: &str, content: &str, is_error: bool) -> Value {
    json!({
        "type": "user",
        "session_id": sid,
        "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": tool_id, "content": content, "is_error": is_error}
        ]}
    })
}

fn result_frame(sid: &str, subtype: &str, is_error: bool) -> Value {
    json!({
        "type": "result", "subtype": subtype, "is_error": is_error,
        "stop_reason": if is_error { serde_json::Value::Null } else { json!("end_turn") },
        "duration_ms": 1, "duration_api_ms": 1, "num_turns": 1, "result": "", "session_id": sid
    })
}
