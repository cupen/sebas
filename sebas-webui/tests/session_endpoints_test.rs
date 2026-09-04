//! HTTP integration tests for the session APIs: `POST /api/sessions/{key}/close`,
//! `POST /api/sessions/{key}/switch`, and the detail-read focus semantics.
//! Uses axum's `oneshot` to drive the router in-process — no live listener.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_acp::claude::AcpEvent;
use sebas_channels::ChannelKey;
use sebas_feishu::cards::CardConfig;
use sebas_router::router::RouterHandle;
use sebas_router::state::{Mapping, SessionMap};
use sebas_webui::build_router;
use sebas_webui::models::GatewayInfo;
use std::sync::Arc;
use tower::ServiceExt;

fn key(id: &str) -> ChannelKey {
    ChannelKey::feishu(&format!("oc_{id}"), None)
}

/// 与 routes::encode_session_key 同形：`channel\0reference` 后 URL 编码。
fn encode(key: &ChannelKey) -> String {
    urlencoding::encode(&format!("{}\0{}", key.channel.as_str(), key.reference)).into_owned()
}

/// RouterHandle preloaded with one Active (s1), one Dormant (s2), one
/// Spawning (s3) session, plus the axum app wired against the in-process
/// backend seam. The outbound receiver stays alive so `emit` never trips
/// its closed-channel debug assertion.
async fn fixture() -> (
    RouterHandle,
    tokio::sync::mpsc::Receiver<sebas_router::router::Out>,
    axum::Router,
) {
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

async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn switch_session_marks_active_and_returns_redirect() {
    let (router, _rx, app) = fixture().await;
    let k2 = key("b");
    let encoded = encode(&k2);

    let resp = app.clone()
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
    // The redirect is the client route target with the key still
    // percent-encoded (usable as a URL segment).
    assert!(
        body.contains(&format!("/sessions/{encoded}")),
        "switch response missing encoded redirect URL: {body}"
    );

    // The router now reports k2 as the focused session.
    assert_eq!(
        router.active_session_snapshot().await,
        Some(k2.clone()),
        "switch must update the router's active_session slot"
    );

    // And a subsequent /api/sessions list marks the row focused.
    let resp2 = app.clone()
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body2 = body_string(resp2.into_body()).await;
    assert!(
        body2.contains(r#""is_active":true"#),
        "focused session row should be marked after switch: {body2}"
    );
}

#[tokio::test]
async fn switch_unknown_session_returns_404() {
    let (_router, _rx, app) = fixture().await;
    let encoded = encode(&ChannelKey::feishu("oc_ghost", None));
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
    let (router, _rx, app) = fixture().await;
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
    let (router, _rx, app) = fixture().await;
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
    let (router, _rx, app) = fixture().await;
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
    let (_router, _rx, app) = fixture().await;
    let encoded = encode(&ChannelKey::feishu("oc_ghost", None));
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
    let (_router, _rx, app) = fixture().await;
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
    let (router, _rx, app) = fixture().await;
    let k1 = key("a");
    let encoded = encode(&k1);

    router.web_set_active(Some(k1.clone())).await;
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
async fn detail_read_marks_session_active() {
    let (router, _rx, app) = fixture().await;
    let k2 = key("b");
    let encoded = encode(&k2);
    assert_eq!(router.active_session_snapshot().await, None);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{encoded}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        router.active_session_snapshot().await,
        Some(k2),
        "reading the detail API must web_set_active"
    );
}

#[tokio::test]
async fn spa_fallback_serves_entry_for_client_routes() {
    let (_router, _rx, app) = fixture().await;
    // `/` and deep links return the SPA entry document (200, HTML).
    for path in ["/", "/settings", "/sessions", "/admin/status"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "GET {path} must serve SPA");
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("<sebas-app>"), "SPA shell missing for {path}");
    }
    // Unknown API path stays a JSON-ish 404, never the SPA document.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---- Project API integration tests ----

/// Project API tests serialize on this lock because `SEBAS_PROJECTS_PATH` is
/// process-global env state; the guard below restores it even on panic.
static PROJECTS_TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));
static PROJECTS_TEST_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

