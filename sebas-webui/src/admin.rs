//! Admin dashboard routes, Adapter trait, and security middleware.
//!
//! Provides the control-plane admin interface for the WebUI:
//! - `/admin/status`  — control plane status (operations, version, events)
//! - `/admin/events`  — event timeline
//! - `/admin/update`  — update controls (release, dev, dry-run, rollback)
//! - `/admin/services` — managed-services overview
//! - `/admin/restart` — restart core
//! - `/admin/login`   — login page (if password is set)
//! - `/admin/logout`  — logout
//!
//! All mutation endpoints are POST-only and protected by CSRF + origin check.
//! When `SEBAS_WEBUI_PASSWORD` is set, all admin routes (except login) require
//! a valid session cookie.
//!
//! The [`AdminAdapter`] trait allows the sebas binary crate to provide either
//! a real control-RPC implementation or a no-op stub.

use crate::admin_auth::SessionStore;
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

// ─── AdminAdapter Trait ────────────────────────────────────────────────────

/// Trait for communicating with the watchdog control plane.
///
/// The sebas binary crate implements this trait using the control RPC socket
/// (or a fake for tests). When the adapter is absent, admin routes show a
/// "not connected" message.
#[async_trait]
pub trait AdminAdapter: Send + Sync {
    /// Get control-plane status: operations list, active operation, version.
    async fn status(&self) -> Result<AdminStatus, String>;

    /// Get events since the given sequence number.
    async fn events_since(&self, seq: u64) -> Result<Vec<AdminEvent>, String>;

    /// Submit an update (release) operation.
    async fn update(&self, dev: bool, dry_run: bool) -> Result<AdminMutationResult, String>;

    /// Submit a rollback operation.
    async fn rollback(&self, dry_run: bool) -> Result<AdminMutationResult, String>;

    /// Restart the core service.
    async fn restart_core(&self) -> Result<AdminMutationResult, String>;

    /// Set a managed service's desired state (`desired` ∈ {"on", "off"}).
    /// 选择会被 watchdog 持久化（services.json），重启后保持。
    async fn service_set(&self, service: &str, desired: &str)
        -> Result<AdminMutationResult, String>;

    /// Get the list of managed services and their status.
    async fn services(&self) -> Result<Vec<AdminService>, String>;
}

// ─── Data Models ───────────────────────────────────────────────────────────

/// Status information returned by the adapter.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AdminStatus {
    pub version: String,
    pub uptime_secs: u64,
    pub operations: Vec<AdminOperation>,
    pub active_operation: Option<AdminOperation>,
}

/// A single control-plane operation.
#[derive(Debug, Clone, Serialize)]
pub struct AdminOperation {
    pub operation_id: String,
    pub request_type: String,
    pub status: String,
    pub message: String,
}

/// A single control-plane event.
#[derive(Debug, Clone, Serialize)]
pub struct AdminEvent {
    pub seq: u64,
    pub operation_id: String,
    pub kind: String,
    pub message: String,
}

/// Result of a mutation action.
#[derive(Debug, Clone, Serialize)]
pub struct AdminMutationResult {
    pub operation_id: String,
    pub status: String,
    pub message: String,
}

/// A managed service entry.
#[derive(Debug, Clone, Serialize)]
pub struct AdminService {
    pub name: String,
    pub status: String,
    pub desired: String,
    pub uptime_secs: Option<u64>,
}

// ─── Admin State ───────────────────────────────────────────────────────────

/// Shared state for admin routes.
#[derive(Clone)]
pub struct AdminState {
    pub adapter: Option<Arc<dyn AdminAdapter>>,
    pub started_at: Arc<Instant>,
    pub password: Option<Arc<str>>,
    pub session_store: SessionStore,
}

impl AdminState {
    /// Create a new admin state with an optional adapter.
    pub fn new(adapter: Option<Arc<dyn AdminAdapter>>) -> Self {
        let password = std::env::var("SEBAS_WEBUI_PASSWORD")
            .ok()
            .filter(|p| !p.is_empty())
            .map(|p| Arc::from(p.as_str()));
        Self {
            adapter,
            started_at: Arc::new(Instant::now()),
            password,
            session_store: SessionStore::new(),
        }
    }

    /// Whether a password is configured (auth required for mutations).
    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }
}

// ─── Route Handlers ────────────────────────────────────────────────────────





