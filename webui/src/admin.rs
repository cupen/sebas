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
use axum::response::{Html, IntoResponse, Json, Redirect};
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
    pub templates: Arc<minijinja::Environment<'static>>,
    pub started_at: Arc<Instant>,
    pub password: Option<Arc<str>>,
    pub session_store: SessionStore,
}

impl AdminState {
    /// Create a new admin state with an optional adapter.
    pub fn new(
        adapter: Option<Arc<dyn AdminAdapter>>,
        templates: Arc<minijinja::Environment<'static>>,
    ) -> Self {
        let password = std::env::var("SEBAS_WEBUI_PASSWORD")
            .ok()
            .filter(|p| !p.is_empty())
            .map(|p| Arc::from(p.as_str()));
        Self {
            adapter,
            templates,
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

/// GET /admin/status — control plane status page.
pub async fn admin_status(State(state): State<AdminState>) -> impl IntoResponse {
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
    let data = serde_json::json!({
        "page": "admin_status",
        "adapter_ok": adapter_ok,
        "status": status,
        "uptime_secs": uptime.as_secs(),
        "uptime_display": format_uptime(uptime),
    });
    render_template(&state, "admin_status.html", &data).await
}

/// GET /admin/events — event timeline page.
pub async fn admin_events(State(state): State<AdminState>) -> impl IntoResponse {
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

    let data = serde_json::json!({
        "page": "admin_events",
        "adapter_ok": adapter_ok,
        "events": events,
    });
    render_template(&state, "admin_events.html", &data).await
}

/// GET /admin/update — update control page.
pub async fn admin_update_page(State(state): State<AdminState>) -> impl IntoResponse {
    let last_result: Option<AdminMutationResult> = None;
    let data = serde_json::json!({
        "page": "admin_update",
        "adapter_ok": state.adapter.is_some(),
        "last_result": last_result,
    });
    render_template(&state, "admin_update.html", &data).await
}

/// GET /admin/services — managed services page.
pub async fn admin_services(State(state): State<AdminState>) -> impl IntoResponse {
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

    // watchdog/updater 是伪行（非受管服务），不渲染启停按钮。
    let rows: Vec<serde_json::Value> = services
        .iter()
        .map(|svc| {
            serde_json::json!({
                "name": svc.name,
                "status": svc.status,
                "desired": svc.desired,
                "uptime_secs": svc.uptime_secs,
                "managed": matches!(svc.name.as_str(), "core" | "webui" | "gateway"),
            })
        })
        .collect();

    let data = serde_json::json!({
        "page": "admin_services",
        "adapter_ok": adapter_ok,
        "services": rows,
    });
    render_template(&state, "admin_services.html", &data).await
}

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

/// Middleware that checks authentication for all admin routes (except login).
///
/// If `SEBAS_WEBUI_PASSWORD` is set, all `/admin/*` routes (except `/admin/login`
/// and `/admin/logout`) require a valid session cookie.  If the password is not
/// set, this middleware is a no-op.
pub async fn admin_auth_guard(
    State(state): State<AdminState>,
    req: Request<Body>,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    if !state.has_password() {
        // No password configured — allow all requests (read-only mode).
        return Ok(next.run(req).await);
    }

    let path = req.uri().path().to_string();
    // Allow login/logout without auth
    if path == "/admin/login" || path == "/admin/logout" {
        return Ok(next.run(req).await);
    }

    // Check session cookie
    let session_id = req
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix(&format!("{}=", SESSION_COOKIE_NAME))
                    .map(|val| val.to_string())
            })
        });

    match session_id {
        Some(id) => match state.session_store.validate(&id).await {
            Ok(csrf_token) => {
                // Store CSRF token in request extensions for mutation middleware
                let mut req = req;
                req.extensions_mut().insert(CsrfExtension(csrf_token));
                Ok(next.run(req).await)
            }
            Err(_) => redirect_to_login(),
        },
        None => redirect_to_login(),
    }
}

/// Redirect to login page.
fn redirect_to_login() -> Result<axum::response::Response<Body>, StatusCode> {
    Ok(Redirect::to("/admin/login").into_response())
}

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

/// GET /admin/login — show login form.
pub async fn admin_login_page(State(state): State<AdminState>) -> impl IntoResponse {
    let data = serde_json::json!({
        "page": "admin_login",
        "has_password": state.has_password(),
        "error": "",
    });
    render_template(&state, "admin_login.html", &data).await
}

/// POST /admin/login — authenticate and create session.
#[derive(serde::Deserialize)]
pub struct LoginForm {
    password: String,
}

