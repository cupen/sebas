//! Integration tests for the JSON API surface (`/api/*`): summary, session
//! list/detail, settings/gateway/about, and the session mutations with the
//! unified `{ "error": ... }` envelope. Drives the [`FakeBackend`] via
//! axum's `oneshot` — no live listener required — and pins the honest
//! degradation contract (`core_connected`) and rejection mapping.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_webui::backend::{FakeBackend, TurnItem};
use sebas_webui::models::{GatewayInfo, SessionRow};
use sebas_webui::build_router;
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

/// Wire-form key for a chat id (the NUL-terminated session key, encoded).
fn wire(chat: &str) -> String {
    urlencoding::encode(&format!("{chat}\0")).into_owned()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn row(key: &str, status: &'static str, session_id: Option<&str>, ago: i64) -> SessionRow {
    let (label, slug, glyph): (&'static str, &'static str, &'static str) = match status {
        "active" => ("Queued", "queued", "◎"),
        "dormant" => ("Dormant", "dormant", "◌"),
        _ => ("Starting", "starting", "◷"),
    };
    SessionRow {
        encoded_key: wire(key),
        chat_id: key.to_string(),
        thread_id: None,
        session_id: session_id.map(str::to_string),
        session_id_short: session_id.map(|s| sebas_webui::models::middle_truncate(s, 18)),
        status,
        status_label: label,
        status_slug: slug,
        status_glyph: glyph,
        last_active: "just now".into(),
        last_active_unix: now_unix() - ago,
        is_active: false,
        project_dir: None,
        prompt_preview: Some(format!("prompt for {key}")),
    }
}

async fn fixture() -> (Arc<FakeBackend>, axum::Router) {
    let backend = Arc::new(FakeBackend::connected());
    backend
        .set_rows(vec![
            row("oc_a", "active", Some("s1"), 5),
            row("oc_b", "dormant", Some("s2"), 120),
            row("oc_c", "spawning", None, 0),
        ])
        .await;
    let app = build_router(backend.clone(), GatewayInfo::default());
    (backend, app)
}

async fn request(app: &axum::Router, method: &str, uri: &str, body: Option<String>) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(b) => builder
            .header("content-type", "application/json")
            .body(Body::from(b))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let v = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("non-JSON response from {uri} [{status}]: {text:?}: {e}"));
    (status, v)
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("non-JSON body from {uri}: {e}"));
    (status, v)
}

#[tokio::test]
async fn summary_returns_counts_uptime_rows_and_core_report() {
    let (_backend, app) = fixture().await;
    let (status, v) = get_json(&app, "/api/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["active_count"], 1);
    assert_eq!(v["dormant_count"], 1);
    assert_eq!(v["spawning_count"], 1);
    assert_eq!(v["total_sessions"], 3);
    assert!(v["uptime"].as_str().is_some());
    // Honest degradation report: connected backend reports so, with no cause.
    assert_eq!(v["core_connected"], true);
    assert_eq!(v["core_cause"], Value::Null);
    // Rows carry the backend-owned status projection.
    let rows = v["recent_sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows[0]["status_slug"].as_str().is_some());
    assert!(rows[0]["status_glyph"].as_str().is_some());
    assert!(rows[0]["status_label"].as_str().is_some());
    // No focus yet: no highlighted row.
    assert_eq!(v["active_session"], Value::Null);
    assert_eq!(v["active_session_key"], Value::Null);
}

/// The degraded half of the honest-degradation contract (task 3.7): an
/// unreachable backend surfaces `core_connected: false` plus its cause.
#[tokio::test]
async fn summary_reports_core_outage_from_backend_reachability() {
    let backend = Arc::new(FakeBackend::unreachable());
    let app = build_router(backend, GatewayInfo::default());
    let (status, v) = get_json(&app, "/api/summary").await;
    assert_eq!(status, StatusCode::OK, "degradation is reported, not a 5xx page");
    assert_eq!(v["core_connected"], false);
    assert!(
        v["core_cause"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
        "cause present: {v}"
    );
}

#[tokio::test]
async fn sessions_list_marks_focused_row_first_and_carries_counts() {
    let (_backend, app) = fixture().await;
    let k1 = wire("oc_a");
    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/sessions/{k1}/switch"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, v) = get_json(&app, "/api/sessions").await;
    assert_eq!(v["active_count"], 1);
    assert_eq!(v["dormant_count"], 1);
    assert_eq!(v["spawning_count"], 1);
    assert_eq!(v["total_sessions"], 3);
    assert_eq!(v["active_session_key"], k1.as_str());

    let rows = v["recent_sessions"].as_array().unwrap();
    assert_eq!(rows[0]["encoded_key"], k1.as_str());
    assert_eq!(rows[0]["is_active"], true);
    assert_eq!(rows[0]["status_slug"], "queued", "backend status passes through");
    assert_eq!(rows[1]["is_active"], false);
    assert_eq!(rows[2]["status_slug"], "starting");
}

#[tokio::test]
async fn session_detail_serves_turns_body_and_sets_focus() {
    let (backend, app) = fixture().await;
    let k1 = wire("oc_a");
    backend
        .push_turn(
            &k1,
            TurnItem { kind: "markdown".into(), content: "hello".into() },
        )
        .await;
    backend
        .push_turn(
            &k1,
            TurnItem { kind: "collapsible".into(), content: "trace".into() },
        )
        .await;

    let (status, v) = get_json(&app, &format!("/api/sessions/{k1}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["chat_id"], "oc_a");
    assert_eq!(v["status"], "active");
    assert_eq!(v["user_prompt"], "prompt for oc_a");
    // The transcript is the backend's turn content in card-element shape.
    let body = v["body"].as_array().unwrap();
    assert_eq!(body.len(), 2);
    assert_eq!(body[0]["element_type"], "markdown");
    assert_eq!(body[0]["content"], "hello");
    assert_eq!(body[1]["element_type"], "collapsible");
    // Feishu-era metadata no longer flows through the session channel.
    assert_eq!(v["msg_id"], Value::Null);
    assert_eq!(v["encoded_key"], k1.as_str());

    // The read focused the session.
    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["active_session_key"], k1.as_str());
}

#[tokio::test]
async fn session_detail_rejects_invalid_and_unknown_keys() {
    let (_backend, app) = fixture().await;
    let (status, v) = get_json(&app, "/api/sessions/notakey").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "unparseable key: {v}");
    assert!(v["error"].as_str().is_some());

    let (status, v) = get_json(&app, &format!("/api/sessions/{}", wire("oc_ghost"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown key: {v}");
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn settings_serve_card_config_and_gateway() {
    let (_backend, app) = fixture().await;
    let (status, v) = get_json(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    let cfg = &v["card_config"];
    assert!(cfg["theme_color"].as_str().is_some());
    assert!(cfg["fold_long_output"].is_boolean());
    assert!(cfg["thinking_display"].as_str().is_some());
    assert!(cfg["max_user_text_chars"].is_u64());
    assert!(cfg["max_tool_output_chars"].is_u64());
    assert_eq!(v["gateway"]["has_auth"], false);
}

#[tokio::test]
async fn gateway_and_about_report_startup_info() {
    let (_backend, app) = fixture().await;
    let (status, v) = get_json(&app, "/api/gateway").await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["gateway"].is_object());

    let (status, v) = get_json(&app, "/api/about").await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["version"].as_str().is_some());
    assert!(v["uptime"].as_str().is_some());
}

#[tokio::test]
async fn create_session_returns_key_and_row_appears() {
    let (backend, app) = fixture().await;
    let (status, v) = request(
        &app,
        "POST",
        "/api/sessions",
        Some(r#"{ "prompt": "hello world" }"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let key = v["key"].as_str().unwrap().to_string();
    assert!(key.starts_with("web-"), "spawned key: {key}");

    // The backend received the call; the new row shows up in the list.
    assert_eq!(backend.spawn_calls().await.len(), 1);
    let (_, list) = get_json(&app, "/api/sessions").await;
    let keys: Vec<&str> = list["recent_sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["encoded_key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&key.as_str()), "spawned row in list: {keys:?}");
    // Creation focuses the new session so the client can navigate to it.
    assert_eq!(list["active_session_key"], key.as_str());
}

/// The unreachable half: mutations never fake success when the core is
/// down — they answer 503 with the cause.
#[tokio::test]
async fn mutations_on_unreachable_core_answer_503() {
    let backend = Arc::new(FakeBackend::unreachable());
    let app = build_router(backend, GatewayInfo::default());

    let (status, v) = request(
        &app,
        "POST",
        "/api/sessions",
        Some(r#"{ "prompt": "hello" }"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert!(v["error"].as_str().is_some());

    let k1 = wire("oc_a");
    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/sessions/{k1}/message"),
        Some(r#"{ "message": "hi" }"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let (status, _) = request(&app, "POST", &format!("/api/sessions/{k1}/close"), None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn send_message_ok_then_maps_unknown_key_to_404() {
    let (backend, app) = fixture().await;
    let k1 = wire("oc_a");
    let (status, v) = request(
        &app,
        "POST",
        &format!("/api/sessions/{k1}/message"),
        Some(r#"{ "message": "hi" }"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let msgs = backend.messages().await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].1, "hi");

    // Unknown keys are typed rejections → 404, not a silent 200.
    let (status, v) = request(
        &app,
        "POST",
        &format!("/api/sessions/{}/message", wire("oc_ghost")),
        Some(r#"{ "message": "hi" }"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{v}");
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn send_message_rejects_invalid_keys_and_empty_bodies() {
    let (_backend, app) = fixture().await;
    // Malformed key.
    let (status, v) = request(
        &app,
        "POST",
        "/api/sessions/oc_a/message",
        Some(r#"{ "message": "hi" }"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    // Missing field: axum's rejection is a plain-text 422.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/message", wire("oc_a")))
                .header("content-type", "application/json")
                .body(Body::from(r#"{ "nope": true }"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn close_returns_status_and_new_focus() {
    let (_backend, app) = fixture().await;
    let k1 = wire("oc_a");
    let (status, v) = request(&app, "POST", &format!("/api/sessions/{k1}/close"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "closed");
    assert_eq!(v["active_session_key"], Value::Null);

    let (status, _) = request(&app, "POST", &format!("/api/sessions/{k1}/close"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "double close");
}

#[tokio::test]
async fn switch_marks_focus_and_returns_redirect() {
    let (_backend, app) = fixture().await;
    let k2 = wire("oc_b");
    let (status, v) = request(&app, "POST", &format!("/api/sessions/{k2}/switch"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "switched");
    assert_eq!(v["redirect"], format!("/sessions/{k2}"));
    assert_eq!(v["active_session_key"], k2.as_str());
}

#[tokio::test]
async fn switch_unknown_key_is_404() {
    let (_backend, app) = fixture().await;
    let (status, v) = request(
        &app,
        "POST",
        &format!("/api/sessions/{}/switch", wire("oc_ghost")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn mutations_reject_non_post_with_405() {
    let (_backend, app) = fixture().await;
    // Note: GET /api/sessions is the *list* endpoint and stays 200; only the
    // mutation-only paths reject non-POST.
    for uri in [
        &format!("/api/sessions/{}/close", wire("oc_a")),
        &format!("/api/sessions/{}/switch", wire("oc_a")),
        &format!("/api/sessions/{}/message", wire("oc_a")),
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED, "GET {uri}");
    }
}
