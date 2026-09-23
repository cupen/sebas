//! `GET /api/nodes` + project registration with a node dimension + remote
//! session presentation (add-remote-execution-node 8.1/8.2/8.4/8.5).
//!
//! Drives the endpoints in-process against the `FakeBackend` so the node
//! registry is an injected source (never the host's real node link). The
//! project registry is the backend's in-memory `projects` domain
//! (`enable_projects_store`), wired per test instance — `migrate-project-
//! registry` deleted the `projects.json` file backend and retired
//! `SEBAS_PROJECTS_PATH`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_dispatch::{RemoteSessionView, SessionInfo};
use sebas_feishu::cards::CardConfig;
use sebas_webui::models::RouterInfo;
use sebas_webui::server::build_router_with_workspace_root;
use sebas_webui::session_backend::{NodeInfo, PathCheck};
use sebas_webui::{SessionBackend, build_router, session_backend::FakeBackend};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

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

/// 挂载带注入节点的 app。`FakeBackend` 的 projects 域接线为「已注册且可写」
/// 的空注册表：migrate-project-registry 删除文件后端后，注册往返必须走真实
/// 状态源，而不是降级路径。
async fn app_with(nodes: Option<Vec<NodeInfo>>) -> (axum::Router, Arc<FakeBackend>) {
    app_with_root(nodes, None).await
}

/// 同 [`app_with`]，但可把 workspace root 钉到指定目录（add-workspace-root：
/// 本机注册路径必须落在根内——注册临时目录的用例把根钉到该临时目录）。
async fn app_with_root(
    nodes: Option<Vec<NodeInfo>>,
    workspace_root: Option<&std::path::Path>,
) -> (axum::Router, Arc<FakeBackend>) {
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let backend = Arc::new(FakeBackend::new());
    backend.set_nodes(nodes);
    backend.enable_projects_store();
    let dyn_backend: Arc<dyn SessionBackend> = backend.clone();
    let app = match workspace_root {
        Some(root) => build_router_with_workspace_root(
            dyn_backend,
            RouterInfo::default(),
            CardConfig::default(),
            Arc::new(sebas_webui::agent_kinds::ConfigAgentKindProvider::new(
                Vec::new(),
            )),
            Arc::new(sebas_webui::auth::AuthHandle::disabled()),
            root.to_path_buf(),
        ),
        None => build_router(dyn_backend, RouterInfo::default(), CardConfig::default()),
    };
    let _ = router;
    (app, backend)
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
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

// ── GET /api/nodes ────────────────────────────────────────────────────────

#[tokio::test]
async fn nodes_always_lists_the_local_node_as_online() {
    let (app, _backend) = app_with(Some(vec![])).await;
    let (status, body) = send(&app, "GET", "/api/nodes", None).await;
    assert_eq!(status, StatusCode::OK);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1, "空注册表只有本机: {body}");
    assert_eq!(nodes[0]["id"], LOCAL);
    assert_eq!(nodes[0]["status"], "online");
    assert_eq!(nodes[0]["local"], true);
    assert_eq!(body["remote_available"], true);
}

#[tokio::test]
async fn nodes_reports_registry_unavailable_instead_of_no_nodes() {
    // `None` = 后端不承载注册表 → 明确区别于「注册表可达但没有节点」。
    let (app, _backend) = app_with(None).await;
    let (status, body) = send(&app, "GET", "/api/nodes", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["remote_available"], false,
        "不可得必须如实上报: {body}"
    );
    assert!(body["cause"].as_str().is_some_and(|c| !c.is_empty()));
    assert_eq!(body["nodes"].as_array().unwrap().len(), 1, "本机仍在列");
}

