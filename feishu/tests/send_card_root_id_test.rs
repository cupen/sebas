//! `send_card` `root_id` plumbing: verify `root_id` appears in the HTTP body.
//!
//! The body is built by `FeishuClient::build_send_card_body`, which is tested
//! directly here.  `send_card` itself only adds the URL and forwards the body,
//! so exercising the body builder is sufficient to verify the `root_id` logic.

use feishu::client::FeishuClient;
use feishu::events::SessionKey;

/// Verifies `build_send_card_body` produces the correct full body structure
/// when `root_id` is `Some("msg_parent_1")`.
#[test]
fn build_send_card_body_includes_root_id_when_some() {
    let card_json = serde_json::json!({ "type": "card", "body": "hello" });
    let key = SessionKey {
        chat_id: "oc_chat_1".into(),
        thread_id: None,
    };

    let body = FeishuClient::build_send_card_body(&card_json, &key, Some("msg_parent_1"))
        .expect("build_send_card_body must not fail");

    assert_eq!(
        body.get("receive_id").and_then(|v| v.as_str()),
        Some("oc_chat_1")
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

/// Verifies `build_send_card_body` does NOT include `root_id` when `root_id` is `None`.
#[test]
fn build_send_card_body_excludes_root_id_when_none() {
    let card_json = serde_json::json!({ "type": "card" });
    let key = SessionKey {
        chat_id: "oc_chat_2".into(),
        thread_id: None,
    };

    let body = FeishuClient::build_send_card_body(&card_json, &key, None)
        .expect("build_send_card_body must not fail");

    assert_eq!(
        body.get("receive_id").and_then(|v| v.as_str()),
        Some("oc_chat_2")
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

/// Verifies `build_send_card_body` does NOT include `root_id` when `root_id` is `Some("")`.
/// Feishu rejects an empty-string `root_id` as invalid; it must be treated as None.
#[test]
fn build_send_card_body_excludes_root_id_when_empty_string() {
    let card_json = serde_json::json!({ "type": "card" });
    let key = SessionKey {
        chat_id: "oc_chat_3".into(),
        thread_id: None,
    };

    let body = FeishuClient::build_send_card_body(&card_json, &key, Some(""))
        .expect("build_send_card_body must not fail");

    assert_eq!(
        body.get("receive_id").and_then(|v| v.as_str()),
        Some("oc_chat_3")
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

/// Verifies the full body structure — receive_id, msg_type, content, and root_id
/// are all present and correct when root_id is provided.
#[test]
fn build_send_card_body_full_structure_with_root_id() {
    let card_json = serde_json::json!({
        "header": { "title": { "tag": "plain_text", "content": "Hello" } },
        "elements": [{ "tag": "markdown", "content": "World" }]
    });
    let key = SessionKey {
        chat_id: "oc_full_test".into(),
        thread_id: None,
    };

    let body = FeishuClient::build_send_card_body(&card_json, &key, Some("parent_123"))
        .expect("build_send_card_body must not fail");

    // Structural assertions
    let receive_id = body.get("receive_id").and_then(|v| v.as_str());
    let msg_type = body.get("msg_type").and_then(|v| v.as_str());
    let content_str = body.get("content").and_then(|v| v.as_str());
    let root_id = body.get("root_id").and_then(|v| v.as_str());

    assert_eq!(
        receive_id,
        Some("oc_full_test"),
        "receive_id must match chat_id"
    );
    assert_eq!(
        msg_type,
        Some("interactive"),
        "msg_type must be 'interactive'"
    );
    assert!(
        content_str.is_some(),
        "content must be present and a string"
    );
    assert_eq!(
        root_id,
        Some("parent_123"),
        "root_id must match the provided parent id"
    );

    // Content must be valid JSON containing the card elements
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