struct ProjectsEnvGuard {
    _lock: tokio::sync::MutexGuard<'static, ()>,
    prev: Option<String>,
    path: std::path::PathBuf,
}

impl Drop for ProjectsEnvGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        match &self.prev {
            Some(p) => unsafe { std::env::set_var("SEBAS_PROJECTS_PATH", p) },
            None => unsafe { std::env::remove_var("SEBAS_PROJECTS_PATH") },
        }
    }
}

/// Points `SEBAS_PROJECTS_PATH` at a unique throwaway file so project API
/// tests never touch the real `~/.sebas/projects.json`.
async fn isolated_projects() -> ProjectsEnvGuard {
    let lock = PROJECTS_TEST_LOCK.lock().await;
    let n = PROJECTS_TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("sebas-projects-test-{n}.json"));
    let prev = std::env::var("SEBAS_PROJECTS_PATH").ok();
    unsafe { std::env::set_var("SEBAS_PROJECTS_PATH", &path); }
    ProjectsEnvGuard { _lock: lock, prev, path }
}

#[tokio::test]
async fn projects_list_empty() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    assert_eq!(body["projects"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn projects_add_and_list() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let dir = std::env::temp_dir().join("projects-test-add-list");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path_str = dir.to_string_lossy().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "path": path_str }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "add must be 201");
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    assert_eq!(
        body["name"],
        dir.file_name().unwrap().to_string_lossy().to_string()
    );

    // List includes it.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    assert_eq!(body["projects"].as_array().unwrap().len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn projects_add_nonexistent_rejected() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "path": "/bogus-path-xyz" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("不存在"),
        "got: {:?}",
        body
    );
}

#[tokio::test]
async fn projects_add_file_rejected() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let f = std::env::temp_dir().join("projects-test-file.txt");
    std::fs::write(&f, "x").unwrap();
    let path_str = f.to_string_lossy().to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "path": path_str }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_file(&f);
}