#[tokio::test]
async fn nodes_lists_remote_entries_with_status_and_last_seen() {
    let (app, _backend) = app_with(Some(vec![node("dev-box", "offline")])).await;
    let (status, body) = send(&app, "GET", "/api/nodes", None).await;
    assert_eq!(status, StatusCode::OK);
    let nodes = body["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    let remote = nodes.iter().find(|n| n["id"] == "dev-box").unwrap();
    assert_eq!(remote["status"], "offline");
    assert!(remote["last_seen_unix"].is_i64());
}

// ── POST /api/projects with a node dimension (8.1) ────────────────────────

#[tokio::test]
async fn registering_on_a_named_node_is_validated_by_that_node() {
    let (app, backend) =
        app_with(Some(vec![local_node(), node("dev-box", "online")])).await;
    backend.set_path_check(
        "dev-box",
        "/srv/repo",
        Ok(PathCheck {
            exists: true,
            is_dir: true,
            within_workspace: true,
        }),
    );

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
}

#[tokio::test]
async fn node_side_rejection_names_the_node_path_and_problem() {
    let (app, backend) =
        app_with(Some(vec![local_node(), node("dev-box", "online")])).await;
    // 节点说路径不存在（界内）——本用例专测「不存在」文案，containment 判定
    // 置 true；越界拒绝另有专测（node_judged_out_of_workspace_...）。
    backend.set_path_check(
        "dev-box",
        "/srv/nope",
        Ok(PathCheck {
            exists: false,
            is_dir: false,
            within_workspace: true,
        }),
    );

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
}

// ── Node-side containment (add-workspace-root 2.4) ───────────────────────

#[tokio::test]
async fn node_judged_out_of_workspace_rejects_registration() {
    let (app, backend) =
        app_with(Some(vec![local_node(), node("dev-box", "online")])).await;
    // 节点自判：路径存在、是目录，但越出**该节点**的 workspace root。
    backend.set_path_check(
        "dev-box",
        "/srv/secret",
        Ok(PathCheck {
            exists: true,
            is_dir: true,
            within_workspace: false,
        }),
    );

    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": "/srv/secret", "node_id": "dev-box" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    let msg = body["error"].as_str().unwrap();
    assert!(msg.contains("dev-box"), "未点名节点: {msg}");
    assert!(msg.contains("/srv/secret"), "未点名路径: {msg}");
    assert!(msg.contains("workspace root"), "未说明越界: {msg}");

    // 注册被拒 → 一个项目都没建。
    let (_, list) = send(&app, "GET", "/api/projects", None).await;
    assert!(list["projects"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn legacy_node_answer_without_within_workspace_field_still_admits() {
    let (app, backend) =
        app_with(Some(vec![local_node(), node("dev-box", "online")])).await;
    // 老节点应答没有 within_workspace 字段：serde 缺省 true → 放行。
    let check: PathCheck = serde_json::from_value(serde_json::json!({
        "exists": true,
        "is_dir": true,
    }))
    .expect("old-node answer deserializes");
    assert!(
        check.within_workspace,
        "缺字段必须缺省为 true（兼容旧应答）"
    );
    backend.set_path_check("dev-box", "/srv/repo", Ok(check));

    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": "/srv/repo", "node_id": "dev-box" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
}

#[tokio::test]
async fn registering_on_an_offline_node_is_refused_with_the_node_named() {
    let (app, _backend) =
        app_with(Some(vec![local_node(), node("dev-box", "offline")])).await;
    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": "/srv/repo", "node_id": "dev-box" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "body={body}");
    assert!(body["error"].as_str().unwrap().contains("dev-box"));
}

#[tokio::test]
async fn registering_on_an_unknown_node_is_refused() {
    let (app, _backend) = app_with(Some(vec![local_node()])).await;
    let (status, body) = send(
        &app,
        "POST",
        "/api/projects",
        Some(serde_json::json!({ "path": "/srv/repo", "node_id": "ghost" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert!(body["error"].as_str().unwrap().contains("ghost"));
}

#[tokio::test]
async fn node_check_unavailable_is_not_reported_as_a_bad_path() {
    let (app, _backend) =
        app_with(Some(vec![local_node(), node("dev-box", "online")])).await;
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
}

#[tokio::test]
async fn local_registration_without_a_node_keeps_the_implicit_behavior() {
    let dir = tempfile::tempdir().unwrap();
    // add-workspace-root：本机注册必须落在 workspace root 内——把根钉到
    // 临时目录本身，`repo` 即在界内；缺省节点 = 本机的隐式行为不变。
    let (app, _backend) =
        app_with_root(Some(vec![local_node()]), Some(dir.path())).await;
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
        desired_mode: "ask".into(),
        effective_mode: Some("edit".into()),
        msg_count: 0,
        available_commands: Vec::new(),
        // fix-pending-queue-liveness 2.3：远端泊车中的会话同样是回合占用。
        turn_engaged: true,
        spawn_failure_reason: None,
        parked_approvals: 0,
        label: None,
    }
}

#[tokio::test]
async fn remote_session_rows_carry_node_mode_and_wait_when_parked() {
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")])).await;
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
}

#[tokio::test]
async fn a_working_remote_session_without_parked_approvals_still_reads_working() {
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")])).await;
    let mut info = remote_session();
    info.remote.as_mut().unwrap().parked_approvals = 0;
    backend.set_sessions(vec![info]).await;

    let (_, body) = send(&app, "GET", "/api/sessions", None).await;
    let row = &body["recent_sessions"][0];
    assert_eq!(row["status_slug"], "working");
}

// ── Session creation carries the project's node into the seam (8.1 wiring) ──

fn remote_project_state() -> Value {
    serde_json::json!({ "projects": [
        { "id": "proj-r", "path": "/srv/repo", "name": "repo", "node_id": "dev-box", "added_at": 0 }
    ] })
}

#[tokio::test]
async fn session_creation_passes_the_projects_node_to_the_seam() {
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")])).await;
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
}

#[tokio::test]
async fn local_project_session_creation_passes_local() {
    // add-workspace-root 2.3：携本机项目的 create 必须落在 workspace root 内——
    // 把根钉到临时目录，项目路径取其子目录（占位创建不触盘，路径无需存在）。
    let dir = tempfile::tempdir().unwrap();
    let (app, backend) = app_with_root(Some(vec![local_node()]), Some(dir.path())).await;
    backend.set_state_domain(
        "projects",
        Some(serde_json::json!({ "projects": [
            { "id": "proj-l", "path": dir.path().join("repo").to_string_lossy(), "name": "repo", "added_at": 0 }
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
}

#[tokio::test]
async fn remote_project_placeholder_is_honestly_rejected() {
    let (app, backend) = app_with(Some(vec![local_node(), node("dev-box", "online")])).await;
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
}
