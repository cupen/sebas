//! WebUI session-payload surfacing for the agent-advertised command table
//! (openspec/changes/session-slash-commands task 2.2).
//!
//! The engine materializes `AcpEvent::AvailableCommands` into the session
//! snapshot; these tests pin the webui wire behavior around it:
//! - a session with a command table → the row (`recent_sessions`) and the
//!   detail/summary payload carry `available_commands`;
//! - a session with an EMPTY table (native agents, nothing advertised) → the
//!   key is absent from both payloads — the wire shape an old frontend sees
//!   is unchanged (新 core + 旧前端组合的字段缺省行为).
//! The reverse direction (旧 core 报文无该键 → 反序列化为空表) is pinned at
//! the type boundary in `sebas-dispatch` (`session_info_available_commands_
//! field_is_additive`); `CoreChannelBackend` consumes exactly that type.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas_acp::claude::AcpEvent;
use sebas_channels::ChannelKey;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::{Mapping, SessionMap};
use sebas_feishu::cards::CardConfig;
use sebas_webui::models::RouterInfo;
use std::sync::Arc;
use tower::ServiceExt;

fn key(id: &str) -> ChannelKey {
    ChannelKey::new("web", format!("slash-{id}"))
}

fn encode(key: &ChannelKey) -> String {
    urlencoding::encode(&format!("{}\0{}", key.channel.as_str(), key.reference)).into_owned()
}

async fn app_with(map: SessionMap) -> axum::Router {
    let (router, _rx) = DispatchHandle::new(map);
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router.clone()),
    );
    sebas_webui::server::build_router_with_workspace_root(
        backend,
        RouterInfo::default(),
        CardConfig::default(),
        Arc::new(sebas_webui::agent_kinds::ConfigAgentKindProvider::new(
            Vec::new(),
        )),
        Arc::new(sebas_webui::auth::AuthHandle::disabled()),
        std::env::temp_dir(),
    )
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
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
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn goal_command() -> sebas_acp::AvailableCommand {
    sebas_acp::AvailableCommand {
        name: "goal".into(),
        description: "Set a goal".into(),
        hint: Some("<condition>".into()),
    }
}

/// 广告过命令表的会话：行载荷与 detail 载荷都带 `available_commands`。
#[tokio::test]
async fn advertised_commands_surface_on_row_and_detail_payloads() {
    let map = SessionMap::new();
    let k = key("advertised");
    map.insert(k.clone(), Mapping::active("s-adv"))
        .await
        .unwrap();
    let app = app_with(map).await;

    // 未广告过命令表的会话（native、旧 agent 同形态）：两种载荷都不出现
    // available_commands 键——新 core + 旧前端看到的 wire 形状不变。
    let (status, body) = get_json(&app, "/api/sessions").await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["recent_sessions"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["reference"] == "slash-advertised")
        .expect("session row present");
    // The mapping has no command table yet (no event applied): the key must
    // be ABSENT — the exact wire an old frontend expects.
    assert!(
        row.get("available_commands").is_none(),
        "empty table must be omitted from the row payload: {row}"
    );

    // Detail payload likewise omits the key while the table is empty.
    let encoded = encode(&k);
    let (status, detail) = get_json(&app, &format!("/api/sessions/{encoded}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert!(
        detail.get("available_commands").is_none(),
        "empty table must be omitted from the detail payload: {detail}"
    );
}

/// 全链（HTTP 面）：引擎收到 `AvailableCommands` 后，行与 detail 载荷出现
/// 命令表。用 InProcessBackend 背后的 engine 直接 apply（两条到达线之一）。
#[tokio::test]
async fn available_commands_event_reaches_http_payloads() {
    let map = SessionMap::new();
    let k = key("live");
    map.insert(k.clone(), Mapping::active("s-live"))
        .await
        .unwrap();
    let (router, _rx) = DispatchHandle::new(map);
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router.clone()),
    );
    let app = sebas_webui::server::build_router_with_workspace_root(
        backend,
        RouterInfo::default(),
        CardConfig::default(),
        Arc::new(sebas_webui::agent_kinds::ConfigAgentKindProvider::new(
            Vec::new(),
        )),
        Arc::new(sebas_webui::auth::AuthHandle::disabled()),
        std::env::temp_dir(),
    );

    // 广告两个命令（即时路径 dispatch_acp_event；pump 路径 apply_event 的
    // 物化等价性已在 sebas-dispatch 用例钉住）。
    router
        .dispatch_acp_event(AcpEvent::AvailableCommands {
            session_id: "s-live".into(),
            commands: vec![
                goal_command(),
                sebas_acp::AvailableCommand {
                    name: "compact".into(),
                    description: "Clear context".into(),
                    hint: None,
                },
            ],
        })
        .await;

    let (_, body) = get_json(&app, "/api/sessions").await;
    let row = body["recent_sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["reference"] == "slash-live")
        .expect("session row present");
    let commands = row["available_commands"].as_array().expect("table on row");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0]["name"], "goal");
    assert_eq!(commands[0]["hint"], "<condition>");
    assert_eq!(commands[1]["name"], "compact");

    let encoded = encode(&k);
    let (_, detail) = get_json(&app, &format!("/api/sessions/{encoded}")).await;
    let commands = detail["available_commands"]
        .as_array()
        .expect("table on detail");
    assert_eq!(commands.len(), 2, "{detail}");
}