#[tokio::test]
async fn projects_remove_project() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let dir = std::env::temp_dir().join("projects-test-remove");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path_str = dir.to_string_lossy().to_string();

    // Add first.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "path": path_str }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Remove.
    let encoded = urlencoding::encode(&path_str);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/projects/{encoded}/remove"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    assert_eq!(body["status"], "removed");

    // List is empty.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    assert_eq!(body["projects"].as_array().unwrap().len(), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn projects_remove_unknown_returns_404() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/nonexistent/remove")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn projects_add_missing_path_field() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(r#"{}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn projects_reorder_persists_user_order() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let mut dirs = Vec::new();
    let mut paths = Vec::new();
    for n in ["ro-a", "ro-b", "ro-c"] {
        let d = std::env::temp_dir().join(format!("projects-test-{n}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        dirs.push(d.clone());
        paths.push(d.to_string_lossy().to_string());
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({ "path": paths.last().unwrap() }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // Reverse via the API.
    let reversed: Vec<String> = paths.iter().rev().cloned().collect();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects/reorder")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "paths": reversed }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    let listed: Vec<String> = body["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(listed, reversed);

    // Persists: a subsequent GET yields the same order.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    let listed: Vec<String> = body["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(listed, reversed);

    for d in dirs {
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[tokio::test]
async fn projects_branch_returns_null_for_non_git_dir() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let d = std::env::temp_dir().join("projects-test-branch-plain");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let path_str = d.to_string_lossy().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "path": &path_str }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let encoded = urlencoding::encode(&path_str);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/projects/{encoded}/branch"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    assert_eq!(body["accessible"], true);
    assert!(body["branch"].is_null(), "non-git must be null: {body}");

    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn projects_branch_detects_git_head() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let d = std::env::temp_dir().join("projects-test-branch-git");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join(".git")).unwrap();
    std::fs::write(d.join(".git/HEAD"), "ref: refs/heads/feature/branch-cache\n").unwrap();
    let path_str = d.to_string_lossy().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "path": &path_str }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let encoded = urlencoding::encode(&path_str);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/projects/{encoded}/branch"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    assert_eq!(body["branch"], "feature/branch-cache");
    assert_eq!(body["accessible"], true);

    let _ = std::fs::remove_dir_all(&d);
}

#[tokio::test]
async fn projects_branch_404_for_unregistered_path() {
    let _env = isolated_projects().await;
    let (_router, _rx, app) = fixture().await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
.uri("/api/projects/never-registered/branch")
        .body(Body::empty())
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn create_session_with_project_dir_binds_to_path() {
    let (router, _rx, app) = fixture().await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"prompt":"hello","project_dir":"/tmp/some-webui-test"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_string(resp.into_body()).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let key_str = v["key"].as_str().expect("key string").to_string();

    // The newly-spawned session's mapping records the project_dir. Web-spawned
    // keys come from `ChannelKey::web_new()` (channel "web", reference
    // "web-{nanos}-{seq}") and live as a Spawning placeholder until the
    // dispatcher promotes them — so identify by a reference starting with
    // "web-" and a status of "spawning".
    let infos = router.session_info_snapshot().await;
    let new_info = infos
        .iter()
        .find(|i| i.channel == "web" && i.key.starts_with("web-") && i.status == "spawning")
        .expect("new web-spawned session info present");
    assert_eq!(new_info.project_dir.as_deref(), Some("/tmp/some-webui-test"));

    // The encoded key round-trips back to the same ChannelKey web_spawn
    // produced: channel "web" + a "web-*" reference.
    let raw = urlencoding::decode(&key_str).unwrap().into_owned();
    let (channel, reference) = raw
        .split_once('\0')
        .expect("encoded key carries the channel\\0reference separator");
    assert_eq!(channel, "web", "encoded key channel; got {raw:?}");
    assert!(
        reference.starts_with("web-"),
        "encoded key should decode to a web-* reference; got {raw:?}"
    );

    // The focus pointer moved to the new session.
    let focused = router.active_session_snapshot().await;
    let focused = focused.expect("focus moved to the new session");
    assert_eq!(focused.channel.as_str(), "web");
    assert!(
        focused.reference.starts_with("web-"),
        "focused reference should match the freshly-created web key; got {:?}",
        focused.reference
    );

    // Sanity: the new key the client received is exactly the focused key,
    // just encoded. Re-encode and compare.
    let reenc = encode(&focused);
    assert_eq!(reenc, key_str);
}

#[tokio::test]
async fn create_session_with_model_threads_spawn_and_mid_session_model_switch_works() {
    let (router, mut rx, app) = fixture().await;

    // 1) create-with-model：POST 带 model → 201，且发出的 Out::WebSpawn 携带
    //    该 model（D3：会话建立后、首 prompt 前应用）。
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"prompt":"hello","model":"pro-model","backend":"acp:opencode"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v: serde_json::Value = serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    let key_str = v["key"].as_str().expect("key string").to_string();

    // web_spawn 经 Out::WebSpawn 出站：断言 model 已透传。
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        rx.recv(),
    )
    .await
    .expect("WebSpawn out within timeout")
    .expect("out channel open");
    match out {
        sebas_router::router::Out::WebSpawn { model, prompt, .. } => {
            assert_eq!(prompt, "hello");
            assert_eq!(model.as_deref(), Some("pro-model"));
        }
        other => panic!("expected Out::WebSpawn carrying model, got {other:?}"),
    }

    // 2) 中程切换模型：POST /api/sessions/{key}/model → 经 Out::SendAcp
    //    送达 SetModel 命令。
    // 先把映射装成 Active（web_spawn 只建 placehholder；activate 需要真实 sid）。
    let decoded = decode_web_key(&key_str);
    router
        .activate(&decoded, "route-s1".into(), Some("acp-real-1".into()), None)
        .await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{key_str}/model"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model_id":"gemini-2.5"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    assert!(body.contains("\"status\":\"ok\""), "unexpected: {body}");

    let out = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("SendAcp out within timeout")
        .expect("out channel open");
    match out {
        sebas_router::router::Out::SendAcp {
            session_id,
            cmd: sebas_acp::AcpCommand::SetModel { model_id, .. },
        } => {
            assert_eq!(session_id, "route-s1");
            assert_eq!(model_id, "gemini-2.5");
        }
        other => panic!("expected Out::SendAcp SetModel, got {other:?}"),
    }
}

// ---- Concurrency across projects (task 6.1) and remove-project semantics (task 6.2) ----

/// Decode an encoded web session key back to its `ChannelKey`. Encoded keys
/// carry the `channel\0reference` separator; web keys are channel "web" with
/// a "web-*" reference.
fn decode_web_key(encoded: &str) -> ChannelKey {
    let raw = urlencoding::decode(encoded).unwrap().into_owned();
    let (channel, reference) = raw
        .split_once('\0')
        .expect("encoded key carries the channel\\0reference separator");
    assert_eq!(channel, "web", "expected a web-* channel; got {channel:?}");
    assert!(
        reference.starts_with("web-"),
        "expected a web-* reference; got {reference:?}"
    );
    ChannelKey::new("web", reference.to_string())
}

/// Canonical absolute path of a temp project dir. The registry canonicalizes
/// on add/remove, so assertions must compare against the same form.
fn canonical_path_str(dir: &tempfile::TempDir) -> String {
    std::fs::canonicalize(dir.path())
        .expect("canonicalize temp dir")
        .to_string_lossy()
        .into_owned()
}

/// POST a JSON document; the content-type header feeds axum's Json extractor.
async fn post_json(
    app: &axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// GET a JSON endpoint as (status, parsed body).
async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    (status, body)
}

/// Spawn a session over HTTP bound to `project_dir`, promote its Spawning
/// placeholder to Active (standing in for the dispatcher's spawn ack), then
/// drive it: one user prompt turn through the composer endpoint plus one
/// agent content turn through the router's ACP event seam. Returns the
/// encoded key and the decoded key.
async fn spawn_and_drive_project_session(
    app: &axum::Router,
    router: &RouterHandle,
    prompt: &str,
    project_dir: &str,
    acp_session_id: &str,
    content: &str,
) -> (String, ChannelKey) {
    let resp = post_json(
        app,
        "/api/sessions",
        serde_json::json!({ "prompt": prompt, "project_dir": project_dir }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "spawn must return 201");
    let encoded: String = {
        let v: serde_json::Value =
            serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
        v["key"]
            .as_str()
            .expect("encoded key in spawn response")
            .to_string()
    };
    let key = decode_web_key(&encoded);

    router.activate(&key, acp_session_id.to_string(), None, None).await;

    // The prompt lands in the transcript (kind "prompt") but is filtered
    // from the rendered detail body.
    let resp = post_json(
        app,
        &format!("/api/sessions/{encoded}/message"),
        serde_json::json!({ "message": prompt }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "message must return 200");

    // The agent content turn is the block the detail view actually renders.
    router
        .apply_event(
            acp_session_id,
            &AcpEvent::TextDelta {
                session_id: acp_session_id.to_string(),
                delta: content.to_string(),
            },
        )
        .await;

    (encoded, key)
}

/// Assert the `after` detail carries the same stable state as `before`:
/// transcript body byte-for-byte plus identity/status fields. `last_active`
/// is a wall-clock relative string, so it is deliberately excluded; content
/// immutability is covered by the turn-count comparison at the call site.
fn assert_detail_untouched(before: &serde_json::Value, after: &serde_json::Value) {
    let body_before = serde_json::to_string(&before["body"]).unwrap();
    let body_after = serde_json::to_string(&after["body"]).unwrap();
    assert_eq!(
        body_before, body_after,
        "detail transcript body must be byte-for-byte unchanged"
    );
    for field in [
        "channel",
        "reference",
        "session_id",
        "status",
        "status_slug",
        "user_prompt",
        "encoded_key",
    ] {
        assert_eq!(
            before[field], after[field],
            "detail field {field:?} must be unchanged"
        );
    }
}

/// Task 6.1: sessions in different projects run simultaneously. Drive
/// project A, switch to B (spawning B moves the focus pointer), drive B,
/// then assert both rows are active under their own project_dir and A's
/// transcript, mapping and rendered detail are untouched.
#[tokio::test]
async fn concurrent_project_sessions_run_simultaneously_and_leave_a_untouched() {
    let (router, _rx, app) = fixture().await;
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let path_a = canonical_path_str(&dir_a);
    let path_b = canonical_path_str(&dir_b);

    // Drive project A end-to-end.
    let (encoded_a, key_a) = spawn_and_drive_project_session(
        &app,
        &router,
        "work on A",
        &path_a,
        "acp-proj-a",
        "A-content-1",
    )
    .await;

    // Baseline of A before B exists: rendered detail, transcript, mapping.
    let (status, detail_before) = get_json(&app, &format!("/api/sessions/{encoded_a}")).await;
    assert_eq!(status, StatusCode::OK);
    let body = detail_before["body"].as_array().expect("body array");
    assert_eq!(body.len(), 1, "one rendered content block: {detail_before}");
    assert_eq!(body[0]["content"], "A-content-1");
    let turns_before = router.session_turns(&key_a, 0).await.unwrap();
    assert_eq!(turns_before.len(), 2, "prompt + content in A's transcript");
    let info_before = router
        .session_info_snapshot()
        .await
        .into_iter()
        .find(|i| i.channel == key_a.channel.as_str() && i.key == key_a.reference)
        .expect("A mapping present");

    // Switch to project B and drive it.
    let (encoded_b, key_b) = spawn_and_drive_project_session(
        &app,
        &router,
        "work on B",
        &path_b,
        "acp-proj-b",
        "B-content-1",
    )
    .await;
    let focused = router
        .active_session_snapshot()
        .await
        .expect("focus pointer set");
    assert_eq!(
        focused, key_b,
        "spawning B must move the focus pointer to B"
    );

    // Both sessions list as active under their own project_dir. The row's
    // project_dir is the grouping key the workbench groups projects by.
    let (_, list) = get_json(&app, "/api/sessions").await;
    let rows = list["recent_sessions"].as_array().expect("session rows");
    let row_for = |encoded: &str| {
        rows.iter()
            .find(|r| r["encoded_key"] == encoded)
            .unwrap_or_else(|| panic!("session {encoded} missing from list: {list}"))
    };
    let row_a = row_for(&encoded_a);
    let row_b = row_for(&encoded_b);
    assert_eq!(row_a["status"], "active");
    assert_eq!(row_b["status"], "active");
    assert_eq!(row_a["project_dir"].as_str(), Some(path_a.as_str()));
    assert_eq!(row_b["project_dir"].as_str(), Some(path_b.as_str()));

    // A is untouched while B ran: same turns, same mapping (project_dir and
    // last_active_unix included), same rendered detail.
    let turns_after = router.session_turns(&key_a, 0).await.unwrap();
    assert_eq!(
        turns_before, turns_after,
        "A's transcript must not change while B is driven"
    );
    let info_after = router
        .session_info_snapshot()
        .await
        .into_iter()
        .find(|i| i.channel == key_a.channel.as_str() && i.key == key_a.reference)
        .expect("A mapping still present");
    assert_eq!(info_before, info_after, "A's mapping must be untouched");
    let (_, detail_after) = get_json(&app, &format!("/api/sessions/{encoded_a}")).await;
    assert_detail_untouched(&detail_before, &detail_after);
}

/// Task 6.2: removing a project leaves its sessions running and reachable.
/// The registry drops the entry, but the session keeps resolving with its
/// transcript intact, and the list still groups it under the origin path —
/// that is how the operator is told where the session moved.
#[tokio::test]
async fn removing_project_keeps_its_session_running_and_reachable() {
    let _env = isolated_projects().await;
    let (router, _rx, app) = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let path = canonical_path_str(&dir);

    // Register the project, then put a live, driven session inside it.
    let resp = post_json(&app, "/api/projects", serde_json::json!({ "path": &path })).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let added: serde_json::Value =
        serde_json::from_str(&body_string(resp.into_body()).await).unwrap();
    assert_eq!(
        added["path"].as_str(),
        Some(path.as_str()),
        "registry stores the canonical path"
    );

    let (encoded, key) = spawn_and_drive_project_session(
        &app,
        &router,
        "project work",
        &path,
        "acp-proj-p",
        "P-content-1",
    )
    .await;
    let (_, detail_before) = get_json(&app, &format!("/api/sessions/{encoded}")).await;
    let turns_before = router.session_turns(&key, 0).await.unwrap();
    assert_eq!(
        turns_before.len(),
        2,
        "live session carries prompt + content"
    );

    // Remove the project from the registry; the session itself is untouched.
    let resp = post_json(
        &app,
        &format!("/api/projects/{}/remove", urlencoding::encode(&path)),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let (_, projects) = get_json(&app, "/api/projects").await;
    assert_eq!(
        projects["projects"].as_array().unwrap().len(),
        0,
        "registry no longer lists the project"
    );

    // The session still resolves: same detail, same transcript.
    let (status, detail_after) = get_json(&app, &format!("/api/sessions/{encoded}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "session must still resolve after project removal"
    );
    assert_detail_untouched(&detail_before, &detail_after);
    let turns_after = router.session_turns(&key, 0).await.unwrap();
    assert_eq!(
        turns_before, turns_after,
        "transcript must survive project removal"
    );

    // The list still shows the session under its origin path: the grouping
    // label is the removed project's path, which is where the operator is
    // told the session lives.
    let (_, list) = get_json(&app, "/api/sessions").await;
    let rows = list["recent_sessions"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["encoded_key"] == encoded)
        .expect("session still listed after project removal");
    assert_eq!(row["status"], "active");
    assert_eq!(row["project_dir"].as_str(), Some(path.as_str()));

    // The board stays reachable (in-process backend) and still counts the
    // session: the three fixture sessions plus this one.
    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["reachability"]["ok"], true);
    assert_eq!(summary["total_sessions"], 4);
}

/// decouple-feishu-channel task 5.1: web 与 feishu 会话在中立 `ChannelKey`
/// 下平级——同一快照可见、同一列表渲染，互不串扰（无跨通道 key 冲突），
/// wire 上只有 `channel`/`reference`，没有飞书形状字段。
#[tokio::test]
async fn web_and_feishu_sessions_are_peers_in_one_snapshot() {
    let map = SessionMap::new();
    let web_key = ChannelKey::new("web", "web-1000");
    let feishu_key = ChannelKey::feishu("oc_peer", Some("om_t"));
    map.insert(web_key.clone(), Mapping::active("s-web"))
        .await
        .unwrap();
    map.insert(feishu_key.clone(), Mapping::spawning())
        .await
        .unwrap();

    let (router, _rx) = RouterHandle::new(map);
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router.clone()),
    );
    let app = build_router(backend, GatewayInfo::default(), CardConfig::default());

    let (status, list) = get_json(&app, "/api/sessions").await;
    assert_eq!(status, StatusCode::OK);
    let rows = list["recent_sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "both channels' sessions listed");

    let web_row = rows
        .iter()
        .find(|r| r["channel"] == "web")
        .expect("web session listed");
    let feishu_row = rows
        .iter()
        .find(|r| r["channel"] == "feishu")
        .expect("feishu session listed");
    assert_eq!(web_row["reference"], "web-1000");
    assert_eq!(feishu_row["reference"], "oc_peer\u{0}om_t");
    // Neutral wire shape: no feishu id fields leak onto the wire.
    assert!(web_row.get("chat_id").is_none());
    assert!(web_row.get("thread_id").is_none());

    // URL keys round-trip per channel without collision.
    assert_ne!(web_row["encoded_key"], feishu_row["encoded_key"]);

    // Focused snapshot includes both channels too.
    let (_, summary) = get_json(&app, "/api/summary").await;
    assert_eq!(summary["total_sessions"], 2);
}
