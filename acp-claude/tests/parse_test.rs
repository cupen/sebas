use acp_claude::session::AcpEvent;

#[test]
fn parses_full_session_stream() {
    let raw = include_str!("../../tests/fixtures/acp/basic_session.jsonl");
    let events: Vec<AcpEvent> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("parse"))
        .collect();
    assert_eq!(events.len(), 7);

    match &events[0] {
        AcpEvent::TextDelta { session_id, delta } => {
            assert_eq!(session_id, "s1");
            assert_eq!(delta, "hello ");
        }
        _ => panic!("expected text_delta"),
    }

    match &events[2] {
        AcpEvent::ToolStart { tool_name, .. } => assert_eq!(tool_name, "Read"),
        _ => panic!("expected tool_start"),
    }

    match &events[4] {
        AcpEvent::PermissionRequest {
            request_id,
            tool_name,
            ..
        } => {
            assert_eq!(request_id, "r1");
            assert_eq!(tool_name, "Bash");
        }
        _ => panic!("expected permission_request"),
    }

    matches!(events.last(), Some(AcpEvent::Finished { .. }));
}
