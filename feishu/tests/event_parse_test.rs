use feishu::events::{FeishuEnvelope, FeishuIn, SessionKey};

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
    // Real Feishu V2 card callback: the button's behaviors[].value object is
    // delivered back at event.action.value. This mirrors what
    // cards.rs::render_permission_card writes.
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "card.action.trigger", "tenant_key": "tk" },
        "event": {
            "chat_id": "oc_perm",
            "chat_type": "p2p",
            "thread_id": "omt_thread",
            "action": {
                "value": {
                    "session_id": "sess_42",
                    "request_id": "req_42",
                    "decision": "allow_once"
                }
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("").expect("should parse");
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
    assert_eq!(action.decision.as_deref(), Some("allow_once"));
}

/// Legacy flat layout (/action/session_id + /action/value/decision split)
/// still parses — tolerance against doc-vs-reality drift.
#[test]
fn parses_card_action_trigger_legacy_flat_layout() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "card.action.trigger" },
        "event": {
            "chat_id": "oc_x",
            "action": {
                "session_id": "sess_leg",
                "request_id": "req_leg",
                "value": { "decision": "deny" }
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let FeishuIn::ButtonCb { action, .. } = env.into_event("").expect("parses") else {
        panic!("expected ButtonCb");
    };
    assert_eq!(action.session_id, "sess_leg");
    assert_eq!(action.request_id.as_deref(), Some("req_leg"));
    assert_eq!(action.decision.as_deref(), Some("deny"));
}

/// Missing `action.session_id` is currently defaulted to `""` by
/// `into_event` rather than dropped. Pin that behaviour so future
/// tightening (e.g. returning `None` for half-shaped frames) is
/// intentional and reviewable.
#[test]
fn card_action_trigger_without_session_id_defaults_empty() {
    // Synthesize an envelope whose event.action lacks session_id entirely.
    let json = r#"{
        "schema": "2.0",
        "header": {"event_type": "card.action.trigger"},
        "event": {
            "chat_id": "oc_xyz",
            "action": {
                "value": {"decision": "allow_once"}
            }
        }
    }"#; // no session_id field above on purpose
    let env: FeishuEnvelope = serde_json::from_str(json).expect("parse");
    let evt = env.into_event("");
    // into_event defaults a missing session_id to "" rather than dropping
    // the frame. Pin that so future tightening is intentional.
    let Some(FeishuIn::ButtonCb { action, .. }) = evt else {
        panic!("expected Some(ButtonCb) for missing session_id, got {evt:?}");
    };
    assert_eq!(action.session_id, "");
    assert_eq!(action.decision.as_deref(), Some("allow_once"));
}

/// Feishu's actual card.action.trigger envelope carries the chat id under
/// `/context/open_chat_id`, not `/chat_id` or `/message/chat_id`. Earlier
/// versions of `into_event` only checked the legacy locations, so button
/// clicks were silently dropped ("replay: envelope produced no FeishuIn")
/// and permission cards never advanced. This test pins the v2 layout.
#[test]
fn parses_card_action_trigger_with_context_open_chat_id() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": {
            "event_type": "card.action.trigger",
            "tenant_key": "tk",
            "app_id": "cli_x"
        },
        "event": {
            "operator": {
                "tenant_key": "tk",
                "open_id": "ou_user",
                "union_id": "on_user"
            },
            "token": "c-cardtoken",
            "action": {
                "value": {
                    "decision": "allow_once",
                    "request_id": "req_real",
                    "session_id": "sess_real"
                },
                "tag": "button"
            },
            "host": "im_message",
            "context": {
                "open_message_id": "om_x",
                "open_chat_id": "oc_real"
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("").expect("should parse v2 layout");
    let FeishuIn::ButtonCb { key, action } = evt else {
        panic!("expected ButtonCb");
    };
    assert_eq!(key.chat_id, "oc_real");
    assert_eq!(action.session_id, "sess_real");
    assert_eq!(action.request_id.as_deref(), Some("req_real"));
    assert_eq!(action.decision.as_deref(), Some("allow_once"));
}