// ─── Mutation Endpoints (POST-only) ────────────────────────────────────────

/// POST /admin/update — run a release update.
pub async fn admin_update_action(State(state): State<AdminState>) -> impl IntoResponse {
    match &state.adapter {
        Some(adapter) => match adapter.update(false, false).await {
            Ok(result) => mutation_json(&result),
            Err(e) => mutation_error(e),
        },
        None => no_adapter_error(),
    }
}

/// POST /admin/update/dry-run — dry-run release update.
pub async fn admin_dry_run_action(State(state): State<AdminState>) -> impl IntoResponse {
    match &state.adapter {
        Some(adapter) => match adapter.update(false, true).await {
            Ok(result) => mutation_json(&result),
            Err(e) => mutation_error(e),
        },
        None => no_adapter_error(),
    }
}

/// POST /admin/update/dev — dev update (build from source).
pub async fn admin_dev_update_action(State(state): State<AdminState>) -> impl IntoResponse {
    match &state.adapter {
        Some(adapter) => match adapter.update(true, false).await {
            Ok(result) => mutation_json(&result),
            Err(e) => mutation_error(e),
        },
        None => no_adapter_error(),
    }
}

/// POST /admin/rollback — rollback to previous version.
pub async fn admin_rollback_action(State(state): State<AdminState>) -> impl IntoResponse {
    match &state.adapter {
        Some(adapter) => match adapter.rollback(false).await {
            Ok(result) => mutation_json(&result),
            Err(e) => mutation_error(e),
        },
        None => no_adapter_error(),
    }
}

/// POST /admin/restart — restart core service.
pub async fn admin_restart_action(State(state): State<AdminState>) -> impl IntoResponse {
    match &state.adapter {
        Some(adapter) => match adapter.restart_core().await {
            Ok(result) => mutation_json(&result),
            Err(e) => mutation_error(e),
        },
        None => no_adapter_error(),
    }
}

/// POST /admin/services/{service}/enable — set desired state to "on"
/// （选择持久化到 services.json，watchdog 重启后保持）。
pub async fn admin_service_enable(
    State(state): State<AdminState>,
    Path(service): Path<String>,
) -> impl IntoResponse {
    service_set_action(state, &service, "on").await
}

/// POST /admin/services/{service}/disable — set desired state to "off".
pub async fn admin_service_disable(
    State(state): State<AdminState>,
    Path(service): Path<String>,
) -> impl IntoResponse {
    service_set_action(state, &service, "off").await
}

async fn service_set_action(
    state: AdminState,
    service: &str,
    desired: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    match &state.adapter {
        Some(adapter) => match adapter.service_set(service, desired).await {
            Ok(result) => mutation_json(&result),
            Err(e) => mutation_error(e),
        },
        None => no_adapter_error(),
    }
}

fn mutation_json(result: &AdminMutationResult) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "accepted",
            "operation_id": result.operation_id,
            "message": result.message,
        })),
    )
}

fn mutation_error(e: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e })),
    )
}

fn no_adapter_error() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({ "error": "control plane not connected" })),
    )
}

// ─── Auth Middleware ──────────────────────────────────────────────────────────

/// Cookie name for the admin session.
const SESSION_COOKIE_NAME: &str = "sebas_admin_session";



/// Extension to pass CSRF token through request layers.
#[derive(Clone)]
struct CsrfExtension(String);

