//! axum server setup for the WebUI: JSON API + WebSocket + embedded SPA.

use crate::admin::{self, AdminAdapter, AdminState};
use crate::agent_kinds::{AgentKindProvider, AgentKindSource, ConfigAgentKindProvider};
use crate::api;
use crate::assets;
use crate::auth::{AuthHandle, SESSION_COOKIE_NAME};
use crate::models::GatewayInfo;
use crate::routes;
use crate::session_backend::SessionBackend;
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
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
    /// 登录鉴权（用户名/密码，见 `auth` 模块）。凭据未配置时鉴权关闭，
    /// 全部路由维持原有行为。
    pub auth: Arc<AuthHandle>,
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
        Arc::new(AuthHandle::disabled()),
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
    build_router_full(
        backend,
        gateway,
        card_config,
        None,
        agent_kinds,
        30,
        Arc::new(AuthHandle::disabled()),
    )
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
        Arc::new(AuthHandle::disabled()),
    )
}

/// Build the axum Router with an explicit auth handle（登录鉴权接线入口）。
pub fn build_router_with_auth(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    agent_kinds: Arc<dyn AgentKindProvider>,
    archive_retention_days: u64,
    auth: Arc<AuthHandle>,
) -> Router {
    build_router_full(
        backend,
        gateway,
        card_config,
        admin_adapter,
        agent_kinds,
        archive_retention_days,
        auth,
    )
}

fn build_router_full(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    agent_kinds: Arc<dyn AgentKindProvider>,
    archive_retention_days: u64,
    auth: Arc<AuthHandle>,
) -> Router {
    let state = WebUiState {
        backend,
        gateway,
        started_at: Instant::now(),
        card_config,
        agent_kinds,
        archive_retention_days,
        auth,
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
        .route("/api/sessions/{key}/model", post(api::set_session_model))
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
        .route("/api/auth/me", get(api::auth_me))
        .route("/api/auth/login", post(api::auth_login))
        .route("/api/auth/logout", post(api::auth_logout))
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
        .with_state(state.clone());

    // The JSON admin API is always mounted: without an adapter, reads report
    // `adapter_ok: false` and mutations answer 503 (honest degradation).
    // It carries its own AdminState, merged as a stateless Router.
    // 登录鉴权层套在 merge 之后的全量路由上（按路径选择：/api/*、
    // /gateway/api/*、/ws；静态资源与 /health 放行），webui 登录未启用时
    // 各路由维持自身原有的防护（admin env-password、gateway origin check）。
    core.merge(gateway_mutations)
        .merge(admin::build_api_admin_router(AdminState::new(
            admin_adapter,
        )))
        .fallback(assets::spa_fallback)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_guard,
        ))
}

// ─── 登录鉴权中间件 ──────────────────────────────────────────────────────────

/// `GET /api/auth/me` 与登录端点自身不受鉴权门拦截（前端需要先探明状态）。
fn is_auth_exempt_path(path: &str) -> bool {
    path == "/api/auth/login" || path == "/api/auth/me" || path == "/api/auth/logout"
}

/// 需要登录的路径面：JSON API、gateway BFF、WebSocket。静态 SPA 资源与
/// `/health`（watchdog 探活）保持公开。
fn is_protected_path(path: &str) -> bool {
    path == "/ws" || path.starts_with("/api/") || path.starts_with("/gateway/api/")
}

/// 提取 webui 会话 cookie 值。
pub fn extract_webui_session_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix(&format!("{SESSION_COOKIE_NAME}="))
                    .map(|val| val.to_string())
            })
        })
}

/// 从 Origin 头取 authority（host[:port]）。仅用于同源比对。
fn origin_authority(origin: &str) -> Option<&str> {
    let rest = origin.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    if authority.is_empty() {
        None
    } else {
        Some(authority)
    }
}

