//! Translate `TranslatedUpdate` into ACP `SessionNotification` for the bridge.

use crate::claude::StopReason as ClaudeStopReason;
use crate::translator::{TranslatedUpdate, ToolStatus};
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, SessionNotification, SessionUpdate, StopReason, TextContent,
    ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
};
use serde_json::json;

pub fn from_update(
    session_id: &str,
    update: TranslatedUpdate,
) -> Option<SessionNotification> {
    let session_id = agent_client_protocol::schema::v1::SessionId::new(session_id.to_string());
    let su = match update {
        TranslatedUpdate::AgentMessageChunk { text } => SessionUpdate::AgentMessageChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new(text))),
        ),
        TranslatedUpdate::ToolCall { id, title, raw_input } => {
            SessionUpdate::ToolCall(ToolCall::new(id, title).raw_input(raw_input))
        }
        TranslatedUpdate::ToolCallUpdate {
            id,
            status,
            raw_output,
        } => {
            let s = match status {
                ToolStatus::Completed => ToolCallStatus::Completed,
                ToolStatus::Failed => ToolCallStatus::Failed,
            };
            let fields = ToolCallUpdateFields::new()
                .status(s)
                .raw_output(raw_output.map(serde_json::Value::String));
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id, fields))
        }
        TranslatedUpdate::TurnEnd { .. } => return None,
    };
    Some(SessionNotification::new(session_id, su))
}

pub fn acp_stop_reason(s: ClaudeStopReason) -> StopReason {
    match s {
        ClaudeStopReason::EndTurn => StopReason::EndTurn,
        ClaudeStopReason::MaxTokens => StopReason::MaxTokens,
        // ACP 没有 ToolUse 变体：claude 调工具是正常 turn 结束
        ClaudeStopReason::ToolUse => StopReason::EndTurn,
        ClaudeStopReason::Cancelled => StopReason::Cancelled,
        ClaudeStopReason::Unknown(_) => StopReason::EndTurn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::StopReason as ClaudeSR;

    fn sid() -> &'static str {
        "sess-1"
    }

    #[test]
    fn agent_message_chunk_to_session_notification() {
        let n = from_update(
            sid(),
            TranslatedUpdate::AgentMessageChunk { text: "hi".into() },
        )
        .expect("notification");
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["sessionId"], "sess-1");
        assert_eq!(v["update"]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(v["update"]["content"]["type"], "text");
        assert_eq!(v["update"]["content"]["text"], "hi");
    }

    #[test]
    fn tool_call_preserves_id_and_title() {
        let n = from_update(
            sid(),
            TranslatedUpdate::ToolCall {
                id: "toolu_01".into(),
                title: "Bash".into(),
                raw_input: json!({"command": "ls"}),
            },
        )
        .expect("notification");
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["update"]["sessionUpdate"], "tool_call");
        assert_eq!(v["update"]["toolCallId"], "toolu_01");
        assert_eq!(v["update"]["title"], "Bash");
        assert_eq!(v["update"]["rawInput"]["command"], "ls");
    }

    #[test]
    fn tool_call_update_with_output() {
        let n = from_update(
            sid(),
            TranslatedUpdate::ToolCallUpdate {
                id: "toolu_01".into(),
                status: ToolStatus::Completed,
                raw_output: Some("file.txt".into()),
            },
        )
        .expect("notification");
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["update"]["sessionUpdate"], "tool_call_update");
        assert_eq!(v["update"]["toolCallId"], "toolu_01");
        assert_eq!(v["update"]["status"], "completed");
        assert_eq!(v["update"]["rawOutput"], "file.txt");
    }

    #[test]
    fn turn_end_returns_none() {
        let r = from_update(
            sid(),
            TranslatedUpdate::TurnEnd {
                stop_reason: ClaudeSR::EndTurn,
            },
        );
        assert!(r.is_none());
    }

    #[test]
    fn acp_stop_reason_mapping() {
        assert_eq!(acp_stop_reason(ClaudeSR::EndTurn), StopReason::EndTurn);
        assert_eq!(acp_stop_reason(ClaudeSR::MaxTokens), StopReason::MaxTokens);
        // ACP 没有 ToolUse 变体：调工具视为正常 turn 结束
        assert_eq!(acp_stop_reason(ClaudeSR::ToolUse), StopReason::EndTurn);
        assert_eq!(acp_stop_reason(ClaudeSR::Cancelled), StopReason::Cancelled);
        assert_eq!(
            acp_stop_reason(ClaudeSR::Unknown("mystery".into())),
            StopReason::EndTurn
        );
    }
}
