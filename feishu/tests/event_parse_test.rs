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
        FeishuIn::Text {
            text,
            key,
            reply_to,
            ..
        } => {
            assert_eq!(text, "hi");
            // 主线消息：reply target = 触发消息 message_id（Q7 现状不变）。
            assert_eq!(key.thread_id, None);
            assert_eq!(reply_to.as_deref(), Some("om_x"));
        }
        _ => panic!("expected Text"),
    }
}

#[test]
fn topic_message_reply_target_is_root_id() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "tk" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_x" } },
            "message": {
                "chat_id": "oc_topic_group",
                "chat_type": "group",
                "message_id": "om_child",
                "root_id": "om_topic_root",
                "parent_id": "om_topic_root",
                "thread_id": "omt_t1",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}"
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("").unwrap();
    match evt {
        FeishuIn::Text { key, reply_to, .. } => {
            assert_eq!(
                key,
                SessionKey {
                    chat_id: "oc_topic_group".into(),
                    thread_id: Some("omt_t1".into()),
                }
            );
            // 话题内子消息：reply target = 话题根消息 message_id。
            assert_eq!(reply_to.as_deref(), Some("om_topic_root"));
        }
        _ => panic!("expected Text"),
    }
}

#[test]
fn topic_root_message_reply_target_is_own_id() {
    // 话题根消息本身：有 thread_id 但没有 root_id，reply target 回退自身
    // message_id，保证回复仍聚合在该话题。
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "tk" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_x" } },
            "message": {
                "chat_id": "oc_topic_group",
                "chat_type": "group",
                "message_id": "om_topic_root",
                "thread_id": "omt_t1",
                "message_type": "text",
                "content": "{\"text\":\"start\"}"
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("").unwrap();
    match evt {
        FeishuIn::Text { key, reply_to, .. } => {
            assert_eq!(key.thread_id.as_deref(), Some("omt_t1"));
            assert_eq!(reply_to.as_deref(), Some("om_topic_root"));
        }
        _ => panic!("expected Text"),
    }
}

#[test]
fn topic_media_message_carries_reply_target() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "tk" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_x" } },
            "message": {
                "chat_id": "oc_topic_group",
                "chat_type": "group",
                "message_id": "om_child",
                "root_id": "om_topic_root",
                "thread_id": "omt_t1",
                "message_type": "image",
                "content": "{\"image_key\":\"img_1\"}"
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("").unwrap();
    match evt {
        FeishuIn::Media {
            key,
            reply_to,
            files,
            ..
        } => {
            assert_eq!(key.thread_id.as_deref(), Some("omt_t1"));
            assert_eq!(reply_to.as_deref(), Some("om_topic_root"));
            assert_eq!(files, vec!["om_child".to_string()]);
        }
        _ => panic!("expected Media"),
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
    let FeishuIn::ButtonCb { key, action, .. } = evt else {
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
    let FeishuIn::ButtonCb { key, action, .. } = evt else {
        panic!("expected ButtonCb");
    };
    assert_eq!(key.chat_id, "oc_real");
    assert_eq!(action.session_id, "sess_real");
    assert_eq!(action.request_id.as_deref(), Some("req_real"));
    assert_eq!(action.decision.as_deref(), Some("allow_once"));
}

/// Form-container submission: the submit button's custom payload lands in
/// `action.value`, the filled fields in `action.form_value`, and the card's
/// `context.open_message_id` lets the handler flip the card in place.
#[test]
fn parses_form_container_submission_to_formcb() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "card.action.trigger", "tenant_key": "tk" },
        "event": {
            "chat_id": "oc_form",
            "action": {
                "value": { "form": "note", "op": "submit", "id": "n1" },
                "tag": "button",
                "form_value": {
                    "title": "旧标题",
                    "priority": "p0"
                }
            },
            "context": {
                "open_message_id": "om_form",
                "open_chat_id": "oc_form"
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("").expect("form submission parses");
    let FeishuIn::FormCb {
        key,
        value,
        form_value,
        message_id,
        chat_type: _,
    } = evt
    else {
        panic!("expected FormCb");
    };
    assert_eq!(key.chat_id, "oc_form");
    assert_eq!(value["form"], "note");
    assert_eq!(value["op"], "submit");
    assert_eq!(value["id"], "n1");
    assert_eq!(
        form_value.get("title").and_then(serde_json::Value::as_str),
        Some("旧标题")
    );
    assert_eq!(
        form_value
            .get("priority")
            .and_then(serde_json::Value::as_str),
        Some("p0")
    );
    assert_eq!(message_id.as_deref(), Some("om_form"));
}

/// Discriminator regression: routing keys off the *presence* of
/// `action.form_value`, so an all-optional empty submission still parses as
/// a form instead of silently becoming a ButtonCb.
#[test]
fn empty_form_value_object_still_routes_as_formcb() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "card.action.trigger" },
        "event": {
            "chat_id": "oc_form",
            "action": {
                "value": { "form": "note", "op": "submit" },
                "form_value": {}
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("").expect("empty form submission parses");
    match evt {
        FeishuIn::FormCb { form_value, .. } => assert!(form_value.is_empty()),
        other => panic!("expected FormCb, got {other:?}"),
    }
}
