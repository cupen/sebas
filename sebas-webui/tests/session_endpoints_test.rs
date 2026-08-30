//! Integration tests for session-centric endpoints (`/api/sessions*`,
//! `/api/summary`) against the [`FakeBackend`] — the API contract the SPA
//! depends on: focus handling, status codes, and the close/switch flows.
//! The real core (in-process or socket) is out of scope here; its rejection
//! mapping is what these tests pin down.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_webui::backend::FakeBackend;
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

/// Row builder: every test row is `is_active: false` — the backend never
/// knows about focus.
fn row(key: &str, status: &'static str, session_id: Option<&str>, ago: i64) -> SessionRow {
    let (label, slug, glyph): (&'static str, &'static str, &'static str) = match status {
        "active" => ("Working", "queued", "◎"),
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

/// Backend preloaded with one active (a), one dormant (b), one spawning (c)
/// session, plus the axum app wired to it.
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

async fn post_json(app: &axum::Router, uri: &str, body: Option<String>) -> (StatusCode, Value) {
    let builder = Request::builder().method("POST").uri(uri);
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

#[tokio::test]
async fn switch_marks_the_row_active_and_sorts_it_first() {
    let (_backend, app) = fixture().await;
    let k2 = wire("oc_b");
    let (status, v) = post_json(&app, &format!("/api/sessions/{k2}/switch"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "switched");
    // The redirect uses the encoded key: a usable URL segment, no raw NUL.
    assert_eq!(v["redirect"], format!("/sessions/{k2}"));
    assert_eq!(v["active_session_key"], k2.as_str());

    let (_, list) = get_json(&app, "/api/sessions").await;
    let rows = list["recent_sessions"].as_array().unwrap();
    assert_eq!(rows[0]["encoded_key"], k2.as_str(), "focused row sorts first");
    assert_eq!(rows[0]["is_active"], true);
    // Everyone else keeps backend order.
    assert_eq!(rows[1]["is_active"], false);

    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["active_session_key"], k2.as_str());
    assert_eq!(summary["active_session"]["chat_id"], "oc_b");
}

#[tokio::test]
async fn switch_unknown_key_is_404() {
    let (_backend, app) = fixture().await;
    let (status, v) = post_json(&app, &format!("/api/sessions/{}/switch", wire("oc_ghost")), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn switch_malformed_key_is_rejected() {
    let (_backend, app) = fixture().await;
    let (status, v) = post_json(&app, "/api/sessions/oc_a/switch", None).await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
        "malformed key: got {status}"
    );
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn close_removes_the_session_from_the_list() {
    let (_backend, app) = fixture().await;
    let k1 = wire("oc_a");
    let (status, v) = post_json(&app, &format!("/api/sessions/{k1}/close"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "closed");
    // No focus was set, so it survives as null.
    assert_eq!(v["active_session_key"], Value::Null);

    let (_, list) = get_json(&app, "/api/sessions").await;
    let keys: Vec<&str> = list["recent_sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["encoded_key"].as_str().unwrap())
        .collect();
    assert!(!keys.contains(&k1.as_str()), "closed session is gone: {keys:?}");
    assert_eq!(list["active_count"], 0);
}

#[tokio::test]
async fn close_focused_session_clears_the_focus() {
    let (_backend, app) = fixture().await;
    let k1 = wire("oc_a");
    let (status, _) = post_json(&app, &format!("/api/sessions/{k1}/switch"), None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, v) = post_json(&app, &format!("/api/sessions/{k1}/close"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["active_session_key"], Value::Null, "focus cleared with the closed row");

    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["active_session"], Value::Null);
    assert_eq!(summary["active_session_key"], Value::Null);
}

#[tokio::test]
async fn close_spawning_and_dormant_succeed() {
    let (_backend, app) = fixture().await;
    for key in [wire("oc_b"), wire("oc_c")] {
        let (status, _) = post_json(&app, &format!("/api/sessions/{key}/close"), None).await;
        assert_eq!(status, StatusCode::OK, "close {key}");
    }
}

#[tokio::test]
async fn close_unknown_key_is_404() {
    let (_backend, app) = fixture().await;
    let (status, v) = post_json(&app, &format!("/api/sessions/{}/close", wire("oc_ghost")), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn close_malformed_key_is_rejected() {
    let (_backend, app) = fixture().await;
    let (status, v) = post_json(&app, "/api/sessions/oc_a/close", None).await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND,
        "malformed key: got {status}"
    );
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn detail_read_focuses_the_session() {
    let (_backend, app) = fixture().await;
    let k3 = wire("oc_c");
    let (status, detail) = get_json(&app, &format!("/api/sessions/{k3}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["status"], "spawning");

    // The read focused the session — visible in the summary's pointer.
    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["active_session_key"], k3.as_str());
    assert_eq!(summary["active_session"]["chat_id"], "oc_c");
}

#[tokio::test]
async fn spa_fallback_serves_the_shell() {
    let (_backend, app) = fixture().await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(html.contains("<sebas-app>"), "SPA shell contains the app root");
}

#[tokio::test]
async fn unknown_api_path_is_404() {
    let (_backend, app) = fixture().await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/definitely-not-a-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
