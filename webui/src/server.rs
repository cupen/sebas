//! axum server setup for the WebUI dashboard.

use crate::models::GatewayInfo;
use crate::routes;
use crate::sse::WebUiEvent;
use axum::Router;
use axum::routing::get;
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
    pub gateway: GatewayInfo,
    pub started_at: Instant,
    pub event_tx: Arc<broadcast::Sender<WebUiEvent>>,
    pub templates: Arc<Environment<'static>>,
}

/// Build the axum Router with all WebUI routes.
pub fn build_router(
    router: RouterHandle,
    gateway: GatewayInfo,
    templates: Arc<Environment<'static>>,
) -> Router {
    let (tx, _) = broadcast::channel::<WebUiEvent>(256);
    let state = WebUiState {
        router,
        gateway,
        started_at: Instant::now(),
        event_tx: Arc::new(tx),
        templates,
    };

    Router::new()
        .route("/", get(routes::dashboard))
        .route("/sessions", get(routes::session_list))
        .route("/sessions/{key}", get(routes::session_detail))
        .route("/settings", get(routes::settings))
        .route("/gateway", get(routes::gateway_page))
        .route("/about", get(routes::about))
        .route("/events", get(crate::sse::event_stream))
        .route("/health", get(routes::health))
        .route("/api/sessions", axum::routing::post(routes::api_create_session))
        .route("/api/sessions/{key}/message", axum::routing::post(routes::api_send_message))
        .nest_service("/static", ServeDir::new(
            concat!(env!("CARGO_MANIFEST_DIR"), "/static"),
        ))
        .with_state(state)
}

/// Run the WebUI server on the given listener.
pub async fn run(
    router: RouterHandle,
    gateway: GatewayInfo,
    listener: tokio::net::TcpListener,
) {
    let templates = Arc::new(init_templates());
    let app = build_router(router, gateway, templates);
    let addr = listener.local_addr().expect("bound listener");
    tracing::info!(%addr, "webui dashboard started");
    if let Err(e) = serve(listener, app).await {
        tracing::error!(error = %e, "webui server error");
    }
}

/// Initialize MiniJinja templates embedded at compile time.
fn init_templates() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_template("base.html", include_str!("../templates/base.html"))
        .expect("base.html template");
    env.add_template("index.html", include_str!("../templates/index.html"))
        .expect("index.html template");
    env.add_template("sessions.html", include_str!("../templates/sessions.html"))
        .expect("sessions.html template");
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
    env
}