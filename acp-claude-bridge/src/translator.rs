//! Translates stream-json events into ACP `SessionUpdate`-shaped messages.

use crate::claude::{StopReason, StreamEvent};

#[derive(Debug, Clone, PartialEq)]
pub enum TranslatedUpdate {
    AgentMessageChunk { text: String },
    AgentThoughtChunk { text: String },
    ToolCall { id: String, title: String, raw_input: serde_json::Value },
    ToolCallUpdate { id: String, status: ToolStatus, raw_output: Option<String> },
    TurnEnd { stop_reason: StopReason },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolStatus {
    Completed,
    Failed,
}

pub fn translate(event: StreamEvent) -> Vec<TranslatedUpdate> {
    match event {
        StreamEvent::System { .. } => vec![],
        StreamEvent::TextDelta { text } => vec![TranslatedUpdate::AgentMessageChunk { text }],
        StreamEvent::ThinkingDelta { thinking } => {
            vec![TranslatedUpdate::AgentThoughtChunk { text: thinking }]
        }
        StreamEvent::ToolUse { id, name, input } => vec![TranslatedUpdate::ToolCall {
            id,
            title: name,
            raw_input: input,
        }],
        StreamEvent::ToolResult { tool_use_id, content, is_error } => vec![TranslatedUpdate::ToolCallUpdate {
            id: tool_use_id,
            status: if is_error { ToolStatus::Failed } else { ToolStatus::Completed },
            raw_output: Some(content),
        }],
        StreamEvent::TurnEnd { stop_reason } => vec![TranslatedUpdate::TurnEnd { stop_reason }],
        StreamEvent::Unknown => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::StopReason;

    #[test]
    fn text_delta_becomes_chunk() {
        let out = translate(StreamEvent::TextDelta { text: "hi".into() });
        assert_eq!(out, vec![TranslatedUpdate::AgentMessageChunk { text: "hi".into() }]);
    }

    #[test]
    fn tool_use_becomes_call() {
        let out = translate(StreamEvent::ToolUse {
            id: "toolu_01".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command":"ls"}),
        });
        assert_eq!(
            out,
            vec![TranslatedUpdate::ToolCall {
                id: "toolu_01".into(),
                title: "Bash".into(),
                raw_input: serde_json::json!({"command":"ls"}),
            }]
        );
    }

    #[test]
    fn tool_result_becomes_update_completed() {
        let out = translate(StreamEvent::ToolResult {
            tool_use_id: "toolu_01".into(),
            content: "file.txt".into(),
            is_error: false,
        });
        assert_eq!(
            out,
            vec![TranslatedUpdate::ToolCallUpdate {
                id: "toolu_01".into(),
                status: ToolStatus::Completed,
                raw_output: Some("file.txt".into()),
            }]
        );
    }

    #[test]
    fn tool_result_error_becomes_failed() {
        let out = translate(StreamEvent::ToolResult {
            tool_use_id: "toolu_01".into(),
            content: "permission denied".into(),
            is_error: true,
        });
        assert_eq!(
            out,
            vec![TranslatedUpdate::ToolCallUpdate {
                id: "toolu_01".into(),
                status: ToolStatus::Failed,
                raw_output: Some("permission denied".into()),
            }]
        );
    }

    #[test]
    fn turn_end_translates_directly() {
        let out = translate(StreamEvent::TurnEnd { stop_reason: StopReason::EndTurn });
        assert_eq!(
            out,
            vec![TranslatedUpdate::TurnEnd { stop_reason: StopReason::EndTurn }]
        );
    }

    #[test]
    fn system_event_emits_nothing() {
        let out = translate(StreamEvent::System {
            session_id: "x".into(),
            model: "m".into(),
            tool_names: vec![],
        });
        assert!(out.is_empty());
    }

    #[test]
    fn unknown_emits_nothing() {
        assert!(translate(StreamEvent::Unknown).is_empty());
    }
}