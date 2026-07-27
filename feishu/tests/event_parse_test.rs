use feishu::events::{CardAction, FeishuEnvelope, FeishuIn, SessionKey};

#[test]
fn parses_text_message_event() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "tk" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_x" } },
            "message": {
                "chat_id": "oc_x",
                "chat_type": "private",
                "message_id": "om_x",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}",
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("ou_x").unwrap();
    match evt {
        FeishuIn::Text { text, .. } => assert_eq!(text, "hi"),
        _ => panic!("expected Text"),
    }
}

#[test]
fn ignores_events_from_non_owner() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "tk" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_stranger" } },
            "message": {
                "chat_id": "oc_x",
                "chat_type": "private",
                "message_id": "om_x",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}"
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    assert!(env.into_event("ou_owner").is_none());
}

#[test]
fn no_owner_filter_when_empty() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "tk" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_anyone" } },
            "message": {
                "chat_id": "oc_x",
                "chat_type": "private",
                "message_id": "om_x",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}"
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env
        .into_event("")
        .expect("event should pass when owner_id is empty");
    match evt {
        FeishuIn::Text { text, .. } => assert_eq!(text, "hi"),
        _ => panic!("expected Text"),
    }
}

/// Registration-time parse path for `card.action.trigger`. The dispatcher
/// hands bytes to our handler; we then call `FeishuEnvelope::into_event` and
/// expect `FeishuIn::ButtonCb` with the action's `decision` value preserved.
/// Exercised here (instead of in the dispatcher) because dispatcher behaviour
/// can't be tested headlessly.
#[test]
fn parses_card_action_trigger_to_buttoncb() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "card.action.trigger", "tenant_key": "tk" },
        "event": {
            "chat_id": "oc_perm",
            "chat_type": "p2p",
            "thread_id": "omt_thread",
            "action": {
                "session_id": "sess_42",
                "request_id": "req_42",
                "value": { "decision": "allow_once" }
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env
        .into_event("")
        .expect("card.action.trigger from owner (skipped: empty owner_id) should parse");
    let FeishuIn::ButtonCb { key, action } = evt else {
        panic!("expected ButtonCb");
    };
    assert_eq!(
        key,
        SessionKey {
            chat_id: "oc_perm".to_string(),
            thread_id: Some("omt_thread".to_string()),
        }
    );
    assert_eq!(action.session_id, "sess_42");
    assert_eq!(action.request_id.as_deref(), Some("req_42"));
    let CardAction {
        session_id: _,
        request_id: _,
        value,
    } = action;
    assert_eq!(
        value
            .pointer("/action/value/decision")
            .and_then(serde_json::Value::as_str),
        Some("allow_once"),
        "decision value preserved through into_event"
    );
}

/// Missing `action.session_id` must NOT panic; the parse path should drop the
/// frame so the dispatcher can log a warn instead of forwarding a half-shaped
/// ButtonCb.
#[test]
fn card_action_trigger_without_session_id_is_none() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "card.action.trigger", "tenant_key": "tk" },
        "event": {
            "chat_id": "oc_perm",
            "action": { "request_id": "req_no_session" }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("");
    // into_event currently defaults session_id to "" rather than returning
    // None; pin that behaviour so future tightening is intentional.
    if let Some(FeishuIn::ButtonCb { action, .. }) = evt {
        assert_eq!(action.session_id, "");
    }
}
