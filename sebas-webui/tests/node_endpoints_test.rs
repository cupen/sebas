//! `GET /api/nodes` + project registration with a node dimension + remote
//! session presentation (add-remote-execution-node 8.1/8.2/8.4/8.5).
//!
//! Drives the endpoints in-process against the `FakeBackend` so the node
//! registry is an injected source (never the host's real node link), and the
//! remote project registry is redirected to a throwaway file (never the
//! operator's `~/.sebas/projects.json`).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_dispatch::{RemoteSessionView, SessionInfo};
use sebas_feishu::cards::CardConfig;
use sebas_webui::models::RouterInfo;
use sebas_webui::session_backend::{NodeInfo, PathCheck};
use sebas_webui::{SessionBackend, build_router, session_backend::FakeBackend};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use tower::ServiceExt;

/// 进程级 `SEBAS_PROJECTS_PATH` 的测试串行锁（env 是进程全局的）。
static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const LOCAL: &str = "local";

fn node(id: &str, status: &str) -> NodeInfo {
    NodeInfo {
        id: id.to_string(),
        status: status.to_string(),
        last_seen_unix: Some(1_700_000_000),
        created_unix: 1,
        local: false,
    }
}

fn local_node() -> NodeInfo {
    NodeInfo {
        id: LOCAL.to_string(),
        status: "online".into(),
        last_seen_unix: None,
        created_unix: 0,
        local: true,
    }
}

/// 挂载带注入节点的 app；`projects_path` 为 None 时不重定向注册表（只读用例）。
async fn app_with(
    nodes: Option<Vec<NodeInfo>>,
    projects_path: Option<PathBuf>,
) -> (axum::Router, Arc<FakeBackend>) {
    if let Some(p) = projects_path {
        unsafe { std::env::set_var("SEBAS_PROJECTS_PATH", p) };
    }
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let backend = Arc::new(FakeBackend::new());
    backend.set_nodes(nodes);
    let dyn_backend: Arc<dyn SessionBackend> = backend.clone();
    let app = build_router(dyn_backend, RouterInfo::default(), CardConfig::default());
    let _ = router;
    (app, backend)
}

fn cleanup_projects_env() {
    unsafe { std::env::remove_var("SEBAS_PROJECTS_PATH") };
}

async fn send(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

fn guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ── GET /api/nodes ────────────────────────────────────────────────────────

#[tokio::test]
async fn nodes_always_lists_the_local_node_as_online() {
    let _g = guard();
    let (app, _backend) = app_with(Some(vec![]), None).await;
    let (status, body) = send(&app, "GET", "/api/nodes", None).await;
    assert_eq!(status, StatusCode::OK);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1, "空注册表只有本机: {body}");
    assert_eq!(nodes[0]["id"], LOCAL);
    assert_eq!(nodes[0]["status"], "online");
    assert_eq!(nodes[0]["local"], true);
    assert_eq!(body["remote_available"], true);
    cleanup_projects_env();
}

#[tokio::test]
async fn nodes_reports_registry_unavailable_instead_of_no_nodes() {
    let _g = guard();
    // `None` = 后端不承载注册表 → 明确区别于「注册表可达但没有节点」。
    let (app, _backend) = app_with(None, None).await;
    let (status, body) = send(&app, "GET", "/api/nodes", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["remote_available"], false, "不可得必须如实上报: {body}");
    assert!(body["cause"].as_str().is_some_and(|c| !c.is_empty()));
    assert_eq!(body["nodes"].as_array().unwrap().len(), 1, "本机仍在列");
    cleanup_projects_env();
}

#[tokio::test]
async fn nodes_lists_remote_entries_with_status_and_last_seen() {
    let _g = guard();
    let (app, _backend) = app_with(Some(vec![node("dev-box", "offline")]), None).await;
    let (status, body) = send(&app, "GET", "/api/nodes", None).await;
    assert_eq!(status, StatusCode::OK);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    let remote = nodes.iter().find(|n| n["id"] == "dev-box").unwrap();
    assert_eq!(remote["status"], "offline");
    assert!(remote["last_seen_unix"].is_i64());
    cleanup_projects_env();
}

// ── POST /api/projects with a node dimension (8.1) ────────────────────────

