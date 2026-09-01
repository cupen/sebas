//! HTTP integration tests for the session endpoints, driven entirely through
//! the `SessionBackend` seam (task 3.5): no `RouterHandle`, no `SessionManager`,
//! no child process — the fake backend supplies the session set. Uses axum's
//! `oneshot` (no live listener required).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_feishu::events::SessionKey;
use sebas_router::SessionInfo;
use sebas_webui::session_backend::{
    FakeBackend, Reachability, SessionBackend,
};
use sebas_webui::{build_router, init_templates_for_tests};
use std::sync::Arc;
use tower::ServiceExt;

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

fn info(id: &str, status: &str, session_id: Option<&str>) -> SessionInfo {
    SessionInfo {
        chat_id: format!("oc_{id}"),
        thread_id: None,
        session_id: session_id.map(str::to_string),
        status: status.to_string(),
        phase: None,
        user_prompt: None,
        last_active_unix: 0,
        project_dir: None,
    }
}

/// Build a fake backend preloaded with three sessions: one Active (s1),
/// one Dormant (s2), one Spawning placeholder (s3) — plus the app wired
/// against it.
async fn fixture() -> (Arc<FakeBackend>, axum::Router) {
    let backend = Arc::new(FakeBackend::new());
    backend
        .set_sessions(vec![
            info("a", "active", Some("s1")),
            info("b", "dormant", Some("s2")),
            info("c", "spawning", None),
        ])
        .await;
    let templates = Arc::new(init_templates_for_tests());
    let app = build_router(
        backend.clone(),
        sebas_webui::models::GatewayInfo::default(),
        Default::default(),
        templates,
    );
    (backend, app)
}

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn sessions_list_renders_all_sessions_and_buttons() {
    let (_backend, app) = fixture().await;
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
    // class name: it is the contract the stylesheet keys off. oc_a is
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
    let (_backend, app) = fixture().await;
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
    let (_backend, app) = fixture().await;
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
    let (backend, app) = fixture().await;
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

    // The backend now reports k2 as the focused session.
    assert_eq!(
        backend.focused().await,
        Some(k2.clone()),
        "switch must update the backend's focused slot"
    );

    // And the next render of /sessions shows the active-row styling + sidebar card.
    let templates = Arc::new(init_templates_for_tests());
    let app2 = build_router(
        backend.clone(),
        sebas_webui::models::GatewayInfo::default(),
        Default::default(),
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
    let (_backend, app) = fixture().await;
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
    let (backend, app) = fixture().await;
    let k1 = key("a");
    let encoded = encode(&k1);
    let pre = backend.snapshot().await;
    assert!(
        pre.iter().any(|s| s.chat_id == k1.chat_id),
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

    let post = backend.snapshot().await;
    assert!(
        !post.iter().any(|s| s.chat_id == k1.chat_id),
        "close must remove the session from the backend"
    );
}

#[tokio::test]
async fn close_dormant_session_returns_200_without_child_kill() {
    let (backend, app) = fixture().await;
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
        backend.snapshot().await.iter().all(|s| s.chat_id != k2.chat_id),
        "dormant session must drop too"
    );
}

#[tokio::test]
async fn close_spawning_placeholder_returns_200_and_drops_it() {
    let (backend, app) = fixture().await;
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
        backend.snapshot().await.iter().all(|s| s.chat_id != k3.chat_id),
        "spawning placeholder must drop on close"
    );
}

#[tokio::test]
async fn close_unknown_session_returns_404() {
    let (_backend, app) = fixture().await;
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
    let (_backend, app) = fixture().await;
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
    let (backend, app) = fixture().await;
    let k1 = key("a");
    let encoded = encode(&k1);

    backend.set_focus(Some(k1.clone())).await;
    assert_eq!(backend.focused().await, Some(k1.clone()));

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
        backend.focused().await,
        None,
        "closing the focused session must clear the focus"
    );
}

#[tokio::test]
async fn detail_page_visiting_marks_session_active() {
    let (backend, app) = fixture().await;
    let k2 = key("b");
    let encoded = encode(&k2);
    assert_eq!(backend.focused().await, None);

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
        backend.focused().await,
        Some(k2),
        "visiting the detail page must focus the session"
    );
}