/// 鉴权门：凭据已配置时，受保护路径需要有效会话 cookie；非安全方法（POST
/// 等）额外要求同源（Origin 头存在且与 Host 一致），与 SameSite=Lax cookie
/// 一起构成 CSRF 防线。未配置凭据时全部放行（原行为）。
async fn auth_guard(State(state): State<WebUiState>, req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path();
    if !is_protected_path(path) || is_auth_exempt_path(path) {
        return next.run(req).await;
    }
    if !state.auth.enabled() {
        return next.run(req).await;
    }

    let authorized = match extract_webui_session_cookie(req.headers()) {
        Some(session_id) => state.auth.session_store.validate(&session_id).await.is_ok(),
        None => false,
    };
    if !authorized {
        // /ws 在升级前拒绝；API 一律 JSON 401（前端据此弹登录页）。
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "sebas-session")],
            axum::Json(serde_json::json!({ "error": "authentication required" })),
        )
            .into_response();
    }

    // CSRF：浏览器发起的跨站写请求会带 Origin 头——与 Host 不一致即拒绝。
    // SameSite=Lax cookie 已挡掉绝大多数跨站携带，这里兜底非浏览器场景。
    if !is_safe_method(req.method().as_str())
        && let Some(origin) = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok())
    {
        let same_origin = origin_authority(origin)
            .zip(req.headers().get(header::HOST).and_then(|h| h.to_str().ok()))
            .map(|(origin_host, host)| origin_host.eq_ignore_ascii_case(host))
            .unwrap_or(false);
        if !same_origin {
            return (
                StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({ "error": "cross-origin request rejected" })),
            )
                .into_response();
        }
    }

    next.run(req).await
}

/// GET/HEAD/OPTIONS 不改状态，不参与同源校验。
fn is_safe_method(method: &str) -> bool {
    method == "GET" || method == "HEAD" || method == "OPTIONS"
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
    run_full(
        backend,
        gateway,
        card_config,
        agent_kinds,
        listener,
        None,
        30,
        Arc::new(AuthHandle::disabled()),
    )
    .await;
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
        Arc::new(AuthHandle::disabled()),
    )
    .await;
}