pub async fn admin_login_action(
    State(state): State<AdminState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::Form(form): axum::Form<LoginForm>,
) -> axum::response::Response<Body> {
    // Per-IP rate limit：同一来源 IP 的登录尝试独立计数，避免单 IP 暴力
    // 破解影响其他用户（admin_auth 已支持 per-IP，这里真正接线）。
    let client_ip = addr.ip().to_string();
    if !state.session_store.check_rate_limit(&client_ip).await {
        let data = serde_json::json!({
            "page": "admin_login",
            "has_password": state.has_password(),
            "error": "Too many login attempts. Try again later.",
        });
        return render_template(&state, "admin_login.html", &data)
            .await
            .into_response();
    }

    // Verify password
    let password_ok = match &state.password {
        Some(expected) => form.password == expected.as_ref(),
        None => false,
    };

    if !password_ok {
        let data = serde_json::json!({
            "page": "admin_login",
            "has_password": state.has_password(),
            "error": "Invalid password.",
        });
        return render_template(&state, "admin_login.html", &data)
            .await
            .into_response();
    }

    // Success: create session, reset per-IP rate limit
    state.session_store.reset_rate_limit(&client_ip).await;
    let (session_id, _csrf) = state.session_store.create().await;

    // Set session cookie (no expiry — session store handles TTL)
    let cookie = format!(
        "{}={}; Path=/admin; HttpOnly; SameSite=Lax",
        SESSION_COOKIE_NAME, session_id
    );

    let mut resp = Redirect::to("/admin/status").into_response();
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

/// POST /admin/logout — end session.
pub async fn admin_logout_action(
    State(state): State<AdminState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // Extract session ID from cookie
    if let Some(session_id) = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix(&format!("{}=", SESSION_COOKIE_NAME))
                    .map(|s| s.to_string())
            })
        })
    {
        state.session_store.remove(&session_id).await;
    }

    // Clear cookie
    let cookie = format!(
        "{}=; Path=/admin; HttpOnly; SameSite=Lax; Max-Age=0",
        SESSION_COOKIE_NAME
    );
    let mut resp = Redirect::to("/admin/login").into_response();
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

// ─── Router Builder ─────────────────────────────────────────────────────────

/// Build an admin-only router for standalone mode.
pub fn build_admin_router(state: AdminState) -> Router {
    // Mutation routes with security middleware
    let mutation_routes = Router::new()
        .route("/admin/update", post(admin_update_action))
        .route("/admin/update/dry-run", post(admin_dry_run_action))
        .route("/admin/update/dev", post(admin_dev_update_action))
        .route("/admin/rollback", post(admin_rollback_action))
        .route("/admin/restart", post(admin_restart_action))
        .route("/admin/services/{service}/enable", post(admin_service_enable))
        .route(
            "/admin/services/{service}/disable",
            post(admin_service_disable),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            admin_mutation_guard,
        ));

    // Protected admin routes (require auth if password is set)
    let protected = Router::new()
        .route("/admin/status", get(admin_status))
        .route("/admin/events", get(admin_events))
        .route("/admin/update", get(admin_update_page))
        .route("/admin/services", get(admin_services))
        .merge(mutation_routes)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            admin_auth_guard,
        ));

    // Public admin routes (no auth required)
    let public = Router::new()
        .route("/admin/login", get(admin_login_page))
        .route("/admin/login", post(admin_login_action))
        .route("/admin/logout", post(admin_logout_action));

    Router::new()
        .merge(protected)
        .merge(public)
        .with_state(state)
}

// ─── Standalone Server ─────────────────────────────────────────────────────