/// The detail page renders the transcript from the backend's turn content.
#[tokio::test]
async fn detail_page_renders_transcript_entries() {
    let (backend, app) = fixture().await;
    backend.push_turn("s1", "prompt", "user prompt here").await;
    backend.push_turn("s1", "content", "agent answer").await;
    backend
        .push_turn_typed("s1", "content", "thinking", "hidden thought")
        .await;

    let encoded = encode(&key("a"));
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
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("user prompt here"), "prompt entry missing");
    assert!(body.contains("agent answer"), "content entry missing");
    assert!(body.contains("hidden thought"), "thinking entry missing");
    assert!(
        body.contains("el-thinking"),
        "thinking entries should render in their own class"
    );
}

// ---- Agent project workspace routes (webui/projects) ----

#[tokio::test]
async fn agent_page_renders_sidebar_and_sessions() {
    let (_backend, app) = fixture().await;
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
    let (backend, app) = fixture().await;
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
        backend.focused().await,
        Some(key("a")),
        "visiting agent detail must focus the session"
    );
}

#[tokio::test]
async fn agent_timeline_fragment_returns_partial() {
    let (_backend, app) = fixture().await;
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
    let backend = Arc::new(FakeBackend::new());
    let templates = Arc::new(init_templates_for_tests());
    let app = build_router(
        backend.clone(),
        sebas_webui::models::GatewayInfo::default(),
        Default::default(),
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
    // The spawn reached the backend (spawning placeholder visible).
    assert_eq!(backend.snapshot().await.len(), 1);
}

#[tokio::test]
async fn agent_create_project_rejects_missing_path() {
    let (_backend, app) = fixture().await;
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
    let (backend, app) = {
        let backend = Arc::new(FakeBackend::new());
        backend
            .set_sessions(vec![info("a", "active", Some("s1"))])
            .await;
        let templates = Arc::new(init_templates_for_tests());
        let app = build_router(
            backend.clone(),
            sebas_webui::models::GatewayInfo::default(),
            Default::default(),
            templates,
        );
        (backend, app)
    };
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
    let body = body_string(resp.into_body()).await;
    assert!(!body.contains("<html"), "timeline fragment, not a page");
    let _ = backend;
}

// ---- Honest degradation (7.3): unreachable core rendering ----

#[tokio::test]
async fn unreachable_core_renders_cause_on_board_and_503s_mutations() {
    let (backend, app) = {
        let backend = Arc::new(FakeBackend::new());
        backend
            .set_sessions(vec![info("a", "active", Some("s1"))])
            .await;
        backend.set_reachable(false, "socket absent");
        let templates = Arc::new(init_templates_for_tests());
        let app = build_router(
            backend.clone(),
            sebas_webui::models::GatewayInfo::default(),
            Default::default(),
            templates,
        );
        (backend, app)
    };

    // The board renders the cause verbatim.
    let resp = app.clone()
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
    assert!(
        body.contains("socket absent"),
        "unreachable cause must be stated on the board: {body}"
    );

    // Mutations fail honestly with 503 — never a success.
    let resp = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("prompt=hi"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "create must 503 while unreachable"
    );

    let resp = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/message", encode(&key("a"))))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("message=hi"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let resp = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{}/close", encode(&key("a"))))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    // Nothing was mutated.
    assert_eq!(backend.snapshot().await.len(), 1);
    assert_eq!(backend.reachability().await, Reachability::Unreachable { cause: "socket absent".into() });
}

// ---- SSE (3.4): backend events surface on /events ----

#[tokio::test]
async fn backend_events_appear_on_the_events_stream() {
    let (backend, app) = {
        let backend = Arc::new(FakeBackend::new());
        let templates = Arc::new(init_templates_for_tests());
        let app = build_router(
            backend.clone(),
            sebas_webui::models::GatewayInfo::default(),
            Default::default(),
            templates,
        );
        (backend, app)
    };

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()["content-type"],
        "text/event-stream",
        "SSE content type"
    );

    // Emit from the backend AFTER the SSE subscription is established.
    let key = backend.spawn("sse test".into(), None).await.unwrap();

    // Read a bounded chunk of the never-ending stream.
    use http_body_util::BodyExt as _;
    let mut stream_body = resp.into_body();
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline
        && !seen.contains(&key.chat_id)
    {
        let chunk = tokio::time::timeout(std::time::Duration::from_millis(500), stream_body.frame())
            .await;
        match chunk {
            Ok(Some(Ok(frame))) => {
                let data = frame.into_data().unwrap_or_default();
                seen.push_str(&String::from_utf8_lossy(&data));
                seen.push('\n');
            }
            _ => break,
        }
    }
    assert!(
        seen.contains("event: update"),
        "SSE stream must emit update events, got: {seen}"
    );
    assert!(
        seen.contains(&key.chat_id),
        "the fake backend's Created event must surface on /events, got: {seen}"
    );
}
