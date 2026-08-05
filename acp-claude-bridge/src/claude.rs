//! Types and parser for Claude Code's stream-json over stdio.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    Cancelled,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    System {
        session_id: String,
        model: String,
        tool_names: Vec<String>,
    },
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    TurnEnd {
        stop_reason: StopReason,
    },
    Unknown,
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Parse one stream-json line into one or more `StreamEvent`s.
/// Returns `Ok(vec![])` if the line is empty, a comment, or a recognized
/// envelope that carries no in-band event (e.g. `system` with a non-init
/// subtype).
pub fn parse_line(line: &str) -> Result<Vec<StreamEvent>, ParseError> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(Vec::new());
    }
    let v: serde_json::Value = serde_json::from_str(line)?;
    parse_value(v)
}

/// Parse one envelope line. Most envelope types map to a single in-band
/// event; `"assistant"` may carry multiple content blocks (text + thinking
/// + tool_use) so it returns one `StreamEvent` per block.
fn parse_value(v: serde_json::Value) -> Result<Vec<StreamEvent>, ParseError> {
    let obj = v.as_object().ok_or_else(|| {
        ParseError::Json(serde_json::from_str::<serde_json::Value>("\"\"").unwrap_err())
    })?;
    let ty = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    Ok(match ty {
        "system" if obj.get("subtype").and_then(|v| v.as_str()) == Some("init") => {
            vec![StreamEvent::System {
                session_id: obj.get("session_id").and_then(|v| v.as_str()).unwrap_or("").into(),
                model: obj.get("model").and_then(|v| v.as_str()).unwrap_or("").into(),
                tool_names: obj
                    .get("tools")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            }]
        }
        "assistant" => parse_assistant_message(obj),
        "stream_event" => vec![parse_stream_event(obj)?],
        "user" => parse_user_message(obj).into_iter().collect(),
        "result" => {
            let stop_reason = match obj.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("") {
                "end_turn" => StopReason::EndTurn,
                "max_tokens" => StopReason::MaxTokens,
                "tool_use" => StopReason::ToolUse,
                other => StopReason::Unknown(other.into()),
            };
            vec![StreamEvent::TurnEnd { stop_reason }]
        }
        _ => Vec::new(),
    })
}

/// Claude Code CLI's `assistant` envelope carries the full assistant turn
/// inside `message.content[]` as a sequence of typed blocks:
///   {"type":"thinking","thinking":"..."}    → ThinkingDelta
///   {"type":"text","text":"..."}            → TextDelta
///   {"type":"tool_use","id":"...","name":"...","input":{...}} → ToolUse
///
/// Earlier versions of the bridge only handled Anthropic-API stream events
/// (`"type":"stream_event"` + nested `content_block_delta`); under Claude
/// Code v2.1.220, assistant turns arrive as `"type":"assistant"` and were
/// silently dropped, so no TextDelta ever reached the card.
fn parse_assistant_message(obj: &serde_json::Map<String, serde_json::Value>) -> Vec<StreamEvent> {
    let message = match obj.get("message").and_then(|m| m.as_object()) {
        Some(m) => m,
        None => return Vec::new(),
    };
    let content = match message.get("content").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return Vec::new(),
    };
    let mut out = Vec::with_capacity(content.len());
    for block in content {
        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match btype {
            "text" => {
                let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                if !text.is_empty() {
                    out.push(StreamEvent::TextDelta { text: text.to_string() });
                }
            }
            "thinking" => {
                let thinking = block.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                if !thinking.is_empty() {
                    out.push(StreamEvent::ThinkingDelta {
                        thinking: thinking.to_string(),
                    });
                }
            }
            "tool_use" => {
                let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                out.push(StreamEvent::ToolUse { id, name, input });
            }
            _ => {} // unknown block kind: ignore (don't drop the rest)
        }
    }
    out
}