/// Run the WebUI server with an auth handle（登录鉴权接线入口）。
pub async fn run_with_admin_adapter_and_auth(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    agent_kinds: Vec<AgentKindSource>,
    listener: tokio::net::TcpListener,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    auth: Arc<AuthHandle>,
) {
    run_full(
        backend,
        gateway,
        card_config,
        agent_kinds,
        listener,
        admin_adapter,
        30,
        auth,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn run_full(
    backend: Arc<dyn SessionBackend>,
    gateway: GatewayInfo,
    card_config: CardConfig,
    agent_kinds: Vec<AgentKindSource>,
    listener: tokio::net::TcpListener,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    archive_retention_days: u64,
    auth: Arc<AuthHandle>,
) {
    let provider = Arc::new(ConfigAgentKindProvider::new(agent_kinds));
    let app = build_router_full(
        backend,
        gateway,
        card_config,
        admin_adapter,
        provider,
        archive_retention_days,
        auth,
    );
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

#[cfg(test)]
mod auth_guard_tests {
    //! 登录鉴权门的路由级测试：未启用时零影响；启用后 /api、/ws 要会话，
    //! 静态资源 / /health 放行；登录-使用-注销闭环；同源校验；限速。
    use super::*;
    use crate::auth::Credentials;
    use crate::models::GatewayInfo;
    use crate::session_backend::FakeBackend;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use http_body_util::BodyExt;
    use sebas_feishu::cards::CardConfig;
    use std::net::{IpAddr, SocketAddr};
    use tower::ServiceExt;

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 12345)
    }

    /// 鉴权开启的 router + 凭据文件句柄（tempdir 必须由调用方持有存活：
    /// AuthHandle.enabled() 每次 mtime 探测，文件被删会即时关闭鉴权）。
    fn auth_on_app() -> (Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        crate::auth::store_credentials(
            &path,
            &Credentials::with_iterations("alice", "password8", 1000),
        )
        .unwrap();
        let app = build_router_with_auth(
            Arc::new(FakeBackend::new()),
            GatewayInfo::default(),
            CardConfig::default(),
            None,
            Arc::new(ConfigAgentKindProvider::new(Vec::new())),
            30,
            Arc::new(AuthHandle::open(path)),
        );
        (app, dir)
    }

    async fn req(
        app: Router,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        origin: Option<&str>,
        body: Option<String>,
    ) -> (StatusCode, String) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()));
        if let Some(c) = cookie {
            builder = builder.header("cookie", c);
        }
        if let Some(o) = origin {
            builder = builder.header("origin", o);
        }
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let req = builder
            .body(Body::from(body.unwrap_or_default()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn auth_disabled_keeps_routes_open() {
        let app = build_router(
            Arc::new(FakeBackend::new()),
            GatewayInfo::default(),
            CardConfig::default(),
        );
        let (status, _) = req(app, "GET", "/api/summary", None, None, None).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn auth_on_blocks_api_and_ws_but_not_static_or_health() {
        let (app, _dir) = auth_on_app();

        // API 401（JSON body）。
        let (status, body) = req(app.clone(), "GET", "/api/summary", None, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("authentication required"), "{body}");

        // WS 升级前拒绝。
        let (status, _) = req(app.clone(), "GET", "/ws", None, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // 静态资源与 /health 放行。
        let (status, _) = req(app.clone(), "GET", "/health", None, None, None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = req(app.clone(), "GET", "/", None, None, None).await;
        assert_eq!(status, StatusCode::OK, "SPA 入口必须公开可取");
    }

    #[tokio::test]
    async fn login_use_logout_round_trip() {
        let (app, _dir) = auth_on_app();

        // 错误密码 → 401。
        let (status, _) = req(
            app.clone(),
            "POST",
            "/api/auth/login",
            None,
            None,
            Some(r#"{"username":"alice","password":"wrong"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // 正确密码 → 200 + cookie。
        let login_req = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header("content-type", "application/json")
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()))
            .body(Body::from(r#"{"username":"alice","password":"password8"}"#))
            .unwrap();
        let resp = app.clone().oneshot(login_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cookie = resp
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .to_string();
        assert!(cookie.starts_with("sebas_webui_session="), "{cookie}");
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"), "{cookie}");
        let session = cookie.split(';').next().unwrap().trim().to_string();

        // 带 cookie 的 API 请求放行。
        let (status, _) = req(
            app.clone(),
            "GET",
            "/api/summary",
            Some(&session),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // me 报告已认证。
        let (_, body) = req(app.clone(), "GET", "/api/auth/me", Some(&session), None, None).await;
        assert!(body.contains("\"authenticated\":true"), "{body}");

        // 注销 → 同 cookie 失效。
        let (status, _) = req(
            app.clone(),
            "POST",
            "/api/auth/logout",
            Some(&session),
            None,
            Some("{}".into()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = req(app, "GET", "/api/summary", Some(&session), None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn cross_origin_mutation_rejected() {
        let (app, _dir) = auth_on_app();
        let login_req = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header("content-type", "application/json")
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()))
            .body(Body::from(r#"{"username":"alice","password":"password8"}"#))
            .unwrap();
        let resp = app.clone().oneshot(login_req).await.unwrap();
        let cookie = resp
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim()
            .to_string();

        // 携带有效会话但 Origin 与 Host 不同源 → 403。
        let (status, _) = req(
            app.clone(),
            "POST",
            "/api/sessions",
            Some(&cookie),
            Some("http://evil.example"),
            Some("{}".into()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // 同源 Origin → 通过。
        let (status, _) = req(
            app,
            "GET",
            "/api/summary",
            Some(&cookie),
            Some("http://127.0.0.1:12345"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn login_rate_limited_per_ip() {
        let (app, _dir) = auth_on_app();
        for _ in 0..5 {
            let (status, _) = req(
                app.clone(),
                "POST",
                "/api/auth/login",
                None,
                None,
                Some(r#"{"username":"alice","password":"wrong"}"#.into()),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = req(
            app,
            "POST",
            "/api/auth/login",
            None,
            None,
            Some(r#"{"username":"alice","password":"password8"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    /// add-webui-auth-switch Scenario「测试环境关闭」：凭据文件存在但开关
    /// 关闭（接线层注入 disabled handle）→ 全路由免登录、me 报 enabled:false。
    #[tokio::test]
    async fn switch_off_disables_auth_even_with_credentials() {
        // disabled handle 的 path 为空：enabled() 恒 false，与凭据文件无关。
        let app = build_router_with_auth(
            Arc::new(FakeBackend::new()),
            GatewayInfo::default(),
            CardConfig::default(),
            None,
            Arc::new(ConfigAgentKindProvider::new(Vec::new())),
            30,
            Arc::new(AuthHandle::disabled()),
        );

        // API 免登录放行。
        let (status, _) = req(app.clone(), "GET", "/api/summary", None, None, None).await;
        assert_eq!(status, StatusCode::OK, "开关关闭时 /api 必须免登录");

        // WS 不再被鉴权拦截（401 之外的状态码——升级本身会因缺参数失败）。
        let (status, _) = req(app.clone(), "GET", "/ws", None, None, None).await;
        assert_ne!(status, StatusCode::UNAUTHORIZED, "开关关闭时 /ws 不做鉴权拦截");

        // me 报 enabled:false → 前端不渲染登录页。
        let (_, body) = req(app, "GET", "/api/auth/me", None, None, None).await;
        assert!(
            body.contains("\"enabled\":false") && body.contains("\"authenticated\":false"),
            "{body}"
        );
    }
}
