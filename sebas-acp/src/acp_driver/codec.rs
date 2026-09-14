//! Pure ACP → `AcpEvent` translation (no I/O; unit-testable).
//!
//! The generic ACP driver only understands a subset of the ACP v1
//! `SessionUpdate` surface that maps onto sebas's vocabulary: text/thinking
//! deltas, tool starts/updates, and — via a request handler, not here —
//! permission requests. Plan/mode/config/session-info updates are ignored
//! (forward-compatible), as is ACP's `UsageUpdate` (which reports context
//! window/cost, not Anthropic-style token counts — see design R1).

use crate::session::{AcpEvent, AvailableCommand};
use agent_client_protocol::schema::v1::{
    AvailableCommandInput, ContentBlock, SessionNotification, SessionUpdate, ToolCallStatus,
    ToolCallUpdate,
};
use serde_json::Value;
use std::collections::HashMap;

/// Translate one ACP session notification into zero or more `AcpEvent`s,
/// stamped with the sebas routing `session_id` (NOT the ACP session id).
///
/// `tool_names` tracks `tool_call_id → title` so a later `ToolCallUpdate`
/// (which only carries the id) can name the tool in `ToolEnd`.
pub fn translate_notification(
    session_id: &str,
    tool_names: &mut HashMap<String, String>,
    n: &SessionNotification,
) -> Vec<AcpEvent> {
    let sid = || session_id.to_string();
    match &n.update {
        SessionUpdate::AgentMessageChunk(chunk) => text_delta(sid(), chunk, false),
        SessionUpdate::AgentThoughtChunk(chunk) => text_delta(sid(), chunk, true),
        SessionUpdate::ToolCall(tc) => {
            tool_names.insert(tc.tool_call_id.to_string(), tc.title.clone());
            vec![AcpEvent::ToolStart {
                session_id: sid(),
                tool_name: tc.title.clone(),
                args: tc.raw_input.clone().unwrap_or(Value::Null),
            }]
        }
        SessionUpdate::ToolCallUpdate(upd) => tool_update(&sid(), tool_names, upd),
        // （session-slash-commands 1.3）agent 广告的命令表 → AvailableCommands
        // 事件（与 claude 路径同一事件词汇）；二次通知原样再发——引擎侧覆盖
        // 旧表，本层不去重不丢帧（重新广告刷新，1.4）。
        SessionUpdate::AvailableCommandsUpdate(upd) => {
            vec![AcpEvent::AvailableCommands {
                session_id: sid(),
                commands: upd
                    .available_commands
                    .iter()
                    .map(|c| AvailableCommand {
                        name: c.name.clone(),
                        description: c.description.clone(),
                        // ACP 的参数提示住在 `input` 的 unstructured 形态里。
                        hint: c.input.as_ref().and_then(|i| match i {
                            AvailableCommandInput::Unstructured(u) => Some(u.hint.clone()),
                            _ => None,
                        }),
                    })
                    .collect(),
            }]
        }
        // Plan, mode updates, config, session info, ACP usage, user-message
        // echoes — not part of the sebas vocabulary.
        _ => vec![],
    }
}

fn text_delta(session_id: String, chunk: &agent_client_protocol::schema::v1::ContentChunk, thinking: bool) -> Vec<AcpEvent> {
    let ContentBlock::Text(t) = &chunk.content else {
        return vec![];
    };
    if thinking {
        vec![AcpEvent::ThinkingDelta {
            session_id,
            delta: t.text.clone(),
        }]
    } else {
        vec![AcpEvent::TextDelta {
            session_id,
            delta: t.text.clone(),
        }]
    }
}

fn tool_update(
    session_id: &str,
    tool_names: &HashMap<String, String>,
    upd: &ToolCallUpdate,
) -> Vec<AcpEvent> {
    let Some(status) = &upd.fields.status else {
        return vec![];
    };
    let name = tool_names
        .get(&upd.tool_call_id.to_string())
        .cloned()
        .or_else(|| upd.fields.title.clone())
        .unwrap_or_else(|| "tool".to_string());
    let result = tool_result(upd);
    match status {
        ToolCallStatus::Completed => vec![AcpEvent::ToolEnd {
            session_id: session_id.to_string(),
            tool_name: name,
            result,
        }],
        ToolCallStatus::Failed => vec![AcpEvent::ToolEnd {
            session_id: session_id.to_string(),
            tool_name: name,
            result,
        }],
        ToolCallStatus::InProgress => vec![AcpEvent::ToolProgress {
            session_id: session_id.to_string(),
            tool_name: name,
            progress: result,
        }],
        ToolCallStatus::Pending => vec![],
        _ => vec![],
    }
}