fn parse_stream_event(obj: &serde_json::Map<String, serde_json::Value>) -> Result<StreamEvent, ParseError> {
    let event = match obj.get("event") {
        Some(e) => e,
        None => return Ok(StreamEvent::Unknown),
    };
    let event_obj = match event.as_object() {
        Some(o) => o,
        None => return Ok(StreamEvent::Unknown),
    };
    let etype = event_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    Ok(match etype {
        "content_block_delta" => {
            let delta = event_obj.get("delta");
            let text = delta
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                StreamEvent::Unknown
            } else {
                StreamEvent::TextDelta { text }
            }
        }
        "content_block_start" => {
            let block = match event_obj.get("content_block") {
                Some(b) => b,
                None => return Ok(StreamEvent::Unknown),
            };
            let block_obj = match block.as_object() {
                Some(o) => o,
                None => return Ok(StreamEvent::Unknown),
            };
            if block_obj.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                return Ok(StreamEvent::Unknown);
            }
            StreamEvent::ToolUse {
                id: block_obj.get("id").and_then(|v| v.as_str()).unwrap_or("").into(),
                name: block_obj.get("name").and_then(|v| v.as_str()).unwrap_or("").into(),
                input: block_obj.get("input").cloned().unwrap_or(serde_json::json!({})),
            }
        }
        _ => StreamEvent::Unknown,
    })
}

fn parse_user_message(obj: &serde_json::Map<String, serde_json::Value>) -> Result<StreamEvent, ParseError> {
    let message = match obj.get("message") {
        Some(m) => m,
        None => return Ok(StreamEvent::Unknown),
    };
    let content = match message.get("content").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return Ok(StreamEvent::Unknown),
    };
    for block in content {
        if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
            return Ok(StreamEvent::ToolResult {
                tool_use_id: block.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("").into(),
                content: block
                    .get("content")
                    .map(|c| match c {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default(),
                is_error: block.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false),
            });
        }
    }
    Ok(StreamEvent::Unknown)
}

pub mod driver {
    //! Subprocess management + stream-json framing for `claude --print`.

    use super::{parse_line, ParseError, StreamEvent};
    use std::ffi::OsStr;
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::{Child, Command};
    use tokio::sync::mpsc;

    pub struct ClaudeDriver {
        child: Child,
        rx: mpsc::Receiver<StreamEvent>,
    }

    impl ClaudeDriver {
        pub async fn spawn<I, S>(binary: &str, args: I) -> anyhow::Result<Self>
        where
            I: IntoIterator<Item = S>,
            S: AsRef<OsStr>,
        {
            let mut child = Command::new(binary)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .kill_on_drop(true)
                .spawn()?;
            let stdout = child.stdout.take().expect("piped stdout");
            let (tx, rx) = mpsc::channel(64);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => match parse_line(&line) {
                            Ok(events) => {
                                let mut closed = false;
                                for ev in events {
                                    if tx.send(ev).await.is_err() {
                                        closed = true;
                                        break;
                                    }
                                }
                                if closed {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error=%e, line=%line, "parse failed");
                            }
                        },
                        Ok(None) => break,
                        Err(e) => {
                            tracing::warn!(error=%e, "stdout read failed");
                            break;
                        }
                    }
                }
            });
            Ok(Self { child, rx })
        }

        pub async fn next_event(&mut self) -> Option<StreamEvent> {
            self.rx.recv().await
        }

        pub async fn send_user(&mut self, text: &str) -> anyhow::Result<()> {
            let stdin = self.child.stdin.as_mut().expect("piped stdin");
            let msg = serde_json::json!({
                "type": "user",
                "message": {"role": "user", "content": [{"type": "text", "text": text}]}
            });
            stdin.write_all(msg.to_string().as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await?;
            Ok(())
        }
    }

    #[allow(dead_code)]
    fn _re_export_parse_error() -> ParseError {
        ParseError::Json(serde_json::from_str::<serde_json::Value>("\"\"").unwrap_err())
    }
}

