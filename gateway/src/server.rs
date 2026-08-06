//! axum server skeleton for the gateway (Task 2).
//!
//! `build_state` resolves upstream api keys and constructs the shared
//! `reqwest::Client` (connect/read timeouts). `build_router` mounts the
//! liveness probe `GET /healthz` (no auth, `"ok\n"`) and a placeholder
//! `fallback` that returns 501 with a protocol-shaped error body — Task 7
//! swaps the fallback for `proxy::handle`.
//!
//! `run` binds `cfg.listen` and serves with graceful shutdown (ctrl_c +
//! unix SIGTERM). Subsequent tasks append `quota`/`table`/`sink` to
//! `AppState`, each `Arc`-wrapped.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use axum::routing::get;

use crate::auth::require_key;
use crate::config::{GatewayConfig, KeyConfig};
use crate::error::{GatewayError, Result, error_response};
use crate::proto::Protocol;

/// Shared server state. All heavy fields are `Arc`-wrapped so the type is
/// cheaply `Clone` (axum `State<S>` requires `Clone + Send + Sync + 'static`).
/// Later tasks append `quota`/`table`/`sink` (also `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<GatewayConfig>,
    pub keys: Arc<HashMap<String, KeyConfig>>,
    pub api_keys: Arc<HashMap<String, String>>,
    pub client: reqwest::Client,
}

/// Resolve api keys + build the upstream client. Called once at startup.
pub fn build_state(cfg: GatewayConfig) -> Result<AppState> {
    let api_keys = cfg.resolve_api_keys()?;
    let keys: HashMap<String, KeyConfig> = cfg
        .keys
        .iter()
        .map(|k| (k.key.clone(), k.clone()))
        .collect();
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(cfg.connect_timeout_secs))
        // per-read timeout: resets on activity, so long SSE streams that keep
        // emitting are not cut. This is the "read timeout" the spec calls out
        // (§4.3, 放宽到分钟级) — distinct from a hard total `.timeout()`.
        .read_timeout(Duration::from_secs(cfg.read_timeout_secs))
        .build()
        .map_err(|e| GatewayError::Upstream(format!("构建 reqwest client 失败: {e}")))?;
    Ok(AppState {
        cfg: Arc::new(cfg),
        keys: Arc::new(keys),
        api_keys: Arc::new(api_keys),
        client,
    })
}

/// Mount routes. `GET /healthz` + placeholder fallback, both behind the
/// `require_key` auth layer. The layer sits above the fallback so `/healthz`
/// also passes through `require_key`, which exempts it by path. Task 7
/// replaces the placeholder fallback with `proxy::handle`.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .fallback(placeholder)
        .layer(from_fn_with_state(state.clone(), require_key))
        .with_state(state)
}

/// Liveness probe — no auth, no state, returns literal `"ok\n"`.
async fn healthz() -> &'static str {
    "ok\n"
}

/// Pre-routing placeholder: every non-healthz path returns 501 with a
/// protocol-shaped error. The protocol is unknown until Task 3 sniffs it,
/// so we default to Anthropic (the gateway's primary surface). Task 7
/// replaces this handler with `proxy::handle`.
async fn placeholder(_state: State<AppState>) -> Response {
    error_response(
        Protocol::Anthropic,
        StatusCode::NOT_IMPLEMENTED,
        "not_implemented",
        "gateway 未实现该端点的透传（占位 fallback，Task 7 接入 proxy::handle）",
    )
}

/// Bind `cfg.listen`, serve with graceful shutdown (ctrl_c + unix SIGTERM).
pub async fn run(cfg: GatewayConfig) -> Result<()> {
    let listen = cfg.listen.clone();
    let state = build_state(cfg)?;
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!(%listen, "sebas gateway listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Wait for ctrl_c (any platform) or SIGTERM (unix). First signal wins.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl_c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("gateway shutdown signal received, draining connections");
}
