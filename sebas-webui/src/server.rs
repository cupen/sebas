//! axum server setup for the WebUI: JSON API + WebSocket + embedded SPA.

use crate::admin::{self, AdminAdapter, AdminState};
use crate::agent_kinds::{AgentKindProvider, AgentKindSource, ConfigAgentKindProvider};
use crate::api;
use crate::assets;
use crate::models::GatewayInfo;
use crate::routes;
use crate::session_backend::SessionBackend;
use axum::Router;
use axum::routing::{get, post};
use axum::serve;
use sebas_feishu::cards::CardConfig;
use std::sync::Arc;
use std::time::Instant;

/// Shared state for the WebUI server. All session data flows through the
/// backend seam — the webui crate never touches `RouterHandle` (the
/// in-process case wraps it inside `InProcessBackend`; the standalone case
/// speaks the core session channel over a Unix socket).
#[derive(Clone)]
pub struct WebUiState {
    pub backend: Arc<dyn SessionBackend>,
    pub gateway: GatewayInfo,
    pub started_at: Instant,
    /// Static snapshot of the card config for the settings page. The session
    /// channel does not transport settings; the caller loads it (from the
    /// local settings.json) at startup.
    pub card_config: CardConfig,
    /// Supplies the reachable agent kinds for the create-session dropdown.
    /// Empty for deployments that never pass a config-driven provider.
    pub agent_kinds: Arc<dyn AgentKindProvider>,
    /// Archive retention period in days. Defaults to 30. Archived sessions
    /// older than this are automatically removed on startup and on list
    /// requests.
    pub archive_retention_days: u64,
}

/// Build the axum Router with all WebUI routes.
pub fn build_router(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
) -> Router {
    build_router_full(
        backend,
        gateway,
        card_config,
        None,
        Arc::new(ConfigAgentKindProvider::new(Vec::new())),
        30,
    )
}

/// Build the axum Router with an explicit agent-kind provider (tests inject a
/// canned provider; production can inject a config-driven one).
pub fn build_router_with_agent_kind_provider(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    agent_kinds: Arc<dyn AgentKindProvider>,
) -> Router {
    build_router_full(backend, gateway, card_config, None, agent_kinds, 30)
}

/// Build the axum Router with optional watchdog admin adapter.
pub fn build_router_with_admin_adapter(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
) -> Router {
    build_router_full(
        backend,
        gateway,
        card_config,
        admin_adapter,
        Arc::new(ConfigAgentKindProvider::new(Vec::new())),
        30,
    )
}

