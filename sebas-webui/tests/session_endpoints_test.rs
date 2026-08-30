//! HTTP integration tests for the session-manager endpoints added to the
//! WebUI dashboard: `/sessions/partial`, `POST /api/sessions/{key}/close`,
//! `POST /api/sessions/{key}/switch`. Uses axum's `oneshot` to drive the
//! router in-process — no live listener required.

use sebas_acp_claude::manager::SessionManager;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use sebas_feishu::events::SessionKey;
use http_body_util::BodyExt;
use sebas_router::router::RouterHandle;
use sebas_router::state::{Mapping, SessionMap};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use sebas_webui::models::GatewayInfo;
use sebas_webui::{build_router, init_templates_for_tests};

fn key(id: &str) -> SessionKey {
    SessionKey {
        chat_id: format!("oc_{id}"),
        thread_id: None,
    }
}

fn encode(key: &SessionKey) -> String {
    let raw = format!(
        "{}\0{}",
        key.chat_id,
        key.thread_id.as_deref().unwrap_or("")
    );
    urlencoding::encode(&raw).into_owned()
}

/// Build a RouterHandle preloaded with three sessions: one Active (s1),
/// one Dormant (s2), one Spawning placeholder (s3). Returns the handle
/// and the axum app wired against a stub `SessionManager`.
async fn fixture() -> (RouterHandle, axum::Router) {
    let map = SessionMap::new();
    let k1 = key("a");
    let k2 = key("b");
    let k3 = key("c");
    map.insert(k1.clone(), Mapping::active("s1")).await.unwrap();
    map.insert(k2.clone(), Mapping::dormant("s2", 1))
        .await
        .unwrap();
    map.insert(k3.clone(), Mapping::spawning()).await.unwrap();

    let (router, _rx) = RouterHandle::new(map);
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(5)));
    let templates = Arc::new(init_templates_for_tests());
    let app = build_router(router.clone(), mgr, GatewayInfo::default(), templates);
    (router, app)
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn sessions_list_renders_all_sessions_and_buttons() {
    let (_router, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    // All three fixtures should appear in the table.
    assert!(body.contains("oc_a"), "active session missing from list");
    assert!(body.contains("oc_b"), "dormant session missing from list");
    assert!(body.contains("oc_c"), "spawning session missing from list");
    // Status is asserted through the `data-status` attribute rather than a
    // class name: it is the contract the stylesheet keys off, and it pins the
    // whole derivation path from MappingState to rendered markup. oc_a is
    // Active with no card phase, which must read Queued, not Working.
    assert!(
        body.contains(r#"data-status="queued""#),
        "active-without-phase row should derive Queued"
    );
    assert!(
        body.contains(r#"data-status="dormant""#),
        "dormant row missing derived status"
    );
    assert!(
        body.contains(r#"data-status="starting""#),
        "spawning row should derive Starting"
    );
    // The raw Feishu reaction names must never reach the response.
    for leak in ["OnIt", "CrossMark", ">Get<"] {
        assert!(
            !body.contains(leak),
            "raw card phase {leak:?} leaked into the session list"
        );
    }
    // Per-row Close affordance: the button owns an inline confirm row.
    assert!(
        body.contains(r#"aria-controls="confirm-"#),
        "Close buttons missing from sessions table"
    );
    // Counts live in the status ribbon.
    assert!(body.contains("ribbon-count"), "status ribbon counts missing");
}

#[tokio::test]
async fn sessions_partial_returns_table_without_layout_chrome() {
    let (_router, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/sessions/partial")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    // Partial = table only; no <html> wrapper, no sidebar.
    assert!(
        !body.contains("<html"),
        "partial should not render full layout"
    );
    assert!(
        body.contains("session-table"),
        "partial should render the session table"
    );
    assert!(body.contains("oc_a"));
}

/// Session keys are percent-encoded, so every row id contains a literal `%`
/// (at minimum the `%00` separating chat_id from thread_id). `%` is not a legal
/// CSS identifier character, so `#row-oc_a%00` is a selector *parse error* —
/// htmx resolves `hx-target` through querySelector, so an id-based target on
/// these rows throws and the request is never sent. The bug is invisible to a
/// status-code test: only the rendered markup shows it. Keep it out.
#[tokio::test]
async fn no_hx_attribute_targets_a_percent_encoded_id() {
    let (_router, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/sessions/partial")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_string(resp.into_body()).await;

    for line in body.lines() {
        let line = line.trim();
        if !line.starts_with("hx-target=") {
            continue;
        }
        let value = line.trim_start_matches("hx-target=").trim_matches('"');
        assert!(
            !(value.starts_with('#') && value.contains('%')),
            "hx-target {value:?} is an unparseable CSS selector — htmx will \
             throw resolving it. Target a quoted attribute selector instead."
        );
    }
}

#[tokio::test]
async fn switch_session_marks_active_and_returns_redirect() {
    let (router, app) = fixture().await;
    let k2 = key("b");
    let encoded = encode(&k2);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{encoded}/switch"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(
        body.contains("\"status\":\"switched\""),
        "unexpected switch response: {body}"
    );
    assert!(
        body.contains("/sessions/"),
        "switch response missing redirect URL: {body}"
    );

    // The router now reports k2 as the focused session.
    assert_eq!(
        router.active_session_snapshot().await,
        Some(k2.clone()),
        "switch must update the router's active_session slot"
    );

    // And the next render of /sessions shows the active-row styling + sidebar card.
    let templates = Arc::new(init_templates_for_tests());
    let app2 = build_router(
        router.clone(),
        Arc::new(SessionManager::new(Duration::from_secs(5))),
        GatewayInfo::default(),
        templates,
    );
    let resp2 = app2
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body2 = body_string(resp2.into_body()).await;
    assert!(
        body2.contains(r#"data-focused="true""#),
        "active session row should be marked after switch"
    );
    assert!(
        body2.contains("sidebar-active-card"),
        "sidebar focused-session card missing after switch"
    );
}

#[tokio::test]
async fn switch_unknown_session_returns_404() {
    let (_router, app) = fixture().await;
    let encoded = encode(&SessionKey {
        chat_id: "oc_ghost".into(),
        thread_id: None,
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{encoded}/switch"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "switching an unknown session must 404 (no silent success)"
    );
}

#[tokio::test]
async fn close_active_session_drops_mapping_and_returns_200() {
    let (router, app) = fixture().await;
    let k1 = key("a");
    let encoded = encode(&k1);
    let pre_map = router.session_snapshot().await;
    assert!(
        pre_map.iter().any(|(k, _)| k == &k1),
        "fixture sanity: k1 should be present before close"
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{encoded}/close"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(
        body.contains("\"status\":\"closed\""),
        "unexpected close response: {body}"
    );

    let post_map = router.session_snapshot().await;
    assert!(
        !post_map.iter().any(|(k, _)| k == &k1),
        "close must remove the mapping from the session map"
    );
}

#[tokio::test]
async fn close_dormant_session_returns_200_without_child_kill() {
    let (router, app) = fixture().await;
    let k2 = key("b");
    let encoded = encode(&k2);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{encoded}/close"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    assert!(
        router
            .session_snapshot()
            .await
            .iter()
            .all(|(k, _)| k != &k2),
        "dormant mapping must drop too"
    );
}

#[tokio::test]
async fn close_spawning_placeholder_returns_200_and_drops_it() {
    let (router, app) = fixture().await;
    let k3 = key("c");
    let encoded = encode(&k3);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{encoded}/close"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        router
            .session_snapshot()
            .await
            .iter()
            .all(|(k, _)| k != &k3),
        "spawning placeholder must drop on close"
    );
}

#[tokio::test]
async fn close_unknown_session_returns_404() {
    let (_router, app) = fixture().await;
    let encoded = encode(&SessionKey {
        chat_id: "oc_ghost".into(),
        thread_id: None,
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{encoded}/close"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn close_malformed_key_returns_400() {
    let (_router, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions/not-a-real-key/close")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Axum's Path extractor rejects the route entirely (no key segment
    // boundary) — anything other than 200 is acceptable; we just want the
    // server to refuse rather than panic. Most commonly this surfaces as
    // 404 (router doesn't match) or 400 (decode failure).
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::BAD_REQUEST,
        "malformed key must not crash the server; got {}",
        resp.status()
    );
}

#[tokio::test]
async fn close_focused_session_clears_active_pointer() {
    let (router, app) = fixture().await;
    let k1 = key("a");
    let encoded = encode(&k1);

    router.web_set_active(k1.clone()).await;
    assert_eq!(router.active_session_snapshot().await, Some(k1.clone()));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{encoded}/close"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        router.active_session_snapshot().await,
        None,
        "closing the focused session must clear active_session"
    );
}

#[tokio::test]
async fn detail_page_visiting_marks_session_active() {
    let (router, app) = fixture().await;
    let k2 = key("b");
    let encoded = encode(&k2);
    assert_eq!(router.active_session_snapshot().await, None);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/sessions/{encoded}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        router.active_session_snapshot().await,
        Some(k2),
        "visiting the detail page must web_set_active"
    );
}

// ---- Agent project workspace routes (webui/projects) ----

#[tokio::test]
async fn agent_page_renders_sidebar_and_sessions() {
    let (_router, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/agent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    // Sidebar lists all sessions.
    assert!(body.contains("oc_a"));
    assert!(body.contains("+ New Project"), "New Project button missing");
    assert!(
        body.contains("Open a project to start working"),
        "empty-state prompt missing"
    );
}

#[tokio::test]
async fn agent_detail_focuses_session_and_renders_timeline() {
    let (router, app) = fixture().await;
    let encoded = encode(&key("a"));
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/agent/{encoded}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("type a message") || body.contains("Composer"));
    assert_eq!(
        router.active_session_snapshot().await,
        Some(key("a")),
        "visiting agent detail must focus the session"
    );
}

#[tokio::test]
async fn agent_timeline_fragment_returns_partial() {
    let (_router, app) = fixture().await;
    let encoded = encode(&key("a"));
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/agent/{encoded}/timeline"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(!body.contains("<html"), "timeline should be a partial");
}

#[tokio::test]
async fn agent_create_project_from_valid_path() {
    // 保留 outbound rx（web_spawn 会 emit Out::WebSpawn；rx 被 drop 则
    // channel 关闭导致 panic，而真实运行有消费者）。
    let map = SessionMap::new();
    let (router, _rx) = RouterHandle::new(map);
    let templates = Arc::new(init_templates_for_tests());
    let app = build_router(
        router.clone(),
        Arc::new(SessionManager::new(Duration::from_secs(5))),
        GatewayInfo::default(),
        templates,
    );
    let existed = std::path::Path::new(".");
    let path = existed.canonicalize().unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/projects")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("path={}", urlencoding::encode(path.to_str().unwrap()))))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("\"key\""), "response must carry key: {body}");
}

#[tokio::test]
async fn agent_create_project_rejects_missing_path() {
    let (_router, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/projects")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("path=/nonexistent/definitely-not-here-xyz"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn agent_send_message_returns_timeline() {
    // 保活 outbound rx：active session 的消息会 emit Out::SendAcp。
    let map = SessionMap::new();
    map.insert(key("a"), Mapping::active("s1"))
        .await
        .unwrap();
    let (router, _rx) = RouterHandle::new(map);
    let templates = Arc::new(init_templates_for_tests());
    let app = build_router(
        router.clone(),
        Arc::new(SessionManager::new(Duration::from_secs(5))),
        GatewayInfo::default(),
        templates,
    );
    let encoded = encode(&key("a"));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/{encoded}/message"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("message=hello"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
