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

fn info(slug: &str, reachable: bool, cause: Option<&str>, version: Option<&str>) -> AgentKindInfo {
    AgentKindInfo {
        name: slug.to_string(),
        slug: slug.to_string(),
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
async fn agent_kinds_returns_canned_kinds_with_optional_fields() {
    let app = app_with(vec![
        info("claude", true, None, Some("claude v2.1.0")),
        info("gemini", false, Some("command not found"), None),
    ])
    .await;

    let (status, v) = get_json(&app, "/api/agent-kinds").await;
    assert_eq!(status, StatusCode::OK);
    let kinds = v["kinds"].as_array().expect("kinds array missing");
    assert_eq!(kinds.len(), 2);

    assert_eq!(kinds[0]["slug"], "claude");
    assert_eq!(kinds[0]["name"], "claude");
    assert_eq!(kinds[0]["reachable"], true);
    assert_eq!(kinds[0]["version"], "claude v2.1.0");
    assert!(kinds[0].get("cause").is_none(), "reachable kind must omit cause");

    assert_eq!(kinds[1]["slug"], "gemini");
    assert_eq!(kinds[1]["reachable"], false);
    assert_eq!(kinds[1]["cause"], "command not found");
    assert!(kinds[1].get("version").is_none(), "unreachable kind must omit version");
}

#[tokio::test]
async fn agent_kinds_empty_when_no_provider_entries() {
    let app = app_with(vec![]).await;
    let (status, v) = get_json(&app, "/api/agent-kinds").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["kinds"].as_array().unwrap().len(), 0);
}