/// Security middleware for mutation routes.
///
/// Checks:
/// 1. POST-only (returns 405 for GET/HEAD/etc.)
/// 2. If password is set: valid CSRF token in `X-CSRF-Token` header, OR
///    valid loopback origin (for CLI tools like curl).
/// 3. If password is not set: valid loopback origin check only.
pub async fn admin_mutation_guard(
    State(state): State<AdminState>,
    req: Request<Body>,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    // POST-only check
    if req.method() != Method::POST {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }

    // Origin check (always enforced)
    let origin_is_valid =
        if let Some(origin) = req.headers().get("origin").and_then(|v| v.to_str().ok()) {
            origin.is_empty() || is_loopback_origin(origin)
        } else {
            // No origin header — may be a CLI tool. Still require CSRF if password is set.
            false
        };

    if state.has_password() {
        // Password mode: require CSRF token OR valid loopback origin
        let csrf_token = req
            .headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let session_csrf = req.extensions().get::<CsrfExtension>().map(|c| c.0.clone());

        let csrf_valid = match (csrf_token, session_csrf) {
            (Some(token), Some(expected)) => token == expected,
            _ => false,
        };

        if !csrf_valid && !origin_is_valid {
            return Err(StatusCode::FORBIDDEN);
        }
    } else if let Some(origin) = req.headers().get("origin").and_then(|v| v.to_str().ok()) {
        // No password mode: origin check is the only protection
        if !origin.is_empty() && !is_loopback_origin(origin) {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    Ok(next.run(req).await)
}

/// Check if the origin is a localhost origin.
fn is_loopback_origin(origin: &str) -> bool {
    // Accept http://127.0.0.1[:port] and http://localhost[:port]
    if let Some(rest) = origin.strip_prefix("http://") {
        let host = rest.split(':').next().unwrap_or(rest);
        host == "127.0.0.1" || host == "localhost" || host == "::1"
    } else {
        false
    }
}

// ─── Login / Logout ─────────────────────────────────────────────────────────


/// POST /admin/login — authenticate and create session.
#[derive(serde::Deserialize)]
pub struct LoginForm {
    password: String,
}



// ─── JSON API for the SPA (see the `webui-api` capability) ──────────────────

/// Extract the admin session cookie value from request headers.
fn extract_session_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix(&format!("{}=", SESSION_COOKIE_NAME))
                    .map(|val| val.to_string())
            })
        })
}

fn api_401() -> axum::response::Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "authentication required" })),
    )
        .into_response()
}

/// GET /api/admin/status — control plane status as JSON.
pub async fn api_admin_status(State(state): State<AdminState>) -> impl IntoResponse {
    let (status, adapter_ok) = match &state.adapter {
        Some(adapter) => match adapter.status().await {
            Ok(s) => (s, true),
            Err(e) => (
                AdminStatus {
                    version: format!("error: {e}"),
                    ..Default::default()
                },
                false,
            ),
        },
        None => (AdminStatus::default(), false),
    };

    let uptime = state.started_at.as_ref().elapsed();
    Json(serde_json::json!({
        "adapter_ok": adapter_ok,
        "status": status,
        "uptime_secs": uptime.as_secs(),
        "uptime_display": format_uptime(uptime),
    }))
}

/// GET /api/admin/events — control-plane event timeline as JSON.
pub async fn api_admin_events(State(state): State<AdminState>) -> impl IntoResponse {
    let (events, adapter_ok) = match &state.adapter {
        Some(adapter) => match adapter.events_since(0).await {
            Ok(events) => (events, true),
            Err(e) => {
                let ev = AdminEvent {
                    seq: 0,
                    operation_id: "error".into(),
                    kind: "error".into(),
                    message: e,
                };
                (vec![ev], false)
            }
        },
        None => (vec![], false),
    };

    Json(serde_json::json!({
        "adapter_ok": adapter_ok,
        "events": events,
    }))
}

/// GET /api/admin/services — managed services as JSON.
pub async fn api_admin_services(State(state): State<AdminState>) -> impl IntoResponse {
    let (services, adapter_ok) = match &state.adapter {
        Some(adapter) => match adapter.services().await {
            Ok(svcs) => (svcs, true),
            Err(e) => {
                let svc = AdminService {
                    name: "error".into(),
                    status: e,
                    desired: "".into(),
                    uptime_secs: None,
                };
                (vec![svc], false)
            }
        },
        None => (vec![], false),
    };

    Json(serde_json::json!({
        "adapter_ok": adapter_ok,
        "services": services,
    }))
}

/// Auth guard for `/api/admin/*`: like [`admin_auth_guard`], but rejects
/// with a JSON `401` instead of redirecting, so any client can branch on
/// the status code.
pub async fn api_admin_auth_guard(
    State(state): State<AdminState>,
    req: Request<Body>,
    next: Next,
) -> axum::response::Response {
    if !state.has_password() {
        // No password configured — allow all requests (read-only mode).
        return next.run(req).await;
    }

    let path = req.uri().path();
    if path == "/api/admin/login" || path == "/api/admin/logout" {
        return next.run(req).await;
    }

    match extract_session_cookie(req.headers()) {
        Some(id) => match state.session_store.validate(&id).await {
            Ok(csrf_token) => {
                let mut req = req;
                req.extensions_mut().insert(CsrfExtension(csrf_token));
                next.run(req).await
            }
            Err(_) => api_401(),
        },
        None => api_401(),
    }
}

