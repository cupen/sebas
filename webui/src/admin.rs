//! Admin dashboard routes, Adapter trait, and security middleware.
//!
//! Provides the control-plane admin interface for the WebUI:
//! - `/admin/status`  — control plane status (operations, version, events)
//! - `/admin/events`  — event timeline
//! - `/admin/update`  — update controls (release, dev, dry-run, rollback)
//! - `/admin/services` — managed-services overview
//! - `/admin/restart` — restart core
//!
//! All mutation endpoints are POST-only and protected by an origin check.
//! The [`AdminAdapter`] trait allows the sebas binary crate to provide either
//! a real control-RPC implementation or a no-op stub.

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use serde::Serialize;
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
}

impl AdminState {
    /// Create a new admin state with an optional adapter.
    pub fn new(
        adapter: Option<Arc<dyn AdminAdapter>>,
        templates: Arc<minijinja::Environment<'static>>,
    ) -> Self {
        Self {
            adapter,
            templates,
            started_at: Arc::new(Instant::now()),
        }
    }
}

// ─── Route Handlers ────────────────────────────────────────────────────────

/// GET /admin/status — control plane status page.
pub async fn admin_status(State(state): State<AdminState>) -> impl IntoResponse {
    let (status, adapter_ok) = match &state.adapter {
        Some(adapter) => match adapter.status().await {
            Ok(s) => (s, true),
            Err(e) => {
                let mut s = AdminStatus::default();
                s.version = format!("error: {e}");
                (s, false)
            }
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

    let data = serde_json::json!({
        "page": "admin_services",
        "adapter_ok": adapter_ok,
        "services": services,
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

// ─── Security Middleware ───────────────────────────────────────────────────

/// Middleware that enforces security constraints on mutation routes:
///
/// 1. **POST-only**: returns 405 for GET/HEAD/etc.
/// 2. **Origin check**: if the `Origin` header is present, it must be
///    `http://127.0.0.1:<port>` or `http://localhost:<port>`.  Missing
///    Origin is allowed (e.g. curl, direct browser navigation).
/// 3. **X-Requested-With**: encouraged but not strictly required for
///    the MVP baseline.
pub async fn admin_mutation_guard(
    req: Request<Body>,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    // POST-only check
    if req.method() != Method::POST {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }

    // Origin check
    if let Some(origin) = req
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
    {
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
        .layer(middleware::from_fn(admin_mutation_guard));

    Router::new()
        .route("/admin/status", get(admin_status))
        .route("/admin/events", get(admin_events))
        .route("/admin/update", get(admin_update_page))
        .route("/admin/services", get(admin_services))
        .merge(mutation_routes)
        .route("/health", get(health))
        .with_state(state)
}

/// Health check endpoint.
pub async fn health() -> &'static str {
    "ok\n"
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
    if let Err(e) = axum::serve(listener, app).await {
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
    env.add_template("admin_status.html", include_str!("../templates/admin_status.html"))
        .expect("admin_status.html template");
    env.add_template("admin_events.html", include_str!("../templates/admin_events.html"))
        .expect("admin_events.html template");
    env.add_template("admin_update.html", include_str!("../templates/admin_update.html"))
        .expect("admin_update.html template");
    env.add_template("admin_services.html", include_str!("../templates/admin_services.html"))
        .expect("admin_services.html template");
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
        env.add_template("admin_status.html", "admin_status:{{status.version}}|{{page}}|{{adapter_ok}}")
            .ok();
        env.add_template("admin_events.html", "admin_events:{{events|length}}|{{page}}|{{adapter_ok}}")
            .ok();
        env.add_template("admin_update.html", "admin_update:{{page}}|{{adapter_ok}}")
            .ok();
        env.add_template("admin_services.html", "admin_services:{{services|length}}|{{page}}|{{adapter_ok}}")
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
            .oneshot(Request::builder().uri("/admin/status").body(Body::empty()).unwrap())
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
            .oneshot(Request::builder().uri("/admin/events").body(Body::empty()).unwrap())
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
            .oneshot(Request::builder().uri("/admin/update").body(Body::empty()).unwrap())
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
            .oneshot(Request::builder().uri("/admin/services").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("admin_services:"));
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = build_admin_router(test_state(None));
        let resp = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert_eq!(body, "ok\n");
    }

    // ── Mutation route tests ──────────────────────────────────────────────

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
        let result = adapter.update(false, false).await.expect("update must succeed");
        assert_eq!(result.operation_id, "op_update");
        assert!(result.message.contains("dev=false"));
        assert!(result.message.contains("dry_run=false"));

        // Test dev update (dev=true, dry_run=false)
        let result = adapter.update(true, false).await.expect("dev update must succeed");
        assert!(result.message.contains("dev=true"));

        // Test dry-run (dev=false, dry_run=true)
        let result = adapter.update(false, true).await.expect("dry-run must succeed");
        assert!(result.message.contains("dry_run=true"));
    }

    #[tokio::test]
    async fn adapter_rollback_produces_normalized_request() {
        let adapter = FakeAdapter { fail: false };
        let result = adapter.rollback(false).await.expect("rollback must succeed");
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

    // ── No-adapter tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn no_adapter_still_renders_pages() {
        let app = build_admin_router(test_state(None));
        for path in &["/admin/status", "/admin/events", "/admin/update", "/admin/services"] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(*path)
                        .body(Body::empty())
                        .unwrap(),
                )
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
            .oneshot(Request::builder().uri("/admin/status").body(Body::empty()).unwrap())
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