fn build_router_full(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    agent_kinds: Arc<dyn AgentKindProvider>,
    archive_retention_days: u64,
) -> Router {
    let state = WebUiState {
        backend,
        gateway,
        started_at: Instant::now(),
        card_config,
        agent_kinds,
        archive_retention_days,
    };

    // Core SPA + API + WS routes, bound to WebUiState.
    let core = Router::new()
        .route("/", get(assets::index))
        .route("/assets/{*path}", get(assets::asset))
        .route("/health", get(routes::health))
        .route(
            "/api/sessions",
            get(api::sessions_list).post(api::create_session),
        )
        .route("/api/sessions/{key}", get(api::session_detail))
        .route("/api/sessions/{key}/message", post(api::send_message))
        .route("/api/sessions/{key}/close", post(api::close_session))
        .route("/api/sessions/{key}/switch", post(api::switch_session))
        .route("/api/summary", get(api::summary))
        .route(
            "/api/permissions/{request_id}/answer",
            post(api::answer_permission),
        )
        .route("/api/settings", get(api::settings))
        .route("/api/gateway", get(api::gateway))
        .route("/api/about", get(api::about))
        .route("/api/agent-kinds", get(api::agent_kinds))
.route(
            "/api/projects",
            get(api::projects_list).post(api::projects_add),
        )
        .route("/api/fs/browse-dirs", get(api::browse_dirs))
        .route("/api/archive", get(api::archive_list))
        .route("/api/sessions/{key}/archive", post(api::archive_session))
        .route("/api/sessions/{key}/restore", post(api::restore_session))
        .route("/api/projects/reorder", post(api::projects_reorder))
        .route("/api/projects/{path}/remove", post(api::projects_remove))
        .route("/api/projects/{path}/branch", get(api::projects_branch))
        .route("/ws", get(api::ws_handler))
        .with_state(state.clone());

    // gateway BFF mutation 面（Task 6.3）：独立子 router，守卫只套这一组
    // （POST-only + loopback origin check）。
    let gateway_mutations = Router::new()
        .route(
            "/gateway/api/providers",
            post(routes::gateway_api_provider_create),
        )
        .route(
            "/gateway/api/providers/{name}",
            axum::routing::put(routes::gateway_api_provider_update)
                .delete(routes::gateway_api_provider_delete),
        )
        .route(
            "/gateway/api/providers/{name}/probe",
            post(routes::gateway_api_provider_probe),
        )
        .route(
            "/gateway/api/model-aliases",
            post(routes::gateway_api_alias_create),
        )
        .route(
            "/gateway/api/model-aliases/{alias}",
            axum::routing::put(routes::gateway_api_alias_update)
                .delete(routes::gateway_api_alias_delete),
        )
        .route("/gateway/api/reload", post(routes::gateway_api_reload))
        .layer(axum::middleware::from_fn(routes::gateway_mutation_guard))
        .with_state(state);

    // The JSON admin API is always mounted: without an adapter, reads report
    // `adapter_ok: false` and mutations answer 503 (honest degradation).
    // It carries its own AdminState, merged as a stateless Router.
    core.merge(gateway_mutations)
        .merge(admin::build_api_admin_router(AdminState::new(
            admin_adapter,
        )))
        .fallback(assets::spa_fallback)
}

/// Run the WebUI server on the given listener. `agent_kinds` supplies the
/// create-session dropdown's reachable agent list (empty for deployments
/// without config-driven agents).
pub async fn run(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    agent_kinds: Vec<AgentKindSource>,
    listener: tokio::net::TcpListener,
) {
    run_full(backend, gateway, card_config, agent_kinds, listener, None, 30).await;
}

/// Run the WebUI server with an optional watchdog admin adapter.
pub async fn run_with_admin_adapter(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    agent_kinds: Vec<AgentKindSource>,
    listener: tokio::net::TcpListener,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
) {
run_full(
        backend,
        gateway,
        card_config,
        agent_kinds,
        listener,
        admin_adapter,
        30,
    )
    .await;
}

async fn run_full(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    agent_kinds: Vec<AgentKindSource>,
    listener: tokio::net::TcpListener,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    archive_retention_days: u64,
) {
    let provider = Arc::new(ConfigAgentKindProvider::new(agent_kinds));
    let app = build_router_full(backend, gateway, card_config, admin_adapter, provider, archive_retention_days);
    let addr = listener.local_addr().expect("bound listener");
    tracing::info!(%addr, "webui dashboard started");
    if let Err(e) = serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    {
        tracing::error!(error = %e, "webui server error");
    }
}

#[cfg(test)]
mod health_dup_tests {
    // 回归测试（sebas-hsb）：admin router 曾与 core router 重复注册
    // GET /health，merge 时 axum panic「Overlapping method route」。
    // build_router_full 无论是否有 admin adapter 都会 merge admin router，
    // 故此处构建完整 router 即可覆盖该冲突。
    use super::*;
    use crate::models::GatewayInfo;
    use crate::session_backend::FakeBackend;
    use sebas_feishu::cards::CardConfig;

    #[test]
    fn full_router_builds_without_route_conflict() {
        let _app = build_router(
            std::sync::Arc::new(FakeBackend::new()),
            GatewayInfo::default(),
            CardConfig::default(),
        );
    }
}