/// Run the standalone admin server.
pub async fn run_standalone(
    listener: tokio::net::TcpListener,
    adapter: Option<Arc<dyn AdminAdapter>>,
) {
    let templates = Arc::new(init_standalone_templates());
    let state = AdminState::new(adapter, templates);
    let app = build_admin_router(state);
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

/// Render a MiniJinja template with context.
async fn render_template(
    state: &AdminState,
    template_name: &str,
    context: &serde_json::Value,
) -> Html<String> {
    let tmpl = state
        .templates
        .get_template(template_name)
        .unwrap_or_else(|_| panic!("template {template_name} should exist"));
    let rendered = tmpl
        .render(minijinja::Value::from_serialize(context))
        .unwrap_or_else(|e| format!("Template error: {e}"));
    Html(rendered)
}

/// Initialize templates for standalone mode.
fn init_standalone_templates() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::new();
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
    env.add_template(
        "admin_login.html",
        include_str!("../templates/admin_login.html"),
    )
    .expect("admin_login.html template");
    env
}

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
        let mut env = minijinja::Environment::new();
        // Register minimal templates for tests
        env.add_template(
            "admin_status.html",
            "admin_status:{{status.version}}|{{page}}|{{adapter_ok}}",
        )
        .ok();
        env.add_template(
            "admin_events.html",
            "admin_events:{{events|length}}|{{page}}|{{adapter_ok}}",
        )
        .ok();
        env.add_template("admin_update.html", "admin_update:{{page}}|{{adapter_ok}}")
            .ok();
        env.add_template(
            "admin_services.html",
            "admin_services:{{services|length}}|{{page}}|{{adapter_ok}}",
        )
        .ok();
        env.add_template(
            "admin_login.html",
            "admin_login:{{page}}|{{error}}|{{has_password}}",
        )
        .ok();
        AdminState::new(adapter, Arc::new(env))
    }

    async fn body_string(body: Body) -> String {
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    // ── Route smoke tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn admin_status_page_returns_200() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("admin_status:"));
        assert!(body.contains("0.1.0-test"));
    }

    #[tokio::test]
    async fn admin_events_page_returns_200() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("admin_events:"));
    }

    #[tokio::test]
    async fn admin_update_page_returns_200() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/update")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("admin_update:"));
    }

    #[tokio::test]
    async fn admin_services_page_returns_200() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/services")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("admin_services:"));
    }

    #[tokio::test]
    async fn admin_router_does_not_register_health() {
        // 回归：/health 属于 base router（webui/src/server.rs）；admin router
        // 若也注册 /health，merge 进完整 router 时 axum 会 panic
        // 「Overlapping method route. Handler for `GET /health` already exists」
        // （watchdog 模式下 webui 子进程启动即崩，sebas-2ty 修复）。
        // 断言 build_admin_router 本身不再携带 /health：请求应 404。
        let admin_app = build_admin_router(test_state(None));
        let resp = admin_app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── Mutation route tests ──────────────────────────────────────────────

    /// POST 一次登录尝试。返回 HTTP 状态码 + 是否被限速（错误文案）。
    async fn login_attempt(
        app: &axum::Router,
        ip: std::net::IpAddr,
    ) -> (StatusCode, bool) {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .extension(ConnectInfo(SocketAddr::new(ip, 12345)))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("password=wrong"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let body = body_string(resp.into_body()).await;
        let rate_limited = body.contains("Too many login attempts");
        (status, rate_limited)
    }

    #[tokio::test]
    async fn login_rate_limit_is_per_ip() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let ip_a: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        let ip_b: std::net::IpAddr = "10.0.0.2".parse().unwrap();

        // 同一 IP 连续失败多次 → 触发限速。
        let mut blocked_a = false;
        for _ in 0..20 {
            let (_s, limited) = login_attempt(&app, ip_a).await;
            if limited {
                blocked_a = true;
                break;
            }
        }
        assert!(blocked_a, "IP A 连续失败后应被限速");

        // 不同 IP 不受影响。
        let (_s, limited_b) = login_attempt(&app, ip_b).await;
        assert!(
            !limited_b,
            "IP B 不应被 IP A 的失败影响（per-IP 限速）"
        );
    }

    #[tokio::test]
    async fn mutation_route_accepts_post() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/update")
                    .header("origin", "http://127.0.0.1:9797")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("accepted"));
        assert!(body.contains("op_update"));
    }

    #[tokio::test]
    async fn mutation_route_rejects_get() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/update")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // GET /admin/update goes to the GET handler, not the mutation
        // The mutation (POST-only) at /admin/update is reached via POST
        // GET should return the update page, not 405
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn mutation_route_without_valid_origin_gets_403() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/update")
                    .header("origin", "http://evil.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn mutation_without_origin_is_allowed() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/update")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn mutation_with_localhost_origin_is_allowed() {
        let adapter = Some(Arc::new(FakeAdapter { fail: false }) as Arc<dyn AdminAdapter>);
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/update")
                    .header("origin", "http://localhost:9797")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
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
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/admin/services/core/enable")
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
        let app = build_admin_router(test_state(adapter));
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/admin/services/gateway/disable")
                    .header("origin", "http://127.0.0.1:9797")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── No-adapter tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn no_adapter_still_renders_pages() {
        let app = build_admin_router(test_state(None));
        for path in &[
            "/admin/status",
            "/admin/events",
            "/admin/update",
            "/admin/services",
        ] {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(*path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "path {path} must return 200");
        }
    }

    #[tokio::test]
    async fn no_adapter_mutation_returns_503() {
        let app = build_admin_router(test_state(None));
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/update")
                    .header("origin", "http://127.0.0.1:9797")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("control plane not connected"));
    }

    // ── Security middleware tests ──────────────────────────────────────────

    #[tokio::test]
    async fn post_without_mutation_path_is_not_affected() {
        // Non-mutation routes should still work with GET
        let app = build_admin_router(test_state(None));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

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
}
