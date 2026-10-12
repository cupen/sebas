//! add-claude-model-slots 3.1 集成层：`/api/agents` 管理面的模型档位映射
//! **端点级**端到端路径——真实 axum handler（`agent_create` / `agent_update`）
//! + 真实状态库引擎（`InProcessBackend` → 全局 `StateStoreEngine` →
//! `agents_mutation` 校验）+ 真实 catalog union（`GET /api/agents` 富化）。
//!
//! 与各层单元测试的分工：
//! - `sebas-models` 单测钉 `ModelSlotMapping` 的形状/不变量；
//! - `sebas-dispatch` 单测钉 `agents_mutation` 的校验函数；
//! - 本文件把 3.1 的三条需求钉在**真实 HTTP 端点 + 真实状态库**上：
//!   ① POST/PUT 新形状落库、② GET 回填原始映射（含 default）供前端预填、
//!   ③ 旧裸数组 / 非 claude driver 携带 / default 落在停用档 → typed 拒绝。
//!
//! **单独成文件**（同 `agent_store_probe_test.rs` 的理由）：要装进程级全局
//! 状态引擎（`install_fresh`，串行锁），与「无引擎」前提的其它测试文件同进程
//! 并行会互相污染。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_feishu::cards::CardConfig;
use sebas_webui::agent_kinds::{AgentKindInfo, AgentKindProvider};
use sebas_webui::build_router_with_agent_kind_provider;
use sebas_webui::models::RouterInfo;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

struct EmptyProvider;

#[async_trait::async_trait]
impl AgentKindProvider for EmptyProvider {
    async fn agent_kinds(&self) -> Vec<AgentKindInfo> {
        Vec::new()
    }

    fn default_agent_kind(&self) -> String {
        "claude".to_string()
    }
}

/// 平台无关的「确定存在且可执行」的绝对路径（catalog 探测会对它跑
/// `--version`；探测结果不影响本文件的断言，只要探测不报错即可）。
fn existing_executable() -> String {
    let p = if cfg!(windows) {
        std::env::current_exe().expect("test binary path")
    } else {
        std::path::PathBuf::from("/bin/sh")
    };
    p.to_string_lossy().replace('\\', "/")
}

/// 装全新内存引擎 + 真 InProcessBackend 的 app。返回的 guard 必须绑到测试
/// 局部变量（drop 即清空引擎）。
fn app_with_engine() -> (sebas_dispatch::test_engine::EngineGuard, axum::Router) {
    let guard = sebas_dispatch::test_engine::install_fresh();
    let (router, _rx) = sebas_dispatch::engine::DispatchHandle::new(sebas_dispatch::state::SessionMap::new());
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router),
    );
    let app = build_router_with_agent_kind_provider(
        backend,
        RouterInfo::default(),
        CardConfig::default(),
        Arc::new(EmptyProvider),
    );
    (guard, app)
}

