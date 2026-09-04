//! Integration tests for the JSON API surface (`/api/*`): summary, session
//! list/detail, settings/gateway/about, and the session mutations with the
//! unified `{ "error": ... }` envelope. Drives the router in-process via
//! axum's `oneshot` — no live listener required.

use sebas_feishu::cards::CardConfig;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use sebas_channels::ChannelKey;
use http_body_util::BodyExt;
use sebas_router::router::RouterHandle;
use sebas_router::state::{Mapping, SessionMap};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;
use sebas_webui::models::GatewayInfo;
use sebas_webui::build_router;

fn key(id: &str) -> ChannelKey {
    ChannelKey::feishu(&format!("oc_{id}"), None)
}

fn encode(key: &ChannelKey) -> String {
    urlencoding::encode(&format!("{}\0{}", key.channel.as_str(), key.reference)).into_owned()
}

/// RouterHandle preloaded with one Active (s1), one Dormant (s2), one
/// Spawning (s3) session, plus the axum app wired against the in-process
/// backend seam. The second element is the outbound receiver: keeping it
/// alive prevents `RouterHandle::emit`'s closed-channel debug assertion
/// from firing when a test drives the create/message mutations.
async fn fixture() -> (RouterHandle, tokio::sync::mpsc::Receiver<sebas_router::router::Out>, axum::Router) {
    let map = SessionMap::new();
    let k1 = key("a");
    let k2 = key("b");
    let k3 = key("c");
    map.insert(k1.clone(), Mapping::active("s1")).await.unwrap();
    map.insert(k2.clone(), Mapping::dormant("s2", 1))
        .await
        .unwrap();
    map.insert(k3.clone(), Mapping::spawning()).await.unwrap();

    let (router, rx) = RouterHandle::new(map);
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router.clone()),
    );
    let app = build_router(backend, GatewayInfo::default(), CardConfig::default());
    (router, rx, app)
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
async fn summary_returns_counts_uptime_and_rows() {
    let (_router, _rx, app) = fixture().await;
    let (status, v) = get_json(&app, "/api/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["active_count"], 1);
    assert_eq!(v["dormant_count"], 1);
    assert_eq!(v["spawning_count"], 1);
    assert_eq!(v["total_sessions"], 3);
    assert!(v["uptime"].as_str().is_some(), "uptime missing: {v}");
    let rows = v["recent_sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert!(row["encoded_key"].as_str().is_some());
        assert!(row["status_label"].as_str().is_some());
        assert!(row["status_slug"].as_str().is_some());
        assert!(row["status_glyph"].as_str().is_some());
        assert!(row["last_active"].as_str().is_some());
    }
    // No focus has been set yet.
    assert!(v["active_session_key"].is_null());
}

#[tokio::test]
async fn sessions_list_is_active_first_with_status_projection() {
    let (_router, _rx, app) = fixture().await;
    // Focus the active session first: the contract is focused-first, then
    // most-recent activity.
    let encoded_a = encode(&key("a"));
    let (status, _) = request(&app, "POST", &format!("/api/sessions/{encoded_a}/switch"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, v) = get_json(&app, "/api/sessions").await;
    assert_eq!(v["active_session_key"], encoded_a.as_str(), "focus must be set");
    let rows = v["recent_sessions"].as_array().unwrap();
    let first = rows[0]["reference"].as_str().unwrap();
    assert_eq!(first, "oc_a", "focused session must sort first: {v}");
    assert_eq!(rows[0]["is_active"], true);
    // The others are not focused; recency puts the dormant fixture (ts=1)
    // after everything created "just now".
    let later: Vec<&str> = rows[1..]
        .iter()
        .map(|r| r["reference"].as_str().unwrap())
        .collect();
    assert!(later.contains(&"oc_b"), "dormant fixture missing: {v}");
    // Backend-owned status projection: slug in the known set, matching
    // label, and a distinct glyph (shape channel, not colour-only).
    let slugs: Vec<&str> = rows
        .iter()
        .map(|r| r["status_slug"].as_str().unwrap())
        .collect();
    for slug in &slugs {
        assert!(
            ["starting", "queued", "working", "done", "failed", "dormant"]
                .contains(slug),
            "unknown status slug {slug}"
        );
    }
    assert_eq!(rows[0]["status_slug"], "queued", "active without phase reads Queued");
    // Numeric recency order: the spawning fixture (just created) precedes
    // the dormant one (timestamp 1), regardless of rendered "…d ago" text.
    assert_eq!(rows[1]["status_slug"], "starting");
    assert_eq!(rows[2]["status_slug"], "dormant");
    assert_ne!(rows[0]["status_glyph"], rows[1]["status_glyph"]);
}
#[tokio::test]
async fn session_detail_returns_payload_and_sets_focus() {
    let (_router, _rx, app) = fixture().await;
    let encoded = encode(&key("a"));
    let (status, v) = get_json(&app, &format!("/api/sessions/{encoded}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["reference"], "oc_a");
    assert_eq!(v["session_id"], "s1");
    assert!(v["status_slug"].as_str().is_some());
    assert!(v["body"].is_array(), "card body must be a list: {v}");
    assert!(v["last_active"].as_str().is_some());
    assert_eq!(v["encoded_key"], encoded.as_str());

    // The read focuses the session — a display pointer only.
    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["active_session_key"], encoded.as_str());
}

#[tokio::test]
async fn session_detail_rejects_invalid_and_unknown_keys() {
    let (_router, _rx, app) = fixture().await;
    // A key with no embedded NUL separator cannot decode.
    let (status, v) = get_json(&app, "/api/sessions/notakey").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().is_some(), "error envelope missing: {v}");

    let encoded = encode(&key("zz"));
    let (status, v) = get_json(&app, &format!("/api/sessions/{encoded}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn settings_gateway_about_expose_page_data() {
    let (_router, _rx, app) = fixture().await;
    let (status, v) = get_json(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["card_config"].is_object(), "card_config missing: {v}");
    assert!(v["card_config"]["theme_color"].as_str().is_some());
    assert!(v["gateway"].is_object());

    let (status, v) = get_json(&app, "/api/gateway").await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["gateway"].is_object());
    assert!(v["gateway"]["provider_count"].is_u64());

    let (status, v) = get_json(&app, "/api/about").await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["version"].as_str().is_some());
    assert!(v["uptime"].as_str().is_some());
    assert!(v["provider_count"].is_u64());
}

#[tokio::test]
async fn create_session_returns_201_with_key() {
    let (_router, _rx, app) = fixture().await;
    let (status, v) = request(
        &app,
        "POST",
        "/api/sessions",
        Some(r#"{"prompt": "hello"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body: {v}");
    let key = v["key"].as_str().expect("created key missing");
    // The key round-trips: it appears in the list.
    let (_, list) = get_json(&app, "/api/sessions").await;
    let keys: Vec<&str> = list["recent_sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["encoded_key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&key), "created session missing from list");
}

#[tokio::test]
async fn send_message_and_error_envelope() {
    let (_router, _rx, app) = fixture().await;
    let encoded = encode(&key("a"));
    let (status, v) = request(
        &app,
        "POST",
        &format!("/api/sessions/{encoded}/message"),
        Some(r#"{"message": "hi"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "ok");

    // Invalid key → 400 with the error envelope.
    let (status, v) = request(
        &app,
        "POST",
        "/api/sessions/notakey/message",
        Some(r#"{"message": "hi"}"#.into()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn close_session_semantics_over_json() {
    let (_router, _rx, app) = fixture().await;
    // Unknown key → 404, nothing mutated.
    let (status, v) = request(&app, "POST", &format!("/api/sessions/{}/close", encode(&key("zz"))), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(v["error"].as_str().is_some());

    // Dormant mapping drops without a kill.
    let (status, v) = request(&app, "POST", &format!("/api/sessions/{}/close", encode(&key("b"))), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "closed");

    let (_, list) = get_json(&app, "/api/sessions").await;
    let keys: Vec<&str> = list["recent_sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["reference"].as_str().unwrap())
        .collect();
    assert!(!keys.contains(&"oc_b"), "closed session still listed");
}

#[tokio::test]
async fn switch_session_returns_route_and_focuses() {
    let (_router, _rx, app) = fixture().await;
    let encoded = encode(&key("b"));
    let (status, v) = request(&app, "POST", &format!("/api/sessions/{encoded}/switch"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["redirect"], format!("/sessions/{encoded}"));

    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["active_session_key"], encoded.as_str());

    // Unknown key → 404 so the client never navigates to a dead view.
    let (status, v) = request(&app, "POST", &format!("/api/sessions/{}/switch", encode(&key("zz"))), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(v["error"].as_str().is_some());
}

#[tokio::test]
async fn mutations_reject_non_post_with_405() {
    let (_router, _rx, app) = fixture().await;
    // Note: GET /api/sessions is the *list* endpoint and stays 200; only the
    // mutation-only paths reject non-POST.
    for uri in [
        "/api/sessions/oc_a/close",
        "/api/sessions/oc_a/switch",
        "/api/sessions/oc_a/message",
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
