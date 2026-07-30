//! Minimal-but-faithful ACP agent stand-in (design doc §6).
//!
//! Speaks ACP JSON-RPC on stdio without depending on the SDK.
//! Fidelity contract:
//! - `session/new` returns a GLOBALLY UNIQUE id per call (sess-1, sess-2, ...).
//! - Update notifications are tagged with the sessionId from the prompt's
//!   params (routing integrity), not a captured global.
//! - A turn ends with the prompt REQUEST's response carrying stopReason;
//!   updates are notifications (no id).
//! - Unknown requests (with id) get -32601; unknown notifications are ignored.
//! - `session/cancel` answers a permission-blocked turn with "cancelled";
//!   otherwise it is a no-op (accurate: no turn in flight).
//!
//! Behaviour switches are argv flags (NOT env vars — env is process-global
//! and races under `cargo test`'s parallelism):
//!   --journal <path>    append every inbound message as {"dir":"in","msg":...}
//!   --hang-on-init      never answer `initialize`
//!   --delay-new-ms <n>  sleep n ms before answering `session/new`

use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;

static SHARED_JOURNAL: OnceLock<String> = OnceLock::new();

struct Flags {
    journal: Option<String>,
    hang_on_init: bool,
    delay_new_ms: u64,
}

fn parse_flags() -> Flags {
    let mut f = Flags {
        journal: None,
        hang_on_init: false,
        delay_new_ms: 0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--journal" => f.journal = it.next(),
            "--hang-on-init" => f.hang_on_init = true,
            "--delay-new-ms" => {
                f.delay_new_ms = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            _ => {}
        }
    }
    f
}

fn main() {
    let flags = parse_flags();
    if let Some(ref p) = flags.journal {
        let _ = SHARED_JOURNAL.set(p.clone());
    }
    let mut journal = flags.journal.as_ref().map(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .expect("open journal")
    });
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let mut session_counter: u64 = 0;
    let mut rpc_counter: u64 = 0;
    // A turn blocked on a permission response: (perm_req_id, prompt_req_id, session_id).
    let mut pending_perm: Option<(Value, Value, String)> = None;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(j) = journal.as_mut() {
            let rec = json!({"dir": "in", "msg": v});
            let mut s = serde_json::to_string(&rec).unwrap_or_default();
            s.push('\n');
            let _ = j.write_all(s.as_bytes());
            let _ = j.flush();
        }

        let id = v.get("id").cloned();
        let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");

        if method.is_empty() {
            // A response (to our session/request_permission).
            if let (Some((perm_id, prompt_id, sid)), Some(rid)) = (&pending_perm, &id) {
                if perm_id == rid {
                    let (sid, prompt_id) = (sid.clone(), prompt_id.clone());
                    pending_perm = None;
                    send_chunk(&mut out, &sid, "perm done");
                    send(
                        &mut out,
                        json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn"}}),
                    );
                }
            }
            continue;
        }

        match method {
            "initialize" => {
                if flags.hang_on_init {
                    continue;
                }
                send(
                    &mut out,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": 1,
                            "agentCapabilities": {
                                "loadSession": false,
                                "promptCapabilities": {
                                    "image": false,
                                    "audio": false,
                                    "embeddedContext": false
                                },
                                "mcpCapabilities": {"http": false, "sse": false},
                                "sessionCapabilities": {}
                            },
                            "authMethods": [],
                            "agentInfo": {
                                "name": "fake-claude",
                                "title": "fake claude",
                                "version": "0.1.0"
                            }
                        }
                    }),
                );
            }
            "session/new" => {
                if flags.delay_new_ms > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(flags.delay_new_ms));
                }
                session_counter += 1;
                let sid = format!("sess-{session_counter}");
                send(
                    &mut out,
                    json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":sid}}),
                );
            }
            "session/prompt" => {
                let sid = v
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let text = v
                    .pointer("/params/prompt/0/text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let prompt_id = id.clone().unwrap_or(Value::Null);
                match text {
                    "crash" => {
                        send_chunk(&mut out, &sid, "boom");
                        std::process::exit(2);
                    }
                    "perm" => {
                        rpc_counter += 1;
                        let perm_id = json!(format!("perm-{rpc_counter}"));
                        send(
                            &mut out,
                            json!({
                                "jsonrpc": "2.0",
                                "id": perm_id,
                                "method": "session/request_permission",
                                "params": {
                                    "sessionId": sid,
                                    "toolCall": {
                                        "toolCallId": "tc-1",
                                        "title": "Bash",
                                        "rawInput": {"cmd": "rm -rf /"}
                                    },
                                    "options": [
                                        {"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"},
                                        {"optionId": "allow_always", "name": "Allow session", "kind": "allow_always"},
                                        {"optionId": "reject_once", "name": "Deny", "kind": "reject_once"}
                                    ]
                                }
                            }),
                        );
                        pending_perm = Some((perm_id, prompt_id, sid));
                    }
                    _ => {
                        send_chunk(&mut out, &sid, "hello ");
                        send_chunk(&mut out, &sid, "world");
                        send(
                            &mut out,
                            json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn"}}),
                        );
                    }
                }
            }
            "session/cancel" => {
                let sid = v
                    .pointer("/params/sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if let Some((_, prompt_id, p_sid)) = &pending_perm {
                    if p_sid == sid {
                        let prompt_id = prompt_id.clone();
                        pending_perm = None;
                        send(
                            &mut out,
                            json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"cancelled"}}),
                        );
                    }
                }
                // No turn in flight -> ignore (accurate: cancel is a notification).
            }
            _ => {
                if let Some(req_id) = id {
                    send(
                        &mut out,
                        json!({"jsonrpc":"2.0","id":req_id,"error":{"code":-32601,"message":"method not found"}}),
                    );
                }
            }
        }
    }
}

fn send_chunk(out: &mut io::StdoutLock<'_>, sid: &str, text: &str) {
    send(
        out,
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": sid,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": text}
                }
            }
        }),
    );
}

/// Write msg to stdout; also append to the shared journal (if any) as `dir:"out"`.
fn send(stdout: &mut io::StdoutLock<'_>, msg: Value) {
    // Journal outbound.
    if let Some(jpath) = SHARED_JOURNAL.get() {
        if let Ok(mut jf) = std::fs::OpenOptions::new().create(true).append(true).open(jpath) {
            let rec = json!({"dir": "out", "msg": msg});
            let mut s = serde_json::to_string(&rec).unwrap_or_default();
            s.push('\n');
            let _ = jf.write_all(s.as_bytes());
            let _ = jf.flush();
        }
    }
    let mut s = serde_json::to_string(&msg).unwrap_or_default();
    s.push('\n');
    let _ = stdout.write_all(s.as_bytes());
    let _ = stdout.flush();
}
