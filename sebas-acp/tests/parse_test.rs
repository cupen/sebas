use sebas_acp::claude::session::AcpEvent;

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

#[test]
fn error_terminal_defaults_to_false_and_round_trips() {
    // Legacy shape (no terminal field) still parses, defaulting to false.
    let legacy = r#"{"type":"error","session_id":"s1","message":"boom"}"#;
    let evt: AcpEvent = serde_json::from_str(legacy).expect("legacy parses");
    match evt {
        AcpEvent::Error {
            session_id,
            message,
            terminal,
        } => {
            assert_eq!(session_id, "s1");
            assert_eq!(message, "boom");
            assert!(!terminal);
        }
        other => panic!("expected Error, got {other:?}"),
    }
    // New shape round-trips.
    let evt = AcpEvent::Error {
        session_id: "s2".into(),
        message: "dead".into(),
        terminal: true,
    };
    let s = serde_json::to_string(&evt).unwrap();
    assert!(s.contains("\"terminal\":true"));
    let back: AcpEvent = serde_json::from_str(&s).unwrap();
    assert!(matches!(back, AcpEvent::Error { terminal: true, .. }));
}
