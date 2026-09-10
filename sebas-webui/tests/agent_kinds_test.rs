//! `GET /api/agent-kinds` — the create-session dropdown's reachable agent list.
//! Drives the endpoint in-process with a canned `AgentKindProvider` so the
//! shape is pinned without probing the host's real binaries.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_feishu::cards::CardConfig;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_webui::agent_kinds::{AgentKindInfo, AgentKindProvider};
use sebas_webui::build_router_with_agent_kind_provider;
use sebas_webui::models::RouterInfo;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

/// Canned provider: returns a fixed list, no subprocess probing.
struct CannedProvider {
    kinds: Vec<AgentKindInfo>,
}

#[async_trait::async_trait]
impl AgentKindProvider for CannedProvider {
    async fn agent_kinds(&self) -> Vec<AgentKindInfo> {
        self.kinds.clone()
    }
}

fn info(id: &str, reachable: bool, cause: Option<&str>, version: Option<&str>) -> AgentKindInfo {
    AgentKindInfo {
        id: id.to_string(),
        display: id.to_string(),
        reachable,
        cause: cause.map(str::to_string),
        version: version.map(str::to_string),
    }
}

async fn app_with(kinds: Vec<AgentKindInfo>) -> axum::Router {
    let map = SessionMap::new();
    let (router, _rx) = DispatchHandle::new(map);
    let backend: Arc<dyn sebas_webui::SessionBackend> =
        Arc::new(sebas_webui::session_backend::InProcessBackend::new(router));
    build_router_with_agent_kind_provider(
        backend,
        RouterInfo::default(),
        CardConfig::default(),
        Arc::new(CannedProvider { kinds }),
    )
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("non-JSON body from {uri}: {e}"));
    (status, v)
}

#[tokio::test]
async fn agents_catalog_returns_canned_agents_with_optional_fields() {
    let app = app_with(vec![
        info("claude", true, None, Some("claude v2.1.0")),
        info("gemini", false, Some("command not found"), None),
    ])
    .await;

    let (status, v) = get_json(&app, "/api/agents").await;
    assert_eq!(status, StatusCode::OK);
    let agents = v["agents"].as_array().expect("agents array missing");
    // 2 个配置 agent + 1 行内置 native（catalog 是唯一真源，3.2）。
    assert_eq!(agents.len(), 3);
    let native = agents.iter().find(|a| a["id"] == "native").expect("native row");
    assert_eq!(native["display"], "Native Kernel");

    let claude = agents.iter().find(|a| a["id"] == "claude").expect("claude row");
    let gemini = agents.iter().find(|a| a["id"] == "gemini").expect("gemini row");
    let (claude, gemini) = (claude, gemini);

    assert_eq!(claude["id"], "claude");
    assert_eq!(claude["display"], "claude");
    assert_eq!(claude["reachable"], true);
    assert_eq!(claude["version"], "claude v2.1.0");
    assert!(claude.get("cause").is_none(), "reachable agent must omit cause");
    // driver 是配置层概念，不上 wire（workbench-agent-wire-fix D3）。
    assert!(claude.get("driver").is_none(), "driver must not leak");
    assert!(claude.get("slug").is_none(), "slug is retired vocabulary");

    assert_eq!(gemini["id"], "gemini");
    assert_eq!(gemini["reachable"], false);
    assert_eq!(gemini["cause"], "command not found");
    assert!(gemini.get("version").is_none(), "unreachable agent must omit version");
}

#[tokio::test]
async fn agents_catalog_still_lists_native_when_no_provider_entries() {
    let app = app_with(vec![]).await;
    let (status, v) = get_json(&app, "/api/agents").await;
    assert_eq!(status, StatusCode::OK);
    let agents = v["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1, "native row is always present");
    assert_eq!(agents[0]["id"], "native");
}
