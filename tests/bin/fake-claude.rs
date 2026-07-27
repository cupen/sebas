//! Minimal stand-in for `claude-code-acp` used in tests until the
//! real binary lands.
//!
//! Speaks the official Agent Client Protocol (ACP) JSON-RPC v2 on
//! stdio. The wire format is the same as the messages emitted by
//! `agent-client-protocol` v2 — this binary just writes them
//! directly without depending on the SDK, so the test fixture does
//! not need to depend on the SDK to be useful.
//!
//! Behaviour:
//! - Reply to `initialize` with protocolVersion 1, agentInfo populated.
//! - Reply to `session/new` with a fixed `sessionId: "sess-1"`.
//! - On `session/prompt`, emit two `session/update` notifications
//!   (text chunks "hello " and "world") then a `session/prompt`
//!   response with `stopReason: "end_turn"`.
//! - Anything else: no-op (we don't try to be a complete agent).

use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut session_id = "sess-1".to_string();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(?e, "fake-claude: failed to parse line");
                continue;
            }
        };
        let id = v.get("id").cloned();
        let method = v
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        match method.as_str() {
            "initialize" => {
                send(
                    &mut stdout,
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
                session_id = "sess-1".to_string();
                send(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "sessionId": session_id
                        }
                    }),
                );
            }
            "session/prompt" => {
                let prompt = v.get("params").and_then(|p| p.get("prompt"));
                let want_text = prompt
                    .and_then(|p| p.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|blk| blk.get("type"))
                    .and_then(|t| t.as_str())
                    .map(|t| t == "text")
                    .unwrap_or(false);
                if want_text {
                    send_notification(
                        &mut stdout,
                        "session/update",
                        json!({
                            "sessionId": session_id,
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": {
                                    "type": "text",
                                    "text": "hello "
                                }
                            }
                        }),
                    );
                    send_notification(
                        &mut stdout,
                        "session/update",
                        json!({
                            "sessionId": session_id,
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": {
                                    "type": "text",
                                    "text": "world"
                                }
                            }
                        }),
                    );
                }
                send(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "stopReason": "end_turn"
                        }
                    }),
                );
            }
            "session/cancel" => {
                // acknowledge nothing; client treats Finished as the
                // session-ending signal anyway.
            }
            _ => {
                tracing::debug!(method, "fake-claude: unhandled method");
            }
        }
    }
}

fn send(stdout: &mut io::StdoutLock<'_>, msg: Value) {
    let mut s = serde_json::to_string(&msg).unwrap_or_default();
    s.push('\n');
    let _ = stdout.write_all(s.as_bytes());
    let _ = stdout.flush();
}

fn send_notification(stdout: &mut io::StdoutLock<'_>, method: &str, params: Value) {
    send(
        stdout,
        json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }),
    );
}