async fn request_json(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn catalog_row<'a>(catalog: &'a Value, id: &str) -> &'a Value {
    catalog["agents"]
        .as_array()
        .unwrap_or_else(|| panic!("agents array missing: {catalog}"))
        .iter()
        .find(|a| a["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("agent '{id}' must be listed: {catalog}"))
}

/// 3.1 主路径：POST 新形状（结构化映射）落库 → GET 回填原始映射（含 default）
/// 供前端编辑表单预填与 new-session 联动。
#[tokio::test]
async fn create_then_catalog_backfills_the_slot_mapping() {
    let (_engine, app) = app_with_engine();

    let (status, body) = request_json(
        &app,
        "POST",
        "/api/agents",
        Some(json!({
            "id": "myclaude",
            "driver": "claude",
            "path": existing_executable(),
            "models": {"opus": "opus-x", "sonnet": "sonnet-y", "default": "sonnet"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create with mapping: {body}");

    let (status, catalog) = request_json(&app, "GET", "/api/agents", None).await;
    assert_eq!(status, StatusCode::OK, "{catalog}");
    let row = catalog_row(&catalog, "myclaude");
    assert_eq!(
        row["models"],
        json!({"opus": "opus-x", "sonnet": "sonnet-y", "default": "sonnet"}),
        "the stored slot mapping (including default) must ride the catalog row: {row}"
    );
}

/// 3.1 主路径：PUT 部分更新携带新映射 → 即时反映到 catalog（免重启生效）。
#[tokio::test]
async fn update_replaces_the_slot_mapping_live() {
    let (_engine, app) = app_with_engine();

    let (status, body) = request_json(
        &app,
        "POST",
        "/api/agents",
        Some(json!({
            "id": "myclaude",
            "driver": "claude",
            "path": existing_executable(),
            "models": {"opus": "opus-x", "default": "opus"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // PUT 只交改动面（合并语义）：display + 新映射。
    let (status, body) = request_json(
        &app,
        "PUT",
        "/api/agents/myclaude",
        Some(json!({
            "display": "My Claude",
            "models": {"fable": "glm-5.3", "sonnet": "sonnet-z", "default": "sonnet"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update mapping: {body}");

    let (_status, catalog) = request_json(&app, "GET", "/api/agents", None).await;
    let row = catalog_row(&catalog, "myclaude");
    assert_eq!(
        row["models"],
        json!({"fable": "glm-5.3", "sonnet": "sonnet-z", "default": "sonnet"}),
        "the updated mapping must be live in the catalog: {row}"
    );
    assert_eq!(row["display"], "My Claude", "{row}");
}

/// 3.1/spec「legacy bare list in config is rejected at parse time」的 wire 对
/// 应面：旧裸数组形态经 mutation 面 typed 拒绝（400），且不落行。
#[tokio::test]
async fn create_with_legacy_bare_list_is_rejected() {
    let (_engine, app) = app_with_engine();

    let (status, body) = request_json(
        &app,
        "POST",
        "/api/agents",
        Some(json!({
            "id": "legacy",
            "driver": "claude",
            "path": existing_executable(),
            "models": ["opus", "sonnet"],
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a legacy bare list must be rejected with 400: {body}"
    );
    let err = body["error"].as_str().unwrap_or_default();
    assert!(err.contains("models"), "the error must name the models field: {body}");

    // 不落行。
    let (_status, catalog) = request_json(&app, "GET", "/api/agents", None).await;
    assert!(
        !catalog.to_string().contains("\"legacy\""),
        "a rejected create must not leave a row: {catalog}"
    );
}

/// spec「invalid mapping is a typed validation error」：default 落在停用档 /
/// 无 default → 400 typed 拒绝，错误点名越界项。
#[tokio::test]
async fn invalid_mappings_are_typed_validation_errors() {
    let (_engine, app) = app_with_engine();

    // default 落在停用档（haiku 未启用）→ 拒绝，错误点名 haiku。
    let (status, body) = request_json(
        &app,
        "POST",
        "/api/agents",
        Some(json!({
            "id": "bad-default",
            "driver": "claude",
            "path": existing_executable(),
            "models": {"opus": "o", "default": "haiku"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap_or_default().contains("haiku"),
        "the error must name the offending slot: {body}"
    );

    // 缺 default 键 → 拒绝。
    let (status, body) = request_json(
        &app,
        "POST",
        "/api/agents",
        Some(json!({
            "id": "no-default",
            "driver": "claude",
            "path": existing_executable(),
            "models": {"opus": "o"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "missing default must be rejected: {body}");
}

/// D6：非 claude driver 携带映射 → typed 拒绝（校验确实发生、错误点名 claude，
/// 且是 400 业务校验拒绝而非传输类 503——review 修复：校验错误补 `put:` 前缀，
/// `routes::map_mutation_error` 才认得出它）。
#[tokio::test]
async fn non_claude_driver_carrying_models_is_a_typed_rejection() {
    let (_engine, app) = app_with_engine();

    let (status, body) = request_json(
        &app,
        "POST",
        "/api/agents",
        Some(json!({
            "id": "oc",
            "driver": "acp",
            "path": "opencode",
            "args": ["acp"],
            "models": {"opus": "o", "default": "opus"},
        })),
    )
    .await;
    // 校验确实发生、错误点名 claude（typed 拒绝的实质）。
    assert!(
        body["error"].as_str().unwrap_or_default().contains("claude"),
        "the error must say only claude may carry models: {body}"
    );
    // spec 契约：业务校验拒绝 = 400（当前实现缺陷下是 503）。
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a business validation rejection must be 400, not a transport-class 503: {body}"
    );
}

