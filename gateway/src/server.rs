//! axum server skeleton for the gateway (Task 2).
//!
//! `build_state` resolves upstream api keys, constructs the shared
//! `reqwest::Client` (connect/read timeouts), and builds the `RouteTable`.
//! `build_router` mounts the liveness probe `GET /healthz` (no auth,
//! `"ok\n"`) and the `proxy::handle` fallback for everything else, both
//! behind the `require_key` auth layer.
//!
//! `run` binds `cfg.listen` and serves with graceful shutdown (ctrl_c +
//! unix SIGTERM).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::routing::get;

use crate::auth::require_key;
use crate::config::{GatewayConfig, KeyConfig};
use crate::error::{GatewayError, Result};
use crate::proxy;
use crate::quota::Quota;
use crate::routing::RouteTable;
use crate::usage::UsageSink;

/// Shared server state. All heavy fields are `Arc`-wrapped so the type is
/// cheaply `Clone` (axum `State<S>` requires `Clone + Send + Sync + 'static`).
/// Task 7 added `table`; Task 8 adds `sink`（`mpsc::Sender` 自身 `Clone`，
/// 不需 `Arc` 包裹）。
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<GatewayConfig>,
    pub keys: Arc<HashMap<String, KeyConfig>>,
    pub api_keys: Arc<HashMap<String, String>>,
    pub client: reqwest::Client,
    pub quota: Arc<Quota>,
    pub table: Arc<RouteTable>,
    pub sink: UsageSink,
}

/// Resolve api keys + build the upstream client + route table. Called once
/// at startup.
pub fn build_state(cfg: GatewayConfig) -> Result<AppState> {
    let api_keys = cfg.resolve_api_keys()?;
    // resolve_keys 会把 `key_env` 解析成真实密钥，并作为 map key 与
    // `KeyIdentity.config.key`（quota 记账用）。
    let keys = cfg.resolve_keys()?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(cfg.connect_timeout_secs))
        // per-read timeout: resets on activity, so long SSE streams that keep
        // emitting are not cut. This is the "read timeout" the spec calls out
        // (§4.3, 放宽到分钟级) — distinct from a hard total `.timeout()`.
        .read_timeout(Duration::from_secs(cfg.read_timeout_secs))
        .build()
        .map_err(|e| GatewayError::Upstream(format!("构建 reqwest client 失败: {e}")))?;
    let table = RouteTable::from_config(&cfg);
    // usage jsonl sink：spawn_writer 起后台 task + 建父目录。失败 → Config
    // 错误拒绝启动（spec：usage_file 父目录由 spawn_writer 创建）。
    let sink = UsageSink::spawn_writer(&cfg.usage_file).map_err(|e| {
        GatewayError::Config(format!(
            "usage sink (path {}) spawn failed: {e}",
            cfg.usage_file
        ))
    })?;
    Ok(AppState {
        cfg: Arc::new(cfg),
        keys: Arc::new(keys),
        api_keys: Arc::new(api_keys),
        client,
        quota: Arc::new(Quota::new()),
        table: Arc::new(table),
        sink,
    })
}

/// Mount routes. `GET /healthz` + `proxy::handle` fallback, both behind the
/// `require_key` auth layer. The layer sits above the fallback so `/healthz`
/// also passes through `require_key`, which exempts it by path. Everything
/// else flows through `proxy::handle`.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .fallback(proxy::handle)
        .layer(from_fn_with_state(state.clone(), require_key))
        // 最外层：nginx 风格 access log，覆盖 /healthz 与全部透传请求。
        .layer(axum::middleware::from_fn(crate::access_log::access_log))
        .with_state(state)
}

/// Liveness probe — no auth, no state, returns literal `"ok\n"`.
async fn healthz() -> &'static str {
    "ok\n"
}

/// Bind `cfg.listen`, serve with graceful shutdown (ctrl_c + unix SIGTERM).
pub async fn run(cfg: GatewayConfig) -> Result<()> {
    let listen = cfg.listen.clone();
    let state = build_state(cfg)?;
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, "sebas gateway listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

/// 以外部提供的 listener 启动 gateway（不接管 ctrl_c/SIGTERM），返回实际监听
/// 地址与 serve task。供嵌入方（`sebas run --gateway`）在随机端口（`127.0.0.1:0`）
/// 上启动并向日志输出真实端口；进程生命周期由调用方/runtime 管理。
pub fn serve_with_listener(
    cfg: GatewayConfig,
    listener: tokio::net::TcpListener,
) -> Result<(
    std::net::SocketAddr,
    tokio::task::JoinHandle<std::io::Result<()>>,
)> {
    let state = build_state(cfg)?;
    let app = build_router(state);
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
    });
    Ok((addr, handle))
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