#[tokio::test]
async fn registering_on_a_named_node_is_validated_by_that_node() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("projects.json");
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")]), Some(registry)).await;
    backend.set_path_check("dev-box", "/srv/repo", Ok(PathCheck { exists: true, is_dir: true }));

    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": "/srv/repo", "node_id": "dev-box" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["node_id"], "dev-box");
    assert_eq!(body["path"], "/srv/repo");

    // 列表里带节点维度；同样路径换一个节点是**另一个**项目。
    let (_, list) = send(&app, "GET", "/api/projects", None).await;
    let projects = list["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["node_id"], "dev-box");
    cleanup_projects_env();
}

#[tokio::test]
async fn node_side_rejection_names_the_node_path_and_problem() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("projects.json");
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")]), Some(registry)).await;
    // 节点说路径不存在。
    backend.set_path_check("dev-box", "/srv/nope", Ok(PathCheck { exists: false, is_dir: false }));

    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": "/srv/nope", "node_id": "dev-box" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    let msg = body["error"].as_str().unwrap();
    assert!(msg.contains("dev-box"), "未点名节点: {msg}");
    assert!(msg.contains("/srv/nope"), "未点名路径: {msg}");
    assert!(msg.contains("不存在"), "未说明哪里不对: {msg}");

    // 注册被拒 → 一个项目都没建。
    let (_, list) = send(&app, "GET", "/api/projects", None).await;
    assert!(list["projects"].as_array().unwrap().is_empty());
    cleanup_projects_env();
}

#[tokio::test]
async fn registering_on_an_offline_node_is_refused_with_the_node_named() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("projects.json");
    let (app, _backend) = app_with(Some(vec![local_node(), node("dev-box", "offline")]), Some(registry)).await;
    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": "/srv/repo", "node_id": "dev-box" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert!(body["error"].as_str().unwrap().contains("dev-box"));
    cleanup_projects_env();
}

#[tokio::test]
async fn registering_on_an_unknown_node_is_refused() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("projects.json");
    let (app, _backend) = app_with(Some(vec![local_node()]), Some(registry)).await;
    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": "/srv/repo", "node_id": "ghost" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert!(body["error"].as_str().unwrap().contains("ghost"));
    cleanup_projects_env();
}

#[tokio::test]
async fn node_check_unavailable_is_not_reported_as_a_bad_path() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("projects.json");
    let (app, _backend) = app_with(Some(vec![local_node(), node("dev-box", "online")]), Some(registry)).await;
    // 未注入 path check → 后端默认「不能向节点发起路径校验」。
    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": "/srv/repo", "node_id": "dev-box" })),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={body}");
    let msg = body["error"].as_str().unwrap();
    assert!(msg.contains("无法"), "未如实说明校验未完成: {msg}");
    assert!(!msg.contains("不是目录") && !msg.contains("路径不存在"));
    cleanup_projects_env();
}

#[tokio::test]
async fn local_registration_without_a_node_keeps_the_implicit_behavior() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("projects.json");
    let (app, _backend) = app_with(Some(vec![local_node()]), Some(registry)).await;
    let project_dir = dir.path().join("repo");
    std::fs::create_dir_all(&project_dir).unwrap();

    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": project_dir.to_string_lossy() })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["node_id"], LOCAL, "缺省落到本机节点: {body}");
    cleanup_projects_env();
}

// ── Remote session rows: waiting ≠ running, node/mode surfaced ────────────

fn remote_session() -> SessionInfo {
    SessionInfo {
        channel: "web".into(),
        key: "r1".into(),
        session_id: Some("sess-r1".into()),
        status: "active".into(),
        phase: Some("OnIt".into()),
        user_prompt: None,
        last_active_unix: 1_700_000_000,
        project_dir: Some("/srv/repo".into()),
        current_model: None,
        available_models: None,
        agent_kind: Some("claude".into()),
        usage: None,
        backend: Some("acp".into()),
        pending: Vec::new(),
        remote: Some(RemoteSessionView {
            node_id: "dev-box".into(),
            node_status: "online".into(),
            node_cause: None,
            desired_mode: Some("ask".into()),
            effective_mode: Some("edit".into()),
            parked_approvals: 2,
            // 7.1：期望/生效 provider 与成因，同样随行下发。
            desired_provider: Some("work".into()),
            provider: None,
            provider_cause: Some("节点上没有名为 work 的 provider profile".into()),
        }),
    }
}