/// Best-effort result text from a tool update: prefer `raw_output`, else the
/// text of any `content` blocks, else a status-only placeholder.
fn tool_result(upd: &ToolCallUpdate) -> String {
    if let Some(raw) = &upd.fields.raw_output {
        return raw.to_string();
    }
    if let Some(content) = &upd.fields.content {
        let mut parts = Vec::new();
        for c in content {
            if let agent_client_protocol::schema::v1::ToolCallContent::Content(inner) = c
                && let ContentBlock::Text(t) = &inner.content
            {
                parts.push(t.text.clone());
            }
        }
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    match &upd.fields.status {
        Some(ToolCallStatus::Failed) => "tool call failed".to_string(),
        _ => "done".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{ContentChunk, TextContent, ToolCall, ToolCallUpdateFields};

    fn chunk_text(text: &str) -> SessionNotification {
        SessionNotification::new(
            "acp-sess",
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new(text),
            ))),
        )
    }

    #[test]
    fn agent_message_chunk_becomes_text_delta() {
        let mut names = HashMap::new();
        let evts = translate_notification("route-1", &mut names, &chunk_text("hello"));
        assert_eq!(evts.len(), 1);
        match &evts[0] {
            AcpEvent::TextDelta { session_id, delta } => {
                assert_eq!(session_id, "route-1");
                assert_eq!(delta, "hello");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn thought_chunk_becomes_thinking_delta() {
        let mut names = HashMap::new();
        let n = SessionNotification::new(
            "acp-sess",
            SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new("thinking…"),
            ))),
        );
        let evts = translate_notification("route-1", &mut names, &n);
        assert!(matches!(evts[0], AcpEvent::ThinkingDelta { .. }));
    }

    #[test]
    fn tool_call_then_update_produce_start_and_end() {
        let mut names = HashMap::new();
        let start = SessionNotification::new(
            "acp-sess",
            SessionUpdate::ToolCall(
                ToolCall::new("tc-1", "run_bash").raw_input(serde_json::json!({"cmd": "ls"})),
            ),
        );
        let evts = translate_notification("route-1", &mut names, &start);
        assert!(matches!(evts[0], AcpEvent::ToolStart { ref tool_name, .. } if tool_name == "run_bash"));
        assert_eq!(names.get("tc-1").map(String::as_str), Some("run_bash"));

        let end = SessionNotification::new(
            "acp-sess",
            SessionUpdate::ToolCallUpdate(agent_client_protocol::schema::v1::ToolCallUpdate::new(
                "tc-1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .raw_output(serde_json::json!("ok")),
            )),
        );
        let evts = translate_notification("route-1", &mut names, &end);
        assert!(matches!(evts[0], AcpEvent::ToolEnd { ref tool_name, .. } if tool_name == "run_bash"));
    }

    // ---- session-slash-commands 1.3：AvailableCommandsUpdate → 事件 ----

    use agent_client_protocol::schema::v1::{
        AvailableCommand as AcpCommandEntry, AvailableCommandsUpdate, UnstructuredCommandInput,
    };

    fn commands_notification(commands: Vec<AcpCommandEntry>) -> SessionNotification {
        SessionNotification::new(
            "acp-sess",
            SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(commands)),
        )
    }

    #[test]
    fn available_commands_update_maps_to_event_with_hint() {
        let mut names = HashMap::new();
        let n = commands_notification(vec![
            AcpCommandEntry::new("goal", "Set a goal").input(Some(
                AvailableCommandInput::Unstructured(UnstructuredCommandInput::new("<condition>")),
            )),
            AcpCommandEntry::new("compact", "Summarize the session"),
        ]);
        let evts = translate_notification("route-1", &mut names, &n);
        assert_eq!(evts.len(), 1);
        match &evts[0] {
            AcpEvent::AvailableCommands {
                session_id,
                commands,
            } => {
                assert_eq!(session_id, "route-1", "stamped with the routing id");
                assert_eq!(commands.len(), 2);
                assert_eq!(commands[0].name, "goal");
                assert_eq!(commands[0].hint.as_deref(), Some("<condition>"));
                assert_eq!(commands[1].name, "compact");
                assert_eq!(commands[1].hint, None, "no input → hint None");
            }
            other => panic!("expected AvailableCommands, got {other:?}"),
        }
    }

    #[test]
    fn second_available_commands_update_is_forwarded_too() {
        // 1.4 重新广告：本层不去重不丢帧——第二次通知同样产出事件，引擎
        // 侧据此覆盖旧表。
        let mut names = HashMap::new();
        let first = commands_notification(vec![AcpCommandEntry::new("goal", "old table")]);
        let second = commands_notification(vec![
            AcpCommandEntry::new("goal", "refreshed"),
            AcpCommandEntry::new("compact", "new entry"),
        ]);
        let all = translate_notification("route-1", &mut names, &first)
            .into_iter()
            .chain(translate_notification("route-1", &mut names, &second))
            .count();
        assert_eq!(all, 2, "re-advertisement must not be dropped or deduped");
    }

    #[test]
    fn empty_commands_update_maps_to_event_with_empty_table() {
        // 空广告是合法的诚实退化输入：事件照发，表为空。
        let mut names = HashMap::new();
        let n = commands_notification(Vec::new());
        let evts = translate_notification("route-1", &mut names, &n);
        match &evts[0] {
            AcpEvent::AvailableCommands { commands, .. } => assert!(commands.is_empty()),
            other => panic!("expected AvailableCommands, got {other:?}"),
        }
    }
}