/// POST /api/admin/login — authenticate with JSON `{ "password": ... }`.
/// On success sets the admin session cookie (Path=/ so it covers
/// `/api/admin/*`), HttpOnly, SameSite=Lax; the 24 h inactivity TTL lives
/// in the session store.
pub async fn api_login_action(
    State(state): State<AdminState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(form): Json<LoginForm>,
) -> axum::response::Response {
    // Per-IP rate limit：同一来源 IP 的登录尝试独立计数，避免单 IP 暴力
    // 破解影响其他用户（admin_auth 已支持 per-IP，这里真正接线）。
    let client_ip = addr.ip().to_string();
    if !state.session_store.check_rate_limit(&client_ip).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": "Too many login attempts. Try again later." })),
        )
            .into_response();
    }

    let password_ok = match &state.password {
        Some(expected) => form.password == expected.as_ref(),
        None => false,
    };
    if !password_ok {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Invalid password." })),
        )
            .into_response();
    }

    state.session_store.reset_rate_limit(&client_ip).await;
    let (session_id, _csrf) = state.session_store.create().await;
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax",
        SESSION_COOKIE_NAME, session_id
    );
    let mut resp = (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok" })),
    )
        .into_response();
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

/// POST /api/admin/logout — end the session and clear the cookie.
pub async fn api_logout_action(
    State(state): State<AdminState>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    if let Some(session_id) = extract_session_cookie(&headers) {
        state.session_store.remove(&session_id).await;
    }
    let cookie = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        SESSION_COOKIE_NAME
    );
    let mut resp = (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok" })),
    )
        .into_response();
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

/// Build the JSON admin API router (mounted always, unlike the HTML admin
/// cluster): reads report `adapter_ok: false` without an adapter and
/// mutations return 503, so the SPA can present an honest degradation
/// instead of a dead link.
pub fn build_api_admin_router(state: AdminState) -> Router {
    let mutation_routes = Router::new()
        .route("/api/admin/update", post(admin_update_action))
        .route("/api/admin/update/dry-run", post(admin_dry_run_action))
        .route("/api/admin/update/dev", post(admin_dev_update_action))
        .route("/api/admin/rollback", post(admin_rollback_action))
        .route("/api/admin/restart", post(admin_restart_action))
        .route(
            "/api/admin/services/{service}/enable",
            post(admin_service_enable),
        )
        .route(
            "/api/admin/services/{service}/disable",
            post(admin_service_disable),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            admin_mutation_guard,
        ));

    let protected = Router::new()
        .route("/api/admin/status", get(api_admin_status))
        .route("/api/admin/events", get(api_admin_events))
        .route("/api/admin/services", get(api_admin_services))
        .merge(mutation_routes)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            api_admin_auth_guard,
        ));

    let public = Router::new()
        .route("/api/admin/login", post(api_login_action))
        .route("/api/admin/logout", post(api_logout_action));

    Router::new()
        .merge(protected)
        .merge(public)
        .with_state(state)
}

// ─── Router Builder ─────────────────────────────────────────────────────────


/// Health check endpoint.
pub async fn health() -> &'static str {
    "ok\n"
}

// ─── Standalone Server ─────────────────────────────────────────────────────

/// Run the standalone admin server.
pub async fn run_standalone(listener: tokio::net::TcpListener, adapter: Option<Arc<dyn AdminAdapter>>) {
    let state = AdminState::new(adapter);
    let app = build_api_admin_router(state);
    let addr = listener.local_addr().expect("bound listener");
    tracing::info!(%addr, "admin dashboard started (standalone)");
    if let Err(e) = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    {
        tracing::error!(error = %e, "admin server error");
    }
}

// ─── Template Helpers ──────────────────────────────────────────────────────