pub use driver::ClaudeDriver;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/tests/fixtures/stream-json/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
    }

    #[test]
    fn parses_system_init() {
        let line = fixture("system_init.jsonl");
        let ev = &parse_line(&line).unwrap()[0];
        match ev {
            StreamEvent::System { session_id, model, tool_names } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(model, "claude-sonnet-4-5");
                assert_eq!(
                    tool_names,
                    &["Bash".to_string(), "Read".to_string(), "Edit".to_string()]
                );
            }
            other => panic!("expected System, got {other:?}"),
        }
    }

    #[test]
    fn parses_text_delta() {
        let line = fixture("text_delta.jsonl");
        let ev = &parse_line(&line).unwrap()[0];
        assert_eq!(*ev, StreamEvent::TextDelta { text: "hello ".into() });
    }

    #[test]
    fn parses_tool_use() {
        let line = fixture("tool_use.jsonl");
        let ev = &parse_line(&line).unwrap()[0];
        match ev {
            StreamEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_01");
                assert_eq!(name, "Bash");
                assert_eq!(input, &serde_json::json!({"command": "ls"}));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn parses_result_end_turn() {
        let line = fixture("result_end_turn.jsonl");
        let ev = &parse_line(&line).unwrap()[0];
        assert_eq!(
            *ev,
            StreamEvent::TurnEnd { stop_reason: StopReason::EndTurn }
        );
    }

    #[test]
    fn empty_line_returns_empty_vec() {
        assert!(parse_line("").unwrap().is_empty());
        assert!(parse_line("   \n").unwrap().is_empty());
    }

    #[test]
    fn unknown_envelope_returns_empty_vec() {
        // `system` with non-init subtype is recognized as a system envelope
        // but carries no in-band event; non-system unknowns likewise drop.
        let line = r#"{"type":"some_future_event","data":1}"#;
        assert!(parse_line(line).unwrap().is_empty());
    }

    // ---- assistant envelope (Claude Code v2.1+ stream-json format) ----

    #[test]
    fn parses_assistant_text_block() {
        // The Claude Code envelope wraps the message as
        // {"type":"assistant","message":{"content":[{"type":"text","text":"OK"}]}}
        let line = r#"{
            "type":"assistant",
            "message":{
                "role":"assistant",
                "content":[{"type":"text","text":"OK"}]
            }
        }"#;
        let evs = parse_line(line).unwrap();
        assert_eq!(evs, vec![StreamEvent::TextDelta { text: "OK".into() }]);
    }

    #[test]
    fn parses_assistant_thinking_block() {
        let line = r#"{
            "type":"assistant",
            "message":{
                "role":"assistant",
                "content":[{"type":"thinking","thinking":"the user wants OK"}]
            }
        }"#;
        let evs = parse_line(line).unwrap();
        assert_eq!(
            evs,
            vec![StreamEvent::ThinkingDelta {
                thinking: "the user wants OK".into()
            }]
        );
    }

    #[test]
    fn parses_assistant_tool_use_block() {
        let line = r#"{
            "type":"assistant",
            "message":{
                "role":"assistant",
                "content":[{
                    "type":"tool_use",
                    "id":"toolu_01",
                    "name":"Bash",
                    "input":{"command":"ls"}
                }]
            }
        }"#;
        let evs = parse_line(line).unwrap();
        assert_eq!(
            evs,
            vec![StreamEvent::ToolUse {
                id: "toolu_01".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command": "ls"}),
            }]
        );
    }

    #[test]
    fn parses_assistant_with_multiple_blocks_in_order() {
        // Real turns emit thinking then text (and sometimes tool_use).
        // The parser must emit all blocks in source order so the card
        // accumulates them as a faithful transcript.
        let line = r#"{
            "type":"assistant",
            "message":{
                "role":"assistant",
                "content":[
                    {"type":"thinking","thinking":"hi"},
                    {"type":"text","text":"hello"},
                    {"type":"tool_use","id":"toolu_01","name":"Bash","input":{}}
                ]
            }
        }"#;
        let evs = parse_line(line).unwrap();
        assert_eq!(evs.len(), 3);
        assert_eq!(
            evs[0],
            StreamEvent::ThinkingDelta { thinking: "hi".into() }
        );
        assert_eq!(evs[1], StreamEvent::TextDelta { text: "hello".into() });
        assert_eq!(
            evs[2],
            StreamEvent::ToolUse {
                id: "toolu_01".into(),
                name: "Bash".into(),
                input: serde_json::json!({}),
            }
        );
    }

    #[test]
    fn parses_assistant_with_unknown_block_kind_keeps_the_rest() {
        // Future Claude Code versions might add a new block kind; unknown
        // kinds must not poison the parse — siblings still arrive.
        let line = r#"{
            "type":"assistant",
            "message":{
                "role":"assistant",
                "content":[
                    {"type":"future_block","data":1},
                    {"type":"text","text":"hi"}
                ]
            }
        }"#;
        let evs = parse_line(line).unwrap();
        assert_eq!(evs, vec![StreamEvent::TextDelta { text: "hi".into() }]);
    }

    #[test]
    fn parses_assistant_with_empty_text_block_skips_it() {
        let line = r#"{
            "type":"assistant",
            "message":{
                "role":"assistant",
                "content":[
                    {"type":"text","text":""},
                    {"type":"text","text":"real"}
                ]
            }
        }"#;
        let evs = parse_line(line).unwrap();
        assert_eq!(evs, vec![StreamEvent::TextDelta { text: "real".into() }]);
    }
}
