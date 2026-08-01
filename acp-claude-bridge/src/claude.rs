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

/// Parse one stream-json line into a StreamEvent.
/// Returns `Ok(None)` if the line is empty or a comment.
pub fn parse_line(line: &str) -> Result<Option<StreamEvent>, ParseError> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let v: serde_json::Value = serde_json::from_str(line)?;
    Ok(Some(parse_value(v)?))
}

fn parse_value(v: serde_json::Value) -> Result<StreamEvent, ParseError> {
    let obj = v.as_object().ok_or_else(|| {
        ParseError::Json(serde_json::from_str::<serde_json::Value>("\"\"").unwrap_err())
    })?;
    let ty = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    Ok(match ty {
        "system" if obj.get("subtype").and_then(|v| v.as_str()) == Some("init") => {
            StreamEvent::System {
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
            }
        }
        "stream_event" => parse_stream_event(obj)?,
        "user" => parse_user_message(obj)?,
        "result" => {
            let stop_reason = match obj.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("") {
                "end_turn" => StopReason::EndTurn,
                "max_tokens" => StopReason::MaxTokens,
                "tool_use" => StopReason::ToolUse,
                other => StopReason::Unknown(other.into()),
            };
            StreamEvent::TurnEnd { stop_reason }
        }
        _ => StreamEvent::Unknown,
    })
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
        let ev = parse_line(&line).unwrap().unwrap();
        match ev {
            StreamEvent::System { session_id, model, tool_names } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(model, "claude-sonnet-4-5");
                assert_eq!(tool_names, vec!["Bash", "Read", "Edit"]);
            }
            other => panic!("expected System, got {other:?}"),
        }
    }

    #[test]
    fn parses_text_delta() {
        let line = fixture("text_delta.jsonl");
        let ev = parse_line(&line).unwrap().unwrap();
        assert_eq!(ev, StreamEvent::TextDelta { text: "hello ".into() });
    }

    #[test]
    fn parses_tool_use() {
        let line = fixture("tool_use.jsonl");
        let ev = parse_line(&line).unwrap().unwrap();
        match ev {
            StreamEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_01");
                assert_eq!(name, "Bash");
                assert_eq!(input, serde_json::json!({"command": "ls"}));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn parses_result_end_turn() {
        let line = fixture("result_end_turn.jsonl");
        let ev = parse_line(&line).unwrap().unwrap();
        assert_eq!(
            ev,
            StreamEvent::TurnEnd { stop_reason: StopReason::EndTurn }
        );
    }

    #[test]
    fn empty_line_returns_none() {
        assert!(parse_line("").unwrap().is_none());
        assert!(parse_line("   \n").unwrap().is_none());
    }

    #[test]
    fn unknown_event_returns_unknown() {
        let line = r#"{"type":"some_future_event","data":1}"#;
        let ev = parse_line(line).unwrap().unwrap();
        assert_eq!(ev, StreamEvent::Unknown);
    }
}
