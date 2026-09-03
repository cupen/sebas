//! axum server setup for the WebUI: JSON API + WebSocket + embedded SPA.

use crate::admin::{self, AdminAdapter, AdminState};
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
}

/// Build the axum Router with all WebUI routes.
pub fn build_router(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
) -> Router {
    build_router_with_admin_adapter(backend, gateway, card_config, None)
}

/// Build the axum Router with optional watchdog admin adapter.
pub fn build_router_with_admin_adapter(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
) -> Router {
    let state = WebUiState {
        backend,
        gateway,
        started_at: Instant::now(),
        card_config,
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
        .route("/api/projects", get(api::projects_list).post(api::projects_add))
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
        .merge(admin::build_api_admin_router(AdminState::new(admin_adapter)))
        .fallback(assets::spa_fallback)
}

/// Run the WebUI server on the given listener.
pub async fn run(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    listener: tokio::net::TcpListener,
) {
    run_with_admin_adapter(backend, gateway, card_config, listener, None).await;
}

/// Run the WebUI server with an optional watchdog admin adapter.
pub async fn run_with_admin_adapter(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    listener: tokio::net::TcpListener,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
) {
    let app = build_router_with_admin_adapter(backend, gateway, card_config, admin_adapter);
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