#[tokio::test]
async fn remote_session_rows_carry_node_mode_and_wait_when_parked() {
    let _g = guard();
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")]), None).await;
    backend.set_sessions(vec![remote_session()]).await;

    let (status, body) = send(&app, "GET", "/api/sessions", None).await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let rows = body["recent_sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    // 8.4：有悬空审批 ⇒ 等待，绝不呈现为 running。
    assert_eq!(row["status_slug"], "waiting", "row={row}");
    assert_eq!(row["status_label"], "Waiting");
    assert_ne!(row["status_slug"], "working");
    // 8.3/8.5：节点与双 mode 透传。
    assert_eq!(row["remote"]["node_id"], "dev-box");
    assert_eq!(row["remote"]["desired_mode"], "ask");
    assert_eq!(row["remote"]["effective_mode"], "edit");
    assert_eq!(row["remote"]["parked_approvals"], 2);
    // 8.1：项目 id 按 `(节点, 路径)` 派生——与本机同路径的 id 不同。
    let remote_id = row["project_id"].as_str().unwrap();
    assert_ne!(
        remote_id,
        sebas_webui::projects::project_id_for("/srv/repo"),
        "远端会话不能挂到本机同路径项目上"
    );
    assert_eq!(
        remote_id,
        sebas_webui::projects::project_id_for_on("dev-box", "/srv/repo")
    );
    cleanup_projects_env();
}

#[tokio::test]
async fn a_working_remote_session_without_parked_approvals_still_reads_working() {
    let _g = guard();
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")]), None).await;
    let mut info = remote_session();
    info.remote.as_mut().unwrap().parked_approvals = 0;
    backend.set_sessions(vec![info]).await;

    let (_, body) = send(&app, "GET", "/api/sessions", None).await;
    let row = &body["recent_sessions"][0];
    assert_eq!(row["status_slug"], "working");
    cleanup_projects_env();
}

// ── Session creation carries the project's node into the seam (8.1 wiring) ──

fn remote_project_state() -> Value {
    serde_json::json!({ "projects": [
        { "id": "proj-r", "path": "/srv/repo", "name": "repo", "node_id": "dev-box", "added_at": 0 }
    ] })
}

#[tokio::test]
async fn session_creation_passes_the_projects_node_to_the_seam() {
    let _g = guard();
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")]), None).await;
    backend.set_state_domain("projects", Some(remote_project_state()));
    let (status, body) = send(
        &app,
        "POST",
        "/api/sessions",
        Some(serde_json::json!({ "prompt": "hi", "project_id": "proj-r", "agent": "claude" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(
        backend.last_spawn_node().as_deref(),
        Some("dev-box"),
        "项目的 node_id 必须传到 SessionBackend::spawn_with"
    );
    cleanup_projects_env();
}

#[tokio::test]
async fn local_project_session_creation_passes_local() {
    let _g = guard();
    let (app, backend) = app_with(Some(vec![local_node()]), None).await;
    backend.set_state_domain(
        "projects",
        Some(serde_json::json!({ "projects": [
            { "id": "proj-l", "path": "/home/me/repo", "name": "repo", "added_at": 0 }
        ] })),
    );
    let (status, _body) = send(
        &app,
        "POST",
        "/api/sessions",
        Some(serde_json::json!({ "prompt": "hi", "project_id": "proj-l", "agent": "claude" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    // 本机项目显式带 "local"：行为与今日逐字一致（不是 None）。
    assert_eq!(backend.last_spawn_node().as_deref(), Some(LOCAL));
    cleanup_projects_env();
}

#[tokio::test]
async fn remote_project_placeholder_is_honestly_rejected() {
    let _g = guard();
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")]), None).await;
    backend.set_state_domain("projects", Some(remote_project_state()));
    let (status, body) = send(
        &app,
        "POST",
        "/api/sessions",
        Some(serde_json::json!({ "project_id": "proj-r", "agent": "claude" })),
    )
    .await;
    // 远端 0-turn 占位没有可对应的远端执行事实：如实拒绝，并指出正确做法。
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={body}");
    let msg = body["error"].as_str().unwrap();
    assert!(msg.contains("占位"), "未说明拒绝的是什么: {msg}");
    assert!(msg.contains("第一条输入"), "未指出正确做法: {msg}");
    cleanup_projects_env();
}