/// Format a Duration as a human-readable string.
fn format_uptime(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    // ── Fake Adapter ───────────────────────────────────────────────────────

    struct FakeAdapter {
        fail: bool,
    }

    #[async_trait]
    impl AdminAdapter for FakeAdapter {
        async fn status(&self) -> Result<AdminStatus, String> {
            if self.fail {
                return Err("fake failure".into());
            }
            Ok(AdminStatus {
                version: "0.1.0-test".into(),
                uptime_secs: 42,
                operations: vec![AdminOperation {
                    operation_id: "op_1".into(),
                    request_type: "Update".into(),
                    status: "Succeeded".into(),
                    message: "update completed".into(),
                }],
                active_operation: None,
            })
        }

        async fn events_since(&self, seq: u64) -> Result<Vec<AdminEvent>, String> {
            if self.fail {
                return Err("fake failure".into());
            }
            let all = vec![
                AdminEvent {
                    seq: 1,
                    operation_id: "op_1".into(),
                    kind: "Started".into(),
                    message: "operation started".into(),
                },
                AdminEvent {
                    seq: 2,
                    operation_id: "op_1".into(),
                    kind: "Done".into(),
                    message: "operation completed".into(),
                },
            ];
            Ok(all.into_iter().filter(|e| e.seq > seq).collect())
        }

        async fn update(&self, dev: bool, dry_run: bool) -> Result<AdminMutationResult, String> {
            if self.fail {
                return Err("fake failure".into());
            }
            Ok(AdminMutationResult {
                operation_id: "op_update".into(),
                status: "accepted".into(),
                message: format!("update dev={dev} dry_run={dry_run}"),
            })
        }

        async fn rollback(&self, dry_run: bool) -> Result<AdminMutationResult, String> {
            if self.fail {
                return Err("fake failure".into());
            }
            Ok(AdminMutationResult {
                operation_id: "op_rollback".into(),
                status: "accepted".into(),
                message: format!("rollback dry_run={dry_run}"),
            })
        }

        async fn restart_core(&self) -> Result<AdminMutationResult, String> {
            if self.fail {
                return Err("fake failure".into());
            }
            Ok(AdminMutationResult {
                operation_id: "op_restart".into(),
                status: "accepted".into(),
                message: "restart requested".into(),
            })
        }

        async fn service_set(
            &self,
            service: &str,
            desired: &str,
        ) -> Result<AdminMutationResult, String> {
            if self.fail {
                return Err("fake failure".into());
            }
            Ok(AdminMutationResult {
                operation_id: format!("op_service_{service}"),
                status: "accepted".into(),
                message: format!("{service} set to {desired}"),
            })
        }

        async fn services(&self) -> Result<Vec<AdminService>, String> {
            if self.fail {
                return Err("fake failure".into());
            }
            Ok(vec![AdminService {
                name: "webui".into(),
                status: "running".into(),
                desired: "enabled".into(),
                uptime_secs: Some(3600),
            }])
        }
    }

    fn test_state(adapter: Option<Arc<dyn AdminAdapter>>) -> AdminState {
        AdminState::new(adapter)
    }











    // ── Adapter contract tests ────────────────────────────────────────────

    #[tokio::test]
    async fn adapter_status_returns_expected_operations() {
        let adapter = FakeAdapter { fail: false };
        let status = adapter.status().await.expect("status must succeed");
        assert_eq!(status.version, "0.1.0-test");
        assert_eq!(status.operations.len(), 1);
        assert_eq!(status.operations[0].operation_id, "op_1");
        assert_eq!(status.operations[0].status, "Succeeded");
    }

    #[tokio::test]
    async fn adapter_events_since_returns_filtered_events() {
        let adapter = FakeAdapter { fail: false };
        let events = adapter.events_since(1).await.expect("events must succeed");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, 2);
    }

    #[tokio::test]
    async fn adapter_update_produces_normalized_request() {
        let adapter = FakeAdapter { fail: false };
        // Test release update (dev=false, dry_run=false)
        let result = adapter
            .update(false, false)
            .await
            .expect("update must succeed");
        assert_eq!(result.operation_id, "op_update");
        assert!(result.message.contains("dev=false"));
        assert!(result.message.contains("dry_run=false"));

        // Test dev update (dev=true, dry_run=false)
        let result = adapter
            .update(true, false)
            .await
            .expect("dev update must succeed");
        assert!(result.message.contains("dev=true"));

        // Test dry-run (dev=false, dry_run=true)
        let result = adapter
            .update(false, true)
            .await
            .expect("dry-run must succeed");
        assert!(result.message.contains("dry_run=true"));
    }

    #[tokio::test]
    async fn adapter_rollback_produces_normalized_request() {
        let adapter = FakeAdapter { fail: false };
        let result = adapter
            .rollback(false)
            .await
            .expect("rollback must succeed");
        assert_eq!(result.operation_id, "op_rollback");
        assert!(result.message.contains("dry_run=false"));
    }

    #[tokio::test]
    async fn adapter_restart_core_produces_normalized_request() {
        let adapter = FakeAdapter { fail: false };
        let result = adapter.restart_core().await.expect("restart must succeed");
        assert_eq!(result.operation_id, "op_restart");
        assert_eq!(result.message, "restart requested");
    }

    #[tokio::test]
    async fn adapter_service_set_produces_normalized_request() {
        let adapter = FakeAdapter { fail: false };
        let result = adapter
            .service_set("core", "on")
            .await
            .expect("service_set must succeed");
        assert_eq!(result.operation_id, "op_service_core");
        assert_eq!(result.message, "core set to on");
    }

    #[tokio::test]
    async fn service_enable_route_accepts_post() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_api_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/services/core/enable")
                    .header("origin", "http://127.0.0.1:9797")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn service_disable_route_accepts_post() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_api_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/services/router/disable")
                    .header("origin", "http://127.0.0.1:9797")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── No-adapter tests ───────────────────────────────────────────────────




    #[test]
    fn is_loopback_origin_accepts_localhost() {
        assert!(is_loopback_origin("http://localhost:9797"));
        assert!(is_loopback_origin("http://127.0.0.1:9797"));
        assert!(is_loopback_origin("http://localhost"));
        assert!(is_loopback_origin("http://127.0.0.1"));
        assert!(!is_loopback_origin("http://evil.com"));
        assert!(!is_loopback_origin("https://localhost:9797"));
        assert!(!is_loopback_origin(""));
    }

    // ── JSON API tests (`/api/admin/*`) ───────────────────────────────────

    use serde_json::Value;

    async fn api_json(app: Router, method: &str, uri: &str, body: Option<&str>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .extension(ConnectInfo(SocketAddr::new(std::net::IpAddr::from([127, 0, 0, 1]), 12345)));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let req = match body {
            Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("non-JSON from {uri} [{status}]: {e}"));
        (status, v)
    }

    #[tokio::test]
    async fn api_admin_status_reports_adapter_presence() {
        // With adapter: adapter_ok true, data present.
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_api_admin_router(test_state(adapter));
        let (status, v) = api_json(app, "GET", "/api/admin/status", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["adapter_ok"], true);
        assert_eq!(v["status"]["version"], "0.1.0-test");

        // Without adapter: honest degradation, adapter_ok false.
        let app = build_api_admin_router(test_state(None));
        let (status, v) = api_json(app, "GET", "/api/admin/status", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["adapter_ok"], false);
    }

    #[tokio::test]
    async fn api_admin_events_and_services_report_adapter_presence() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_api_admin_router(test_state(adapter));
        let (status, v) = api_json(app, "GET", "/api/admin/events", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["adapter_ok"], true);
        assert_eq!(v["events"].as_array().unwrap().len(), 2);

        let app = build_api_admin_router(test_state(None));
        let (status, v) = api_json(app, "GET", "/api/admin/events", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["adapter_ok"], false);
        assert_eq!(v["events"].as_array().unwrap().len(), 0);

        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_api_admin_router(test_state(adapter));
        let (status, v) = api_json(app, "GET", "/api/admin/services", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["services"][0]["name"], "webui");
    }

    #[tokio::test]
    async fn api_admin_mutation_proxies_and_reports_missing_adapter() {
        // With adapter: mutation accepted (POST with loopback origin).
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_api_admin_router(test_state(adapter));
        let (status, v) = api_json(
            app,
            "POST",
            "/api/admin/restart",
            Some("{}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {v}");
        assert_eq!(v["operation_id"], "op_restart");

        // Without adapter: 503 with the error envelope.
        let app = build_api_admin_router(test_state(None));
        let (status, v) = api_json(app, "POST", "/api/admin/update", Some("{}")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(v["error"].as_str().unwrap().contains("control plane not connected"));
    }

    #[tokio::test]
    async fn api_admin_mutations_reject_get_with_405() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_api_admin_router(test_state(adapter));
        // /api/admin/update is registered POST-only; GET is 405.
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/admin/update")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn api_admin_foreign_origin_rejected() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_api_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/admin/restart")
                    .header("origin", "https://evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// State with a password configured, bypassing env-var races.
    fn password_state() -> AdminState {
        let mut st = test_state(None);
        st.password = Some(Arc::from("sekrit"));
        st
    }

    #[tokio::test]
    async fn api_admin_login_logout_round_trip() {
        let state = password_state();
        let app = build_api_admin_router(state.clone());

        // Unauthenticated read → 401 with JSON error.
        let (status, v) = api_json(app.clone(), "GET", "/api/admin/status", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(v["error"].as_str().is_some());

        // Wrong password → 403.
        let (status, _) = api_json(
            app.clone(),
            "POST",
            "/api/admin/login",
            Some(r#"{"password": "wrong"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Correct password → 200 + cookie (HttpOnly, Path=/).
        let req = Request::builder()
            .method("POST")
            .uri("/api/admin/login")
            .extension(ConnectInfo(SocketAddr::new(std::net::IpAddr::from([127, 0, 0, 1]), 12345)))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"password": "sekrit"}"#.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cookie = resp
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .to_string();
        assert!(cookie.starts_with("sebas_admin_session="), "cookie: {cookie}");
        assert!(cookie.contains("HttpOnly"), "cookie: {cookie}");
        assert!(cookie.contains("Path=/"), "cookie must cover /api/admin: {cookie}");
        let session_value = cookie
            .split(';')
            .next()
            .unwrap()
            .trim()
            .to_string();

        // Authenticated read succeeds.
        let (status, _) = api_json_with_cookie(app.clone(), "GET", "/api/admin/status", &session_value).await;
        assert_eq!(status, StatusCode::OK);

        // Logout clears the session.
        let (status, _) = api_json_with_cookie(app.clone(), "POST", "/api/admin/logout", &session_value).await;
        assert_eq!(status, StatusCode::OK);

        // After logout the same cookie is invalid again.
        let (status, v) = api_json_with_cookie(app, "GET", "/api/admin/status", &session_value).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(v["error"].as_str().is_some());
    }

    async fn api_json_with_cookie(
        app: Router,
        method: &str,
        uri: &str,
        cookie: &str,
    ) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .extension(ConnectInfo(SocketAddr::new(std::net::IpAddr::from([127, 0, 0, 1]), 12345)))
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("non-JSON from {uri} [{status}]: {e}"));
        (status, v)
    }

    /// POST 一次 JSON 登录尝试，模拟指定来源 IP。返回状态码 + 是否被限速。
    async fn api_login_attempt(
        app: &axum::Router,
        ip: std::net::IpAddr,
    ) -> (StatusCode, bool) {
        let req = Request::builder()
            .method("POST")
            .uri("/api/admin/login")
            .extension(ConnectInfo(SocketAddr::new(ip, 12345)))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"password": "nope"}"#.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let limited = status == StatusCode::TOO_MANY_REQUESTS;
        (status, limited)
    }

    #[tokio::test]
    async fn api_admin_login_rate_limit_still_enforced() {
        let state = password_state();
        let app = build_api_admin_router(state);
        let ip: std::net::IpAddr = "10.0.0.9".parse().unwrap();
        for _ in 0..5 {
            let (status, _limited) = api_login_attempt(&app, ip).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
        // The 6th attempt inside the window is rejected by the limiter.
        let (status, limited) = api_login_attempt(&app, ip).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(limited);
    }

    #[tokio::test]
    async fn api_admin_login_rate_limit_is_per_ip() {
        let state = password_state();
        let app = build_api_admin_router(state);
        let ip_a: std::net::IpAddr = "10.0.1.1".parse().unwrap();
        let ip_b: std::net::IpAddr = "10.0.1.2".parse().unwrap();

        // IP A 连续失败 → 触发限速。
        let mut blocked_a = false;
        for _ in 0..8 {
            let (_status, limited) = api_login_attempt(&app, ip_a).await;
            if limited {
                blocked_a = true;
                break;
            }
        }
        assert!(blocked_a, "IP A 连续失败后应被限速");

        // 不同 IP 不受影响：第一次尝试是普通 403，而非 429。
        let (status, limited_b) = api_login_attempt(&app, ip_b).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!limited_b, "IP B 不应被 IP A 的失败影响（per-IP 限速）");
    }
}
