//! axum server setup for the WebUI dashboard.

use crate::admin::{self, AdminAdapter, AdminState};
use crate::models::GatewayInfo;
use crate::routes;
use crate::sse::WebUiEvent;
use acp_claude::manager::SessionManager;
use axum::Router;
use axum::routing::{get, post};
use axum::serve;
use minijinja::Environment;
use router::router::RouterHandle;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

/// Shared state for the WebUI server.
#[derive(Clone)]
pub struct WebUiState {
    pub router: RouterHandle,
    pub mgr: Arc<SessionManager>,
    pub gateway: GatewayInfo,
    pub started_at: Instant,
    pub event_tx: Arc<broadcast::Sender<WebUiEvent>>,
    pub templates: Arc<Environment<'static>>,
}

/// Build the axum Router with all WebUI routes.
pub fn build_router(
    router: RouterHandle,
    mgr: Arc<SessionManager>,
    gateway: GatewayInfo,
    templates: Arc<Environment<'static>>,
) -> Router {
    build_router_with_admin_adapter(router, mgr, gateway, templates, None)
}

/// Build the axum Router with optional watchdog admin adapter.
pub fn build_router_with_admin_adapter(
    router: RouterHandle,
    mgr: Arc<SessionManager>,
    gateway: GatewayInfo,
    templates: Arc<Environment<'static>>,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
) -> Router {
    let (tx, _) = broadcast::channel::<WebUiEvent>(256);
    let state = WebUiState {
        router,
        mgr,
        gateway,
        started_at: Instant::now(),
        event_tx: Arc::new(tx),
        templates: templates.clone(),
    };

    let mut app = Router::new()
        .route("/", get(routes::dashboard))
        .route("/sessions", get(routes::session_list))
        .route("/sessions/partial", get(routes::session_list_partial))
        .route("/sessions/{key}", get(routes::session_detail))
        .route("/settings", get(routes::settings))
        .route("/gateway", get(routes::gateway_page))
        .route("/about", get(routes::about))
        .route("/events", get(crate::sse::event_stream))
        .route("/health", get(routes::health))
        .route("/api/sessions", post(routes::api_create_session))
        .route("/api/sessions/{key}/message", post(routes::api_send_message))
        .route("/api/sessions/{key}/close", post(routes::api_close_session))
        .route("/api/sessions/{key}/switch", post(routes::api_switch_session))
        .nest_service(
            "/static",
            ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/static")),
        )
        .with_state(state);

    if let Some(adapter) = admin_adapter {
        app = app.merge(admin::build_admin_router(AdminState::new(
            Some(adapter),
            templates.clone(),
        )));
    }
    app
}

/// Run the WebUI server on the given listener.
pub async fn run(
    router: RouterHandle,
    mgr: Arc<SessionManager>,
    gateway: GatewayInfo,
    listener: tokio::net::TcpListener,
) {
    run_with_admin_adapter(router, mgr, gateway, listener, None).await;
}

/// Run the WebUI server with an optional watchdog admin adapter.
pub async fn run_with_admin_adapter(
    router: RouterHandle,
    mgr: Arc<SessionManager>,
    gateway: GatewayInfo,
    listener: tokio::net::TcpListener,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
) {
    let templates = Arc::new(init_templates());
    let app = build_router_with_admin_adapter(router, mgr, gateway, templates, admin_adapter);
    let addr = listener.local_addr().expect("bound listener");
    tracing::info!(%addr, "webui dashboard started");
    if let Err(e) = serve(listener, app).await {
        tracing::error!(error = %e, "webui server error");
    }
}

/// Initialize MiniJinja templates embedded at compile time.
fn init_templates() -> Environment<'static> {
    init_templates_inner()
}

/// Public-facing template initializer for integration tests in `tests/`.
#[doc(hidden)]
pub fn init_templates_for_tests() -> Environment<'static> {
    init_templates_inner()
}

fn init_templates_inner() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_template("base.html", include_str!("../templates/base.html"))
        .expect("base.html template");
    env.add_template("index.html", include_str!("../templates/index.html"))
        .expect("index.html template");
    env.add_template("sessions.html", include_str!("../templates/sessions.html"))
        .expect("sessions.html template");
    env.add_template(
        "sessions_partial.html",
        include_str!("../templates/sessions_partial.html"),
    )
    .expect("sessions_partial.html template");
    env.add_template(
        "session_detail.html",
        include_str!("../templates/session_detail.html"),
    )
    .expect("session_detail.html template");
    env.add_template("settings.html", include_str!("../templates/settings.html"))
        .expect("settings.html template");
    env.add_template("gateway.html", include_str!("../templates/gateway.html"))
        .expect("gateway.html template");
    env.add_template("about.html", include_str!("../templates/about.html"))
        .expect("about.html template");
    env.add_template(
        "sidebar_active.html",
        include_str!("../templates/sidebar_active.html"),
    )
    .expect("sidebar_active.html template");
    env.add_template(
        "admin_status.html",
        include_str!("../templates/admin_status.html"),
    )
    .expect("admin_status.html template");
    env.add_template(
        "admin_events.html",
        include_str!("../templates/admin_events.html"),
    )
    .expect("admin_events.html template");
    env.add_template(
        "admin_update.html",
        include_str!("../templates/admin_update.html"),
    )
    .expect("admin_update.html template");
    env.add_template(
        "admin_services.html",
        include_str!("../templates/admin_services.html"),
    )
    .expect("admin_services.html template");
    env
}