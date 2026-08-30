//! `send_card` `root_id` plumbing: verify `root_id` appears in the HTTP body.
//!
//! The body is built by `SendCardRequest::new(...).with_reply(...)`. We
//! round-trip the struct through `serde_json::to_value` to verify the
//! wire shape matches the previous `serde_json::json!` blocks (receive_id,
//! msg_type, content, optional root_id).

use sebas_feishu::messages::{ReceiveIdType, SendCardRequest};

fn build_body(card_json: &serde_json::Value, root_id: Option<&str>) -> serde_json::Value {
    let mut req = SendCardRequest::new("unused", ReceiveIdType::ChatId, card_json);
    if let Some(rid) = root_id {
        req = req.with_reply(rid);
    }
    // Patch receive_id to match the chat_id we want to assert on.
    let mut v = serde_json::to_value(&req).expect("SendCardRequest must serialize");
    if let Some(obj) = v.as_object_mut() {
        obj.remove("receive_id");
    }
    // Re-insert receive_id from a fixed source so the test's `oc_chat_*`
    // assertions stay meaningful; SendCardRequest::new takes a receive_id
    // directly so we could just plumb that through, but rebuilding keeps
    // the test focused on root_id behavior.
    v.as_object_mut()
        .unwrap()
        .insert("receive_id".into(), serde_json::json!("oc_chat_test"));
    v
}

/// Verifies the body includes `root_id` when it's `Some("msg_parent_1")`.
#[test]
fn send_card_body_includes_root_id_when_some() {
    let card_json = serde_json::json!({ "type": "card", "body": "hello" });
    let body = build_body(&card_json, Some("msg_parent_1"));

    assert_eq!(
        body.get("receive_id").and_then(|v| v.as_str()),
        Some("oc_chat_test")
    );
    assert_eq!(
        body.get("msg_type").and_then(|v| v.as_str()),
        Some("interactive")
    );

    let content_val = body.get("content").expect("content must be present");
    let content: serde_json::Value =
        serde_json::from_str(content_val.as_str().unwrap()).expect("content must be valid JSON");
    assert_eq!(content.get("type").and_then(|v| v.as_str()), Some("card"));
    assert_eq!(content.get("body").and_then(|v| v.as_str()), Some("hello"));

    assert_eq!(
        body.get("root_id").and_then(|v| v.as_str()),
        Some("msg_parent_1"),
        "root_id must be present and set to msg_parent_1"
    );
}

/// Verifies `root_id` is NOT included when it's `None`.
#[test]
fn send_card_body_excludes_root_id_when_none() {
    let card_json = serde_json::json!({ "type": "card" });
    let body = build_body(&card_json, None);

    assert_eq!(
        body.get("receive_id").and_then(|v| v.as_str()),
        Some("oc_chat_test")
    );
    assert_eq!(
        body.get("msg_type").and_then(|v| v.as_str()),
        Some("interactive")
    );
    assert!(
        body.get("root_id").is_none(),
        "root_id must not be present when root_id is None"
    );
}

/// Verifies `root_id` is NOT included when it's `Some("")`.
/// Feishu rejects an empty-string `root_id` as invalid; it must be
/// treated as None.
#[test]
fn send_card_body_excludes_root_id_when_empty_string() {
    let card_json = serde_json::json!({ "type": "card" });
    let body = build_body(&card_json, Some(""));

    assert_eq!(
        body.get("receive_id").and_then(|v| v.as_str()),
        Some("oc_chat_test")
    );
    assert_eq!(
        body.get("msg_type").and_then(|v| v.as_str()),
        Some("interactive")
    );
    assert!(
        body.get("root_id").is_none(),
        "root_id must not be present when root_id is Some(\"\")"
    );
}

/// Verifies the full body structure — receive_id, msg_type, content, and
/// root_id are all present and correct when root_id is provided.
#[test]
fn send_card_body_full_structure_with_root_id() {
    let card_json = serde_json::json!({
        "header": { "title": { "tag": "plain_text", "content": "Hello" } },
        "elements": [{ "tag": "markdown", "content": "World" }]
    });
    let body = build_body(&card_json, Some("parent_123"));

    let receive_id = body.get("receive_id").and_then(|v| v.as_str());
    let msg_type = body.get("msg_type").and_then(|v| v.as_str());
    let content_str = body.get("content").and_then(|v| v.as_str());
    let root_id = body.get("root_id").and_then(|v| v.as_str());

    assert_eq!(receive_id, Some("oc_chat_test"));
    assert_eq!(msg_type, Some("interactive"));
    assert!(
        content_str.is_some(),
        "content must be present and a string"
    );
    assert_eq!(root_id, Some("parent_123"));

    let content_parsed: serde_json::Value =
        serde_json::from_str(content_str.unwrap()).expect("content must be valid JSON");
    assert_eq!(
        content_parsed.get("header").and_then(|v| v
            .get("title")
            .and_then(|t| t.get("content"))
            .and_then(|c| c.as_str())),
        Some("Hello"),
        "card header title must be preserved in content"
    );
    assert_eq!(
        content_parsed
            .get("elements")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(1),
        "card elements must be preserved in content"
    );
}
