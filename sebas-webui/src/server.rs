//! axum server setup for the WebUI: JSON API + WebSocket + embedded SPA.

use crate::admin::{self, AdminAdapter, AdminState};
use crate::agent_kinds::{AgentKindProvider, AgentKindSource, ConfigAgentKindProvider};
use crate::api;
use crate::assets;
use crate::auth::{AuthHandle, SESSION_COOKIE_NAME};
use crate::models::RouterInfo;
use crate::rbac::Permission;
use crate::routes;
use crate::session_backend::SessionBackend;
use crate::skills::{SkillsService, UnwiredSkills};
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::serve;
use sebas_feishu::cards::CardConfig;
use std::sync::Arc;
use std::time::Instant;

/// Shared state for the WebUI server. All session data flows through the
/// backend seam — the webui crate never touches `DispatchHandle` (the
/// in-process case wraps it inside `InProcessBackend`; the standalone case
/// speaks the core session channel over a Unix socket).
#[derive(Clone)]
pub struct WebUiState {
    pub backend: Arc<dyn SessionBackend>,
    pub router: RouterInfo,
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
    /// 工作区根目录（add-workspace-root）：恒有值的单一机器级边界。项目
    /// 注册、项目列表、browse-dirs 的起点与显式 root 都收敛到它之内；由
    /// 装配方经 `resolve_workspace_root`（env > config > cwd 回退 + 告警）
    /// 计算后注入。
    pub workspace_root: std::path::PathBuf,
    /// skills 管理面（add-agent-skills 5.1）：仓操作接缝，实现在主 crate
    /// （复用 core 的扫仓/删除/投影），未接线的最小装配按空仓诚实退化。
    pub skills: Arc<dyn SkillsService>,
}

/// 便捷装配形态（测试 / 最小入口）的 workspace root 缺省：进程 cwd 回退、
/// 不告警——正式装配点（webui_cmd / run）经 `resolve_workspace_root` 负责
/// env > config 解析与回退告警（add-workspace-root D5）。
fn fallback_workspace_root() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// Build the axum Router with all WebUI routes.
pub fn build_router(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
) -> Router {
    build_router_full(
        backend,
        router,
        card_config,
        None,
        Arc::new(ConfigAgentKindProvider::new(Vec::new())),
        30,
        Arc::new(AuthHandle::disabled()),
        fallback_workspace_root(),
        Arc::new(UnwiredSkills),
    )
}

/// Build the axum Router with an explicit agent-kind provider (tests inject a
/// canned provider; production can inject a config-driven one).
pub fn build_router_with_agent_kind_provider(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
    agent_kinds: Arc<dyn AgentKindProvider>,
) -> Router {
    build_router_full(
        backend,
        router,
        card_config,
        None,
        agent_kinds,
        30,
        Arc::new(AuthHandle::disabled()),
        fallback_workspace_root(),
        Arc::new(UnwiredSkills),
    )
}

/// Build the axum Router with optional watchdog admin adapter.
pub fn build_router_with_admin_adapter(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
) -> Router {
    build_router_full(
        backend,
        router,
        card_config,
        admin_adapter,
        Arc::new(ConfigAgentKindProvider::new(Vec::new())),
        30,
        Arc::new(AuthHandle::disabled()),
        fallback_workspace_root(),
        Arc::new(UnwiredSkills),
    )
}

/// Build the axum Router with an explicit auth handle（登录鉴权接线入口）。
pub fn build_router_with_auth(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    agent_kinds: Arc<dyn AgentKindProvider>,
    archive_retention_days: u64,
    auth: Arc<AuthHandle>,
) -> Router {
    build_router_full(
        backend,
        router,
        card_config,
        admin_adapter,
        agent_kinds,
        archive_retention_days,
        auth,
        fallback_workspace_root(),
        Arc::new(UnwiredSkills),
    )
}

/// Build the axum Router with an explicit workspace root（add-workspace-root）：
/// 测试与特殊装配注入单一机器级边界的入口，语义同 `build_router_with_auth`
/// + 指定根（取代已退役的 `build_router_with_allowed_roots` 多根白名单形态）。
pub fn build_router_with_workspace_root(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
    agent_kinds: Arc<dyn AgentKindProvider>,
    auth: Arc<AuthHandle>,
    workspace_root: std::path::PathBuf,
) -> Router {
    build_router_full(
        backend,
        router,
        card_config,
        None,
        agent_kinds,
        30,
        auth,
        workspace_root,
        Arc::new(UnwiredSkills),
    )
}

/// Build the axum Router with an explicit skills service（add-agent-skills
/// 5.1）：handler 级测试注入真文件系统服务的入口（workspace root 形态 +
/// skills）；生产装配走 [`run_with_admin_adapter_and_auth`]。
pub fn build_router_with_skills(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
    agent_kinds: Arc<dyn AgentKindProvider>,
    auth: Arc<AuthHandle>,
    workspace_root: std::path::PathBuf,
    skills: Arc<dyn SkillsService>,
) -> Router {
    build_router_full(
        backend,
        router,
        card_config,
        None,
        agent_kinds,
        30,
        auth,
        workspace_root,
        skills,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_router_full(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    agent_kinds: Arc<dyn AgentKindProvider>,
    archive_retention_days: u64,
    auth: Arc<AuthHandle>,
    workspace_root: std::path::PathBuf,
    skills: Arc<dyn SkillsService>,
) -> Router {
    let state = WebUiState {
        backend,
        router,
        started_at: Instant::now(),
        card_config,
        agent_kinds,
        archive_retention_days,
        auth,
        workspace_root,
        skills,
    };

    // Core SPA + API + WS routes, bound to WebUiState.
    let core = Router::new()
        .route("/", get(assets::index))
        .route("/assets/{*path}", get(assets::asset))
        // （fix-webui-approval-restore-and-session-identity 5.4）本地图标
        // 子集（`<wa-icon>` 经 setIconPath('/icons') 同源取 SVG）。
        .route("/icons/{*path}", get(assets::icons_file))
        .route("/health", get(routes::health))
        .route(
            "/api/sessions",
            get(api::sessions_list).post(api::create_session),
        )
        .route("/api/sessions/{key}", get(api::session_detail))
        .route("/api/sessions/{key}/message", post(api::send_message))
        // workbench-interaction-polish 1.2：中断在飞 turn（cancel 链路 BFF 面）。
        .route("/api/sessions/{key}/cancel", post(api::cancel_session))
        // 聚焦即拉起（workbench-live-conversation-flow 3.1）。
        .route("/api/sessions/{key}/activate", post(api::activate_session))
        .route("/api/sessions/{key}/model", post(api::set_session_model))
        // （add-agent-mode-selection）中程切换会话权限模式。
        .route("/api/sessions/{key}/mode", post(api::set_session_mode))
        .route("/api/sessions/{key}/close", post(api::close_session))
        .route("/api/sessions/{key}/switch", post(api::switch_session))
        // fix-webui-approval-restore-and-session-identity 1.2：待批审批读模型
        // （刷新/重连后审批面重建的数据源）。
        .route("/api/sessions/{key}/approvals", get(api::session_approvals))
        // fix-webui-approval-restore-and-session-identity 5.1：会话命名。
        .route("/api/sessions/{key}/label", post(api::set_session_label))
        // workbench-turn-queue 6.2：待生效提交的管理面（remove / move）。
        .route(
            "/api/sessions/{key}/pending/{pending_id}/remove",
            post(api::pending_remove),
        )
        .route(
            "/api/sessions/{key}/pending/{pending_id}/move",
            post(api::pending_move),
        )
        .route("/api/summary", get(api::summary))
        .route(
            "/api/permissions/{request_id}/answer",
            post(api::answer_permission),
        )
        .route("/api/settings", get(api::settings))
        .route("/api/provider-presets", get(routes::provider_presets))
        .route("/api/provider-defaults", get(routes::provider_defaults))
        .route("/api/providers", get(routes::providers_list))
        .route("/api/about", get(api::about))
        // split-env-vars-settings-section 1.1：环境变量只读清单（webui 自身
        // env，不依赖 core 通道）。/api/ 前缀落进既有 auth_guard。
        .route("/api/env", get(api::env_vars))
        .route("/api/agents", get(api::agent_kinds))
        // add-remote-execution-node 8.2：执行节点可用性（本机恒在线 + core 注册表）。
        .route("/api/nodes", get(api::nodes))
        .route("/api/auth/me", get(api::auth_me))
        .route("/api/auth/login", post(api::auth_login))
        .route("/api/auth/logout", post(api::auth_logout))
        // add-webui-multiuser-rbac 3.2：首启设置页建 root（零用户专属，
        // design D4）与 root 的用户管理面（design D6；角色执法在 auth_guard
        // 的 required_permission 中央表，users.manage = 仅 root）。
        .route("/api/auth/setup", post(api::auth_setup))
        .route("/api/users", get(api::users_list).post(api::users_create))
        .route("/api/users/{id}/password", post(api::users_set_password))
        .route("/api/users/{id}/role", post(api::users_set_role))
        .route("/api/users/{id}/enabled", post(api::users_set_enabled))
        .route("/api/users/{id}", delete(api::users_delete))
        .route(
            "/api/projects",
            get(api::projects_list).post(api::projects_add),
        )
        .route("/api/fs/browse-dirs", get(api::browse_dirs))
        .route("/api/archive", get(api::archive_list))
        .route("/api/archive/{key}", get(api::archive_detail))
        .route("/api/sessions/{key}/archive", post(api::archive_session))
        .route("/api/sessions/{key}/restore", post(api::restore_session))
        .route("/api/projects/reorder", post(api::projects_reorder))
        .route("/api/projects/{id}/remove", post(api::projects_remove))
        .route("/api/projects/{id}/branch", get(api::projects_branch))
        // add-agent-skills 5.1：skills 管理面（列表 / 详情 / 只删仓 / 投影）。
        // static `sync` 与 `{name}` 不同方法不冲突，静态段优先匹配。
        .route("/api/skills", get(crate::skills::skills_list))
        .route("/api/skills/sync", post(crate::skills::skills_sync))
        .route(
            "/api/skills/{name}",
            get(crate::skills::skills_detail).delete(crate::skills::skills_delete),
        )
        .route("/ws", get(api::ws_handler))
        .with_state(state.clone());

    // provider 管理面 mutation 子 router：独立子 router，守卫只套这一组
    // （POST-only + loopback origin check）。
    let router_mutations = Router::new()
        .route("/api/providers", post(routes::provider_create))
        .route(
            "/api/providers/{name}",
            axum::routing::put(routes::provider_update).delete(routes::provider_delete),
        )
        .route("/api/providers/{name}/probe", post(routes::provider_probe))
        .route("/api/model-aliases", post(routes::alias_create))
        .route(
            "/api/model-aliases/{alias}",
            axum::routing::put(routes::alias_update).delete(routes::alias_delete),
        )
        .layer(axum::middleware::from_fn(routes::provider_mutation_guard))
        .with_state(state.clone());

    // The JSON admin API is always mounted: without an adapter, reads report
    // `adapter_ok: false` and mutations answer 503 (honest degradation).
    // It carries its own AdminState, merged as a stateless Router.
    // 登录鉴权层套在 merge 之后的全量路由上（按路径选择：/api/*、
    // /ws；静态资源与 /health 放行），webui 登录未启用时
    // 各路由维持自身原有的防护（admin env-password、provider origin check）。
    core.merge(router_mutations)
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

/// `GET /api/auth/me` 与登录/登出/首启设置端点不受鉴权门拦截（前端需要先
/// 探明状态；setup 必须在无会话可达时建 root）。setup 的同源校验不豁免，
/// 见 [`auth_guard`]。
fn is_auth_exempt_path(path: &str) -> bool {
    path == "/api/auth/login"
        || path == "/api/auth/me"
        || path == "/api/auth/logout"
        || path == "/api/auth/setup"
}

/// 需要登录的路径面：JSON API、WebSocket。静态 SPA 资源与
/// `/health`（watchdog 探活）保持公开。
fn is_protected_path(path: &str) -> bool {
    path == "/ws" || path.starts_with("/api/")
}

/// RBAC 中央权限表（add-webui-multiuser-rbac 3.1，design D3）：`路径 +
/// 方法 → 所需权限`；`None` = 认证即可、不按角色执法（spec 权限表最后一行
/// 「只读」不设权限词）。`auth_guard` 认证通过后查表执法，不足即 403。
///
/// 逐路由审计（对照 design D3 表与 [`build_router_full`] 的实际路由）：
///
/// | 路由面 | 方法 | 权限 |
/// |---|---|---|
/// | `/api/users*`（列表/创建/改密/改角色/启停/删除） | 全部 | `users.manage`（spec：用户管理面**全部**仅限 root，含列表） |
/// | `/api/admin/*`（状态/事件/服务启停/升级/回滚/restart-core/login） | 全部 | `services.control` |
/// | `/api/settings`（卡片/显示偏好写） | 非安全方法 | `settings.manage`（GET 认证即可） |
/// | `/api/sessions*` 写、`/api/projects*` 写、`/api/permissions/*`（answer） | 非安全方法 | `sessions.write` |
/// | `/api/providers*`、`/api/provider-presets`、`/api/provider-defaults`、`/api/model-aliases*`（provider 管理面读 + 写） | 全部 | 无——design D3 明确排除在角色执法外，仅登录门 + 自身守卫（POST-only + origin） |
/// | `/api/skills*`（skills 管理面读 + 删/sync） | 全部 | 无——add-agent-skills 5.3：与 provider 管理面**同一权限档**（读=管理面读、删/sync=管理面写，都不按角色执法）；仅登录门 + 非安全方法同源校验 |
/// | 其余 `/api/*`（summary / sessions 与 projects 读 / env / agents / nodes / about / archive 读 / browse-dirs）与 `/ws` | 全部 | 无（认证即可，viewer 可读） |
/// | `/api/auth/{login,logout,setup,me}` | — | 豁免路径（[`is_auth_exempt_path`]），不进本表 |
///
/// admin 控制面自身的 `SEBAS_CONTROL_SECRET` 会话保持第二层不动；RBAC 只
/// 是在其外再按角色拦截 viewer/member（design D3）。
fn required_permission(path: &str, method: &str) -> Option<Permission> {
    // 用户管理面：列表也在内——非 root 一律 403，即使已认证（spec
    // 「用户管理（root 专用）」）。
    if path == "/api/users" || path.starts_with("/api/users/") {
        return Some(Permission::UsersManage);
    }
    // provider 管理面（读 + 写）：不纳入 RBAC（design D3 / proposal
    // Non-goals）。显式列出，防止未来新增 `/api/*` 规则误捕这些路径。
    if path == "/api/providers"
        || path.starts_with("/api/providers/")
        || path == "/api/provider-presets"
        || path == "/api/provider-defaults"
        || path == "/api/model-aliases"
        || path.starts_with("/api/model-aliases/")
    {
        return None;
    }
    // skills 管理面（add-agent-skills 5.3）：挂到 provider 管理面同一权限档
    // ——读（list/detail）= 管理面读、删/sync = 管理面写，都只要求登录 +
    // 非安全方法同源校验（auth_guard 既有防线），不按角色执法。显式列出，
    // 防止未来新增 `/api/*` 规则误捕这些路径。
    if path == "/api/skills" || path == "/api/skills/sync" || path.starts_with("/api/skills/") {
        return None;
    }
    // admin 控制面（含服务启停/升级/回滚——同一控制面）。
    if path.starts_with("/api/admin/") {
        return Some(Permission::ServicesControl);
    }
    let mutating = !is_safe_method(method);
    if path == "/api/settings" {
        return mutating.then_some(Permission::SettingsManage);
    }
    // 会话与项目写（创建/发消息/取消/模型/mode/关闭/切换/pending 管理/
    // 归档/恢复/项目增删排序/审批应答）。
    if path == "/api/sessions"
        || path.starts_with("/api/sessions/")
        || path == "/api/projects"
        || path.starts_with("/api/projects/")
        || path.starts_with("/api/permissions/")
    {
        return mutating.then_some(Permission::SessionsWrite);
    }
    None
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

/// 鉴权门（add-webui-multiuser-rbac 3.1 重写）：鉴权开启时受保护路径需要
/// 有效会话 cookie，且按请求实时解析用户身份（禁用/删号即刻失效）。执行
/// 顺序：
///
/// 1. 非受保护路径 / 豁免路径放行（setup 保留非安全方法同源校验——spec
///    「首启 root 引导」：设置页与其它受保护 API 一样受同源校验）；
/// 2. 开关关闭（`auth = false`）全放行（原行为）；
/// 3. 会话 cookie → [`AuthHandle::identity_for_session`] 实时身份解析
///    （design D5）；解析不出（无 cookie / 会话过期 / 绑定用户已删或已
///    禁用 / admin 控制面哨兵 user_id=0）一律 401，fail-closed；
/// 4. [`required_permission`] 中央表执法：角色不足 403（spec：执法完全在
///    服务端路由层完成）；
/// 5. 非安全方法同源校验（CSRF 防线，现状保留）；
/// 6. 把 [`crate::auth::Identity`] 塞进 request extensions 供 handler 按需读取
///    （design D3：如用户管理端点取 user_id 判「不能删自己」）。
async fn auth_guard(State(state): State<WebUiState>, req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let method = req.method().as_str().to_owned();
    if !is_protected_path(&path) {
        return next.run(req).await;
    }
    if is_auth_exempt_path(&path) {
        // setup 无会话可达，但同源校验不豁免（spec「首启 root 引导」）。
        if path == "/api/auth/setup"
            && !is_safe_method(&method)
            && let Some(resp) = cross_origin_rejection(&req)
        {
            return resp;
        }
        return next.run(req).await;
    }
    if !state.auth.enabled() {
        return next.run(req).await;
    }

    // 认证 + 实时身份解析：会话有效但绑定的用户已被禁用/删除时按未认证
    // 处理（spec「禁用用户即刻失效」）。
    let identity = match extract_webui_session_cookie(req.headers()) {
        Some(session_id) => state.auth.identity_for_session(&session_id).await,
        None => None,
    };
    let Some(identity) = identity else {
        // /ws 在升级前拒绝；API 一律 JSON 401（前端据此弹登录页）。
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "sebas-session")],
            axum::Json(serde_json::json!({ "error": "authentication required" })),
        )
            .into_response();
    };

    // RBAC 中央表执法：无对应权限的已认证请求 403（而非仅前端隐藏入口）。
    if let Some(required) = required_permission(&path, &method)
        && !identity.role.has(required)
    {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({
                "error": format!("权限不足：{} 角色无权执行该操作", identity.role),
            })),
        )
            .into_response();
    }

    // CSRF：浏览器发起的跨站写请求会带 Origin 头——与 Host 不一致即拒绝。
    // SameSite=Lax cookie 已挡掉绝大多数跨站携带，这里兜底非浏览器场景。
    if !is_safe_method(&method)
        && let Some(resp) = cross_origin_rejection(&req)
    {
        return resp;
    }

    let mut req = req;
    req.extensions_mut().insert(identity);
    next.run(req).await
}

/// 非 safe 方法的同源校验：`Origin` 头存在且与 `Host` 不一致 → 403 响应；
/// 无 Origin（CLI/curl）或同源 → None（放行）。
fn cross_origin_rejection(req: &Request<Body>) -> Option<Response> {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())?;
    let same_origin = origin_authority(origin)
        .zip(
            req.headers()
                .get(header::HOST)
                .and_then(|h| h.to_str().ok()),
        )
        .map(|(origin_host, host)| origin_host.eq_ignore_ascii_case(host))
        .unwrap_or(false);
    if same_origin {
        return None;
    }
    Some(
        (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "cross-origin request rejected" })),
        )
            .into_response(),
    )
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
    router: RouterInfo,
    card_config: CardConfig,
    agent_kinds: Vec<AgentKindSource>,
    listener: tokio::net::TcpListener,
) {
    run_full(
        backend,
        router,
        card_config,
        Arc::new(ConfigAgentKindProvider::new(agent_kinds)),
        listener,
        None,
        30,
        Arc::new(AuthHandle::disabled()),
        fallback_workspace_root(),
        Arc::new(UnwiredSkills),
    )
    .await;
}

/// Run the WebUI server with an optional watchdog admin adapter.
pub async fn run_with_admin_adapter(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
    agent_kinds: Vec<AgentKindSource>,
    listener: tokio::net::TcpListener,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
) {
    run_full(
        backend,
        router,
        card_config,
        Arc::new(ConfigAgentKindProvider::new(agent_kinds)),
        listener,
        admin_adapter,
        30,
        Arc::new(AuthHandle::disabled()),
        fallback_workspace_root(),
        Arc::new(UnwiredSkills),
    )
    .await;
}

/// Run the WebUI server with an auth handle（登录鉴权接线入口）。
/// `workspace_root` 是恒有值的单一机器级边界（add-workspace-root）：browse-dirs
/// 的起点与显式 root 约束、项目注册/列表/查看的范围判定都以它为准；由装配方
/// 经 `resolve_workspace_root`（env > config > cwd 回退 + 告警）计算。
/// `skills` 是 skills 管理面的仓操作接缝（add-agent-skills 5.1，生产装配点
/// webui_cmd / run 注入 config 装配的真实现）。`agent_kinds` 由装配点以
/// `ConfigAgentKindProvider::with_default_kind` 构造（preselect-last-used-model
/// 3.2：default agent kind 一并从 config 注入，供 /api/about 下发）。
#[allow(clippy::too_many_arguments)]
pub async fn run_with_admin_adapter_and_auth(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
    agent_kinds: Arc<dyn AgentKindProvider>,
    listener: tokio::net::TcpListener,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    auth: Arc<AuthHandle>,
    workspace_root: std::path::PathBuf,
    archive_retention_days: u64,
    skills: Arc<dyn SkillsService>,
) {
    run_full(
        backend,
        router,
        card_config,
        agent_kinds,
        listener,
        admin_adapter,
        archive_retention_days,
        auth,
        workspace_root,
        skills,
    )
    .await;
}

/// bind 地址 → 用户可点开的访问 URL：`0.0.0.0` / `::` 等通配地址在浏览器
/// 里不可点，呈现为 `127.0.0.1`；其余（loopback 或具体网卡地址）原样。
fn access_url(addr: std::net::SocketAddr) -> String {
    if addr.ip().is_unspecified() {
        format!("http://127.0.0.1:{}", addr.port())
    } else {
        format!("http://{addr}")
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_full(
    backend: Arc<dyn SessionBackend>,
    router: RouterInfo,
    card_config: CardConfig,
    agent_kinds: Arc<dyn AgentKindProvider>,
    listener: tokio::net::TcpListener,
    admin_adapter: Option<Arc<dyn AdminAdapter>>,
    archive_retention_days: u64,
    auth: Arc<AuthHandle>,
    workspace_root: std::path::PathBuf,
    skills: Arc<dyn SkillsService>,
) {
    let addr = listener.local_addr().expect("bound listener");
    // 引导用户：就绪日志直接给出可点开的访问地址 + 按鉴权形态的下一步提示。
    // 独立 `sebas webui` 进程与 `core --webui` 内嵌形态共用这一条就绪日志
    // （embedded 模式没有别的引导输出），所以引导信息放在这里而不是接线方。
    // 在 auth 被 move 进 router 之前取好判定。
    let url = access_url(addr);
    let mut hint = match auth.needs_setup() {
        true => "first run: open it to create the admin account".to_string(),
        false if auth.enabled() => "open it and sign in with your webui account".to_string(),
        false => "auth disabled: open it directly (loopback bind only)".to_string(),
    };
    if addr.ip().is_unspecified() {
        hint.push_str(&format!(
            "; LAN devices use http://<host-ip>:{}",
            addr.port()
        ));
    }
    let app = build_router_full(
        backend,
        router,
        card_config,
        admin_adapter,
        agent_kinds,
        archive_retention_days,
        auth,
        workspace_root,
        skills,
    );
    tracing::info!(%url, hint, "webui dashboard started");
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
    use crate::models::RouterInfo;
    use crate::session_backend::FakeBackend;
    use sebas_feishu::cards::CardConfig;

    #[test]
    fn full_router_builds_without_route_conflict() {
        let _app = build_router(
            std::sync::Arc::new(FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
        );
    }
}

#[cfg(test)]
mod agent_defaults_removed_tests {
    //! workbench-agent-wire-fix 3.3：/api/agent-defaults 端点退役——
    //! GET/PUT 一律 404（默认 provider/model 走 router admin providers 面，
    //! 默认 agent 按项目记忆）。用 404 路由层断言钉住退役。
    use super::*;
    use crate::models::RouterInfo;
    use crate::session_backend::FakeBackend;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use sebas_feishu::cards::CardConfig;
    use std::net::{IpAddr, SocketAddr};
    use tower::ServiceExt;

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 12345)
    }

    fn app() -> Router {
        build_router_with_auth(
            Arc::new(FakeBackend::new()),
            RouterInfo {
                listen: None,
                ..RouterInfo::default()
            },
            CardConfig::default(),
            None,
            Arc::new(crate::agent_kinds::ConfigAgentKindProvider::new(Vec::new())),
            30,
            Arc::new(AuthHandle::disabled()),
        )
    }

    async fn req(app: Router, method: &str, uri: &str, body: Option<String>) -> StatusCode {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let req = builder.body(Body::from(body.unwrap_or_default())).unwrap();
        resp_status(app.oneshot(req).await.unwrap()).await
    }

    async fn resp_status(resp: axum::response::Response) -> StatusCode {
        resp.status()
    }

    #[tokio::test]
    async fn agent_defaults_endpoint_is_gone() {
        let app = app();
        assert_eq!(
            req(app.clone(), "GET", "/api/agent-defaults", None).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            req(
                app,
                "PUT",
                "/api/agent-defaults",
                Some(r#"{"provider":"glm"}"#.into())
            )
            .await,
            StatusCode::NOT_FOUND
        );
    }
}

#[cfg(test)]
mod summary_bodies_tests {
    //! wire-webui-sebas-agent-e2e 3.1 路由层验收：`/api/summary` 把后端的
    //! 逐执行体可用性（execution_bodies）原样透传给 composer；后端不区分
    //! 执行体时该段为 null（前端降级为只看整体 reachability）。
    use super::*;
    use crate::session_backend::{ExecutionBodyStatus, FakeBackend};
    use axum::extract::ConnectInfo;
    use http_body_util::BodyExt;
    use sebas_feishu::cards::CardConfig;
    use tower::ServiceExt;

    fn test_addr() -> std::net::SocketAddr {
        std::net::SocketAddr::new(std::net::IpAddr::from([127, 0, 0, 1]), 12345)
    }

    async fn get_summary(app: Router) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method("GET")
            .uri("/api/summary")
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    #[tokio::test]
    async fn summary_passes_through_per_body_availability() {
        let backend = FakeBackend::new();
        backend.set_execution_bodies(Some(vec![
            ExecutionBodyStatus {
                name: "acp".into(),
                ok: true,
                cause: None,
            },
            ExecutionBodyStatus {
                name: "native".into(),
                ok: false,
                cause: Some("no provider credentials".into()),
            },
        ]));
        let app = build_router(
            Arc::new(backend),
            RouterInfo::default(),
            CardConfig::default(),
        );

        let (status, body) = get_summary(app).await;
        assert_eq!(status, StatusCode::OK);
        let bodies = body["execution_bodies"]
            .as_array()
            .expect("execution_bodies must be an array");
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0]["name"], "acp");
        assert_eq!(bodies[0]["ok"], true);
        assert!(bodies[0]["cause"].is_null());
        assert_eq!(bodies[1]["name"], "native");
        assert_eq!(bodies[1]["ok"], false);
        assert_eq!(bodies[1]["cause"], "no provider credentials");
    }

    #[tokio::test]
    async fn summary_reports_null_bodies_when_backend_does_not_distinguish() {
        let app = build_router(
            Arc::new(FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
        );

        let (status, body) = get_summary(app).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body["execution_bodies"].is_null(),
            "no per-body report must degrade to null: {}",
            body["execution_bodies"]
        );
    }
}

#[cfg(test)]
mod auth_guard_tests {
    //! 登录鉴权门的路由级测试：未启用时零影响；启用后 /api、/ws 要会话，
    //! 静态资源 / /health 放行；登录-使用-注销闭环；同源校验；限速。
    use super::*;
    use crate::models::RouterInfo;
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

    /// 鉴权开启的 router + 临时 auth.db（小迭代数提速；tempdir 由调用方
    /// 持有存活：AuthHandle 持有该库的连接）。
    async fn auth_on_app() -> (Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let handle = Arc::new(AuthHandle::open_with_iterations(
            dir.path().join("auth.db"),
            1000,
        ));
        handle
            .setup_root("alice", "password8")
            .await
            .expect("setup root");
        let app = build_router_with_auth(
            Arc::new(FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
            None,
            Arc::new(ConfigAgentKindProvider::new(Vec::new())),
            30,
            handle,
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
        let req = builder.body(Body::from(body.unwrap_or_default())).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn auth_disabled_keeps_routes_open() {
        let app = build_router(
            Arc::new(FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
        );
        let (status, _) = req(app, "GET", "/api/summary", None, None, None).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn auth_on_blocks_api_and_ws_but_not_static_or_health() {
        let (app, _dir) = auth_on_app().await;

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
        let (app, _dir) = auth_on_app().await;

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
        assert!(
            cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"),
            "{cookie}"
        );
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
        let (_, body) = req(
            app.clone(),
            "GET",
            "/api/auth/me",
            Some(&session),
            None,
            None,
        )
        .await;
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
        let (app, _dir) = auth_on_app().await;
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
        let (app, _dir) = auth_on_app().await;
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

    /// spec「首启 root 引导」：setup 端点与其它受保护 API 一样受非安全
    /// 方法同源校验——跨站 POST 携带合法载荷也被 403 拒绝，库保持零用户。
    #[tokio::test]
    async fn setup_rejects_cross_origin_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let handle = Arc::new(AuthHandle::open_with_iterations(
            dir.path().join("auth.db"),
            1000,
        ));
        assert!(handle.needs_setup(), "夹具应是零用户库");
        let app = build_router_with_auth(
            Arc::new(FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
            None,
            Arc::new(ConfigAgentKindProvider::new(Vec::new())),
            30,
            handle,
        );

        let (status, _) = req(
            app.clone(),
            "POST",
            "/api/auth/setup",
            None,
            Some("http://evil.example"),
            Some(r#"{"username":"evil","password":"password8"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "跨站 setup 必须 403");

        // 同源 setup 放行（零用户 → 建 root）。
        let (status, _) = req(
            app,
            "POST",
            "/api/auth/setup",
            None,
            Some("http://127.0.0.1:12345"),
            Some(r#"{"username":"cupen","password":"password8"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    /// add-webui-auth-switch Scenario「测试环境关闭」：开关关闭（接线层注入
    /// disabled handle）→ 全路由免登录、me 报 enabled:false。
    #[tokio::test]
    async fn switch_off_disables_auth_even_with_credentials() {
        // disabled handle 的 path 为空：enabled() 恒 false。
        let app = build_router_with_auth(
            Arc::new(FakeBackend::new()),
            RouterInfo::default(),
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
        assert_ne!(
            status,
            StatusCode::UNAUTHORIZED,
            "开关关闭时 /ws 不做鉴权拦截"
        );

        // me 报 enabled:false → 前端不渲染登录页。
        let (_, body) = req(app, "GET", "/api/auth/me", None, None, None).await;
        assert!(
            body.contains("\"enabled\":false") && body.contains("\"authenticated\":false"),
            "{body}"
        );
    }

    // ── RBAC 中央表执法（add-webui-multiuser-rbac 3.1，design D3）──

    use crate::rbac::Role;

    /// 四角色夹具：root alice（setup 建）、admin ada、member bob、viewer vic
    /// （store 直建）。返回句柄供禁用/查 id 等库操作；tempdir 由调用方持有
    /// 存活。
    async fn rbac_app() -> (Router, tempfile::TempDir, Arc<AuthHandle>) {
        let dir = tempfile::tempdir().unwrap();
        let auth = Arc::new(AuthHandle::open_with_iterations(
            dir.path().join("auth.db"),
            1000,
        ));
        auth.setup_root("alice", "password8").await.unwrap();
        let store = auth.user_store().expect("用户库在场");
        store.create("ada", "password8", Role::Admin).unwrap();
        store.create("bob", "password8", Role::Member).unwrap();
        store.create("vic", "password8", Role::Viewer).unwrap();
        let app = build_router_with_auth(
            Arc::new(FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
            None,
            Arc::new(ConfigAgentKindProvider::new(Vec::new())),
            30,
            auth.clone(),
        );
        (app, dir, auth)
    }

    /// 走登录端点换会话 cookie（形如 `sebas_webui_session=…`）。
    async fn login_cookie(app: &Router, username: &str, password: &str) -> String {
        let login_req = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header("content-type", "application/json")
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()))
            .body(Body::from(format!(
                r#"{{"username":"{username}","password":"{password}"}}"#
            )))
            .unwrap();
        let resp = app.clone().oneshot(login_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "login as {username}");
        let cookie = resp
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .to_string();
        cookie.split(';').next().unwrap().trim().to_string()
    }

    /// spec「viewer 只读」：viewer 的会话/项目写 403，读面放行。
    #[tokio::test]
    async fn viewer_writes_are_403_but_reads_pass() {
        let (app, _dir, _auth) = rbac_app().await;
        let vic = login_cookie(&app, "vic", "password8").await;

        let (status, body) = req(
            app.clone(),
            "POST",
            "/api/sessions",
            Some(&vic),
            None,
            Some(r#"{"agent":"native"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        // 项目写同表（sessions.write）。
        let (status, body) = req(
            app.clone(),
            "POST",
            "/api/projects",
            Some(&vic),
            None,
            Some(r#"{"path":"/tmp/x"}"#.into()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        // 设置写（settings.manage）。
        let (status, body) = req(
            app.clone(),
            "POST",
            "/api/settings",
            Some(&vic),
            None,
            Some("{}".into()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        // 读面：认证即可（viewer 可读）。
        let (status, _) = req(app.clone(), "GET", "/api/summary", Some(&vic), None, None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = req(app, "GET", "/api/users", Some(&vic), None, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "用户管理面 viewer 也 403");
    }

    /// spec「member 不能碰设置与服务」+「admin 不能管用户」：用户管理仅
    /// root（列表也在内），admin/member 一律 403；root 通过且列表无哈希。
    #[tokio::test]
    async fn member_and_admin_cannot_manage_users_root_can() {
        let (app, _dir, _auth) = rbac_app().await;
        let bob = login_cookie(&app, "bob", "password8").await;
        let ada = login_cookie(&app, "ada", "password8").await;
        let alice = login_cookie(&app, "alice", "password8").await;

        for (who, cookie) in [("member", &bob), ("admin", &ada)] {
            let (status, body) =
                req(app.clone(), "GET", "/api/users", Some(cookie), None, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who} 列表必须 403: {body}");
            let (status, body) = req(
                app.clone(),
                "POST",
                "/api/users",
                Some(cookie),
                None,
                Some(r#"{"username":"x","password":"password8","role":"member"}"#.into()),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who} 建户必须 403: {body}");
        }

        // root 通过：列表 200、四个用户、无哈希字段。
        let (status, body) = req(app.clone(), "GET", "/api/users", Some(&alice), None, None).await;
        assert_eq!(status, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["users"].as_array().expect("users array").len(), 4);
        let raw = body;
        for leak in ["salt", "hash", "iterations"] {
            assert!(!raw.contains(leak), "列表泄漏 {leak}: {raw}");
        }

        // 匿名仍是 401（登录门先于角色执法）。
        let (status, _) = req(app, "GET", "/api/users", None, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// member 拥有 sessions.write：会话写到达 handler（绝不 401/403）；
    /// admin 控制面（services.control）对 member 是 403。
    #[tokio::test]
    async fn member_session_writes_pass_and_admin_plane_is_403() {
        let (app, _dir, _auth) = rbac_app().await;
        let bob = login_cookie(&app, "bob", "password8").await;

        let (status, body) = req(
            app.clone(),
            "POST",
            "/api/sessions",
            Some(&bob),
            None,
            Some(r#"{"agent":"native"}"#.into()),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "member 会话写不得被角色拦截: {status} {body}"
        );
        assert_ne!(status, StatusCode::UNAUTHORIZED, "{body}");

        // admin 控制面对 member：RBAC 层 403（在 control-secret 第二层之外）。
        let (status, body) = req(
            app.clone(),
            "GET",
            "/api/admin/status",
            Some(&bob),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        // root 过 RBAC 层后到达 admin 自身守卫（无 secret + loopback →
        // 非 403 的第二层语义）。
        let alice = login_cookie(&app, "alice", "password8").await;
        let (status, _) = req(app, "GET", "/api/admin/status", Some(&alice), None, None).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "root 不得被 RBAC 层拦截");
    }

    /// spec「router BFF 仅登录门」：member 调 router BFF 写不因角色被拦
    /// （登录门 + 自身 POST-only/origin 守卫照常）。FakeBackend 无 router
    /// 可达 → 503（到达 handler 的证据），绝不是 401/403。
    #[tokio::test]
    async fn member_router_bff_write_is_not_role_blocked() {
        let (app, _dir, _auth) = rbac_app().await;
        let bob = login_cookie(&app, "bob", "password8").await;
        let (status, body) = req(
            app.clone(),
            "POST",
            "/api/providers",
            Some(&bob),
            None,
            Some("{}".to_string()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "member 的 router BFF 写必须穿过角色执法到达 handler: {status} {body}"
        );

        // 未登录仍被登录门拦（BFF 只豁免角色执法，不豁免登录）。
        let (status, _) = req(app, "POST", "/api/providers", None, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// add-agent-skills 5.3：skills 四端点挂 provider 管理面**同一权限档**
    /// （读=管理面读、删/sync=管理面写——都不按角色执法，只要求登录）。
    /// viewer 读放行；member 的删/sync 到达 handler（UnwiredSkills 下分别
    /// 得 404/200，绝不是 401/403）；匿名仍被登录门拦 401。
    #[tokio::test]
    async fn skills_endpoints_follow_provider_plane_permission_tier() {
        let (app, _dir, _auth) = rbac_app().await;
        let vic = login_cookie(&app, "vic", "password8").await;
        let bob = login_cookie(&app, "bob", "password8").await;

        // viewer 读（列表）：认证即可，与 provider 管理面读同档。
        let (status, body) = req(app.clone(), "GET", "/api/skills", Some(&vic), None, None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "viewer 读 skills 不得被角色拦: {body}"
        );

        // member 写（删 / sync）：穿过角色执法到达 handler。
        let (status, body) = req(
            app.clone(),
            "DELETE",
            "/api/skills/definitely-absent",
            Some(&bob),
            None,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "member 删 skills 必须到达 handler（404 缺条目）: {status} {body}"
        );
        let (status, body) = req(
            app.clone(),
            "POST",
            "/api/skills/sync",
            Some(&bob),
            None,
            Some("{}".into()),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "member sync 必须到达 handler: {status} {body}"
        );

        // 匿名：登录门照常（豁免的是角色执法，不是登录）。
        let (status, _) = req(app, "GET", "/api/skills", None, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// spec「禁用用户即刻失效」：禁用后既有会话的下一个请求 401。
    #[tokio::test]
    async fn disabled_users_existing_session_is_unauthorized() {
        let (app, _dir, auth) = rbac_app().await;
        let bob = login_cookie(&app, "bob", "password8").await;

        // 用户管理端点的库层效果（端点行为在 api_endpoints_test 覆盖）。
        let uid = auth
            .user_store()
            .unwrap()
            .get_by_username("bob")
            .unwrap()
            .unwrap()
            .id;
        auth.user_store().unwrap().set_enabled(uid, false).unwrap();

        let (status, _) = req(app, "GET", "/api/summary", Some(&bob), None, None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "禁用用户的既有会话必须 401"
        );
    }
}

#[cfg(test)]
mod workspace_root_tests {
    //! add-workspace-root 路由级验收：workspace root 恒存在——browse-dirs 的
    //! 显式 root 与项目注册路径都必须落在它之内，无参浏览起点即根本身。
    use super::*;
    use crate::models::RouterInfo;
    use crate::session_backend::FakeBackend;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use http_body_util::BodyExt;
    use sebas_feishu::cards::CardConfig;
    use serde_json::Value;
    use std::net::{IpAddr, SocketAddr};
    use tower::ServiceExt;
    // 项目 id 的 URL 段转义（非会话键编解码）：urlencoding 原样可用。
    use urlencoding::encode;

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 12345)
    }

    fn app_with_workspace_root(root: std::path::PathBuf) -> Router {
        build_router_with_workspace_root(
            Arc::new(FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
            Arc::new(crate::agent_kinds::ConfigAgentKindProvider::new(Vec::new())),
            Arc::new(AuthHandle::disabled()),
            root,
        )
    }

    async fn req(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<String>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let req = builder.body(Body::from(body.unwrap_or_default())).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, v)
    }

    /// 项目注册走文件注册表降级路径（FakeBackend 无 state_mutate 覆写），
    /// env 是进程级的：与 projects.rs 测试共用同一把串行锁，用完恢复。
    /// 锁跨 await 是刻意的——正是要把整个异步测试体串行化。
    #[allow(clippy::await_holding_lock)]
    async fn with_registry_env<F, R>(f: F) -> R
    where
        F: AsyncFnOnce() -> R,
    {
        let _g = crate::projects::test_env_lock();
        let prev = std::env::var("SEBAS_PROJECTS_PATH").ok();
        let registry = tempfile::tempdir().unwrap();
        // SAFETY: test_env_lock 保证进程内注册表相关测试串行。
        unsafe {
            std::env::set_var("SEBAS_PROJECTS_PATH", registry.path().join("p.json"));
        }
        let r = f().await;
        // SAFETY: 同上。
        unsafe {
            match prev {
                Some(p) => std::env::set_var("SEBAS_PROJECTS_PATH", p),
                None => std::env::remove_var("SEBAS_PROJECTS_PATH"),
            }
        }
        r
    }

    #[tokio::test]
    async fn project_add_rejects_path_outside_workspace_root() {
        let t = two_trees();
        let app = app_with_workspace_root(t.allowed.path().to_path_buf());
        // fail-closed：即使路径真实存在，越界也 400（先范围后存在性）。
        let body = serde_json::json!({ "path": t.outside.path().to_str().unwrap() });
        let (status, resp) = req(app, "POST", "/api/projects", Some(body.to_string())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let msg = resp["error"].as_str().unwrap_or_default();
        assert!(msg.contains("超出允许范围"), "got: {msg}");
    }

    #[tokio::test]
    async fn project_add_accepts_path_inside_workspace_root() {
        with_registry_env(|| async {
            let t = two_trees();
            let app = app_with_workspace_root(t.allowed.path().to_path_buf());
            let dir = t.allowed.path().join("proj");
            std::fs::create_dir_all(&dir).unwrap();
            // 回显的是 canonicalize_plain 的普通形（Windows 无 \\?\ 前缀）。
            let canonical = crate::fs::canonicalize_plain(&dir).expect("canonicalize proj dir");
            let body = serde_json::json!({ "path": dir.to_str().unwrap() });
            let (status, resp) = req(app, "POST", "/api/projects", Some(body.to_string())).await;
            assert_eq!(status, StatusCode::CREATED, "resp: {resp}");
            assert_eq!(resp["path"].as_str(), Some(canonical.as_str()));
        })
        .await;
    }

    // ── 2.1 注册执法：越界与「不存在且越界」同文案（不借 400 探测存在性）──

    #[tokio::test]
    async fn project_add_out_of_scope_error_is_identical_for_existing_and_missing_dirs() {
        let t = two_trees();
        let app = app_with_workspace_root(t.allowed.path().to_path_buf());
        let existing = t.outside.path().join("exists-dir");
        std::fs::create_dir_all(&existing).unwrap();
        let missing = t.outside.path().join("missing-dir");

        let mut msgs = Vec::new();
        for path in [existing, missing] {
            let body = serde_json::json!({ "path": path.to_str().unwrap() });
            let (status, resp) =
                req(app.clone(), "POST", "/api/projects", Some(body.to_string())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
            msgs.push(resp["error"].as_str().expect("error body").to_string());
        }
        assert_eq!(
            msgs[0], msgs[1],
            "越界与「不存在且越界」必须同文案，否则可探测目录存在性"
        );
        assert!(msgs[0].contains("超出允许范围"), "got: {}", msgs[0]);
    }

    // ── 2.2 列表执法 ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn projects_list_hides_out_of_scope_local_project_keeps_in_scope_and_remote() {
        with_registry_env(|| async {
            let t = two_trees();
            let app = app_with_workspace_root(t.allowed.path().to_path_buf());
            // 界内项目经 API 注册（FakeBackend 走文件注册表降级路径）。
            let dir = t.allowed.path().join("in-scope");
            std::fs::create_dir_all(&dir).unwrap();
            let body = serde_json::json!({ "path": dir.to_str().unwrap() });
            let (status, _) =
                req(app.clone(), "POST", "/api/projects", Some(body.to_string())).await;
            assert_eq!(status, StatusCode::CREATED);
            // 越界历史项目（升级前遗留、新二进制注册不进来的形态）直接落注册表；
            // 远端项目同入注册表（不受主控 root 辖区约束）。
            let legacy = crate::projects::add_on(
                crate::projects::LOCAL_NODE_ID,
                t.outside.path().to_str().unwrap(),
            )
            .expect("seed legacy out-of-scope entry");
            let remote =
                crate::projects::add_on("dev-box", "/srv/repo").expect("seed remote entry");

            let (_, list) = req(app, "GET", "/api/projects", None).await;
            let ids: Vec<&str> = list["projects"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|p| p["id"].as_str())
                .collect();
            assert!(
                !ids.contains(&legacy.id.as_str()),
                "越界本机项目必须隐藏: {ids:?}"
            );
            assert!(
                ids.contains(&remote.id.as_str()),
                "远端项目不受主控 root 影响: {ids:?}"
            );
            assert_eq!(ids.len(), 2, "界内本机项目 + 远端项目在列: {ids:?}");
        })
        .await;
    }

    #[tokio::test]
    async fn projects_list_hides_all_local_projects_when_root_unresolvable() {
        with_registry_env(|| async {
            let t = two_trees();
            // 先用可解析的根注册一个界内项目。
            let app = app_with_workspace_root(t.allowed.path().to_path_buf());
            let dir = t.allowed.path().join("in-scope");
            std::fs::create_dir_all(&dir).unwrap();
            let body = serde_json::json!({ "path": dir.to_str().unwrap() });
            let (status, _) =
                req(app.clone(), "POST", "/api/projects", Some(body.to_string())).await;
            assert_eq!(status, StatusCode::CREATED);
            let remote = crate::projects::add_on("dev-box", "/srv/repo").unwrap();

            // root 被删/移走（不可解析）→ 本机项目全部隐藏（fail-closed），
            // 远端项目仍在列。
            let ghost = t.allowed.path().join("__ghost_root__");
            let app = app_with_workspace_root(ghost);
            let (_, list) = req(app, "GET", "/api/projects", None).await;
            let projects = list["projects"].as_array().unwrap();
            assert!(
                projects.iter().all(|p| {
                    p["node_id"]
                        .as_str()
                        .unwrap_or(crate::projects::LOCAL_NODE_ID)
                        != crate::projects::LOCAL_NODE_ID
                }),
                "root 不可解析时本机项目必须全部隐藏: {list}"
            );
            let ids: Vec<&str> = projects.iter().filter_map(|p| p["id"].as_str()).collect();
            assert_eq!(ids, [remote.id.as_str()], "只剩远端项目: {ids:?}");
        })
        .await;
    }

    // ── 2.3 会话面执法 ────────────────────────────────────────────────────

    fn app_and_backend_with_workspace_root(root: std::path::PathBuf) -> (Router, Arc<FakeBackend>) {
        let backend = Arc::new(FakeBackend::new());
        let app = build_router_with_workspace_root(
            backend.clone(),
            RouterInfo::default(),
            CardConfig::default(),
            Arc::new(crate::agent_kinds::ConfigAgentKindProvider::new(Vec::new())),
            Arc::new(AuthHandle::disabled()),
            root,
        );
        (app, backend)
    }

    /// 本机会话行（`remote: None`）：`project_dir` 即会话绑定的项目目录。
    fn local_session(reference: &str, project_dir: Option<String>) -> sebas_dispatch::SessionInfo {
        sebas_dispatch::SessionInfo {
            channel: "web".into(),
            key: reference.into(),
            session_id: None,
            status: "active".into(),
            phase: None,
            user_prompt: None,
            last_active_unix: 0,
            project_dir,
            current_model: None,
            available_models: None,
            agent_kind: None,
            usage: None,
            backend: None,
            pending: Vec::new(),
            remote: None,
            desired_mode: sebas_dispatch::engine::ask_mode(),
            effective_mode: None,
            msg_count: 0,
            available_commands: Vec::new(),
            turn_engaged: false,
            spawn_failure_reason: None,
            parked_approvals: 0,
            label: None,
        }
    }

    fn enc_key(reference: &str) -> String {
        // 会话键编码走唯一实现（add-domain-layer 2.3）。
        sebas_channels::key::encode_channel_key("web", reference)
    }

    /// （session-parallel-liveness-and-unread-polish 1.3）spawn 失败 wire 透传：
    /// 失败会话行（/api/sessions）与详情（/api/sessions/{key}）都携带失败态
    /// （status_slug = failed）与原因原文（spawn_failure_reason）——操作者在
    /// 列表/详情就地看到「为什么失败」，不再滞留成永远排队的幽灵（
    /// session-lifecycle delta「failed spawn names the cause」的投影半边）。
    #[tokio::test]
    async fn spawn_failed_session_row_and_detail_carry_the_reason() {
        let t = two_trees();
        let (app, backend) = app_and_backend_with_workspace_root(t.allowed.path().to_path_buf());
        let mut failed = local_session("web-failed-1", None);
        failed.status = "spawn-failed".into();
        failed.spawn_failure_reason = Some("agent binary not found: claude-fake".into());
        backend.set_sessions(vec![failed]).await;
        let key = enc_key("web-failed-1");

        // 列表行：slug 投影为 failed，原因随行可见。
        let (_, list) = req(app.clone(), "GET", "/api/sessions", None).await;
        let row = &list["recent_sessions"][0];
        assert_eq!(row["status_slug"], "failed", "row: {row}");
        assert_eq!(
            row["spawn_failure_reason"], "agent binary not found: claude-fake",
            "row must name the cause: {row}"
        );

        // 详情：同一份原因原文透传（composer 就地呈现的数据源）。
        let (_, detail) = req(app, "GET", &format!("/api/sessions/{key}"), None).await;
        assert_eq!(detail["status_slug"], "failed", "detail: {detail}");
        assert_eq!(
            detail["spawn_failure_reason"], "agent binary not found: claude-fake",
            "detail must name the cause: {detail}"
        );

        // 非失败会话不得携带该键（skip_serializing_if：wire 干净）。
        let (app, backend) = app_and_backend_with_workspace_root(t.allowed.path().to_path_buf());
        backend
            .set_sessions(vec![local_session("web-ok", None)])
            .await;
        let (_, list) = req(app, "GET", "/api/sessions", None).await;
        let row = &list["recent_sessions"][0];
        assert!(
            row.get("spawn_failure_reason").is_none(),
            "healthy row must not carry the key: {row}"
        );
    }

    #[tokio::test]
    async fn detail_message_and_switch_reject_out_of_scope_local_project() {
        let t = two_trees();
        let (app, backend) = app_and_backend_with_workspace_root(t.allowed.path().to_path_buf());
        let outside = crate::fs::canonicalize_plain(t.outside.path()).expect("canonicalize");
        backend
            .set_sessions(vec![local_session("oos", Some(outside))])
            .await;
        let key = enc_key("oos");

        // detail / message / switch 一律 400 typed 拒绝，文案点名越界。
        let cases = [
            (
                axum::http::Method::GET,
                format!("/api/sessions/{key}"),
                None,
            ),
            (
                axum::http::Method::POST,
                format!("/api/sessions/{key}/message"),
                Some(r#"{"message":"hi"}"#.to_string()),
            ),
            (
                axum::http::Method::POST,
                format!("/api/sessions/{key}/switch"),
                None,
            ),
        ];
        for (method, uri, body) in cases {
            let (status, resp) = req(app.clone(), method.as_str(), &uri, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {resp}");
            let msg = resp["error"].as_str().unwrap_or_default();
            assert!(msg.contains("超出允许范围"), "{uri}: {msg}");
        }

        // switch 被拒后 focus 指针不动（拒绝不留痕迹）。
        assert!(
            backend.focused().await.is_none(),
            "拒绝的 switch 不得改 focus"
        );
    }

    #[tokio::test]
    async fn inbox_and_remote_sessions_stay_outside_the_fence() {
        let t = two_trees();
        let (app, backend) = app_and_backend_with_workspace_root(t.allowed.path().to_path_buf());
        // inbox 会话不绑目录；远端会话的 project_dir 在那台机器上，主控 root
        // 无从裁决（即使路径形似越界）。
        let mut remote = local_session("rem", Some("/srv/definitely/elsewhere".into()));
        remote.remote = Some(sebas_dispatch::RemoteSessionView {
            node_id: "dev-box".into(),
            node_status: "online".into(),
            node_cause: None,
            desired_mode: Some(sebas_dispatch::engine::ask_mode()),
            effective_mode: None,
            parked_approvals: 0,
            desired_provider: None,
            provider: None,
            provider_cause: None,
        });
        backend
            .set_sessions(vec![local_session("inbox", None), remote])
            .await;

        for reference in ["inbox", "rem"] {
            let key = enc_key(reference);
            let (status, resp) = req(
                app.clone(),
                axum::http::Method::GET.as_str(),
                &format!("/api/sessions/{key}"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{reference}: {resp}");
            let (status, _) = req(
                app.clone(),
                axum::http::Method::POST.as_str(),
                &format!("/api/sessions/{key}/message"),
                Some(r#"{"message":"hi"}"#.to_string()),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{reference} 的 message 必须放行");
        }
    }

    #[tokio::test]
    async fn close_and_archive_stay_available_for_out_of_scope_session() {
        // archive 写的是 SEBAS_ARCHIVE_PATH（进程全局）：与 archive.rs 测试共用
        // 其串行锁并重定向到一次性文件，绝不落真实 ~/.sebas。锁跨 await 是刻意
        // 的——整个异步测试体都要串行。
        let _archive_guard = crate::archive::test_env_lock();
        let archive_dir = tempfile::tempdir().unwrap();
        let prev_archive = std::env::var("SEBAS_ARCHIVE_PATH").ok();
        // SAFETY: archive::test_env_lock 保证进程内 archive 测试串行。
        unsafe {
            std::env::set_var(
                "SEBAS_ARCHIVE_PATH",
                archive_dir.path().join("archive.json"),
            );
        }

        let t = two_trees();
        let (app, backend) = app_and_backend_with_workspace_root(t.allowed.path().to_path_buf());
        let outside = crate::fs::canonicalize_plain(t.outside.path()).expect("canonicalize");

        // close 放行（围栏不锁垃圾）。
        backend
            .set_sessions(vec![local_session("oos", Some(outside.clone()))])
            .await;
        let key = enc_key("oos");
        let (status, resp) = req(
            app.clone(),
            axum::http::Method::POST.as_str(),
            &format!("/api/sessions/{key}/close"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");

        // archive 放行（close 已移除会话，重新种子）。
        backend
            .set_sessions(vec![local_session("oos2", Some(outside))])
            .await;
        let key2 = enc_key("oos2");
        let (status, resp) = req(
            app,
            axum::http::Method::POST.as_str(),
            &format!("/api/sessions/{key2}/archive"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");

        // SAFETY: 同上（锁仍持有）。
        unsafe {
            match prev_archive {
                Some(p) => std::env::set_var("SEBAS_ARCHIVE_PATH", p),
                None => std::env::remove_var("SEBAS_ARCHIVE_PATH"),
            }
        }
    }

    #[tokio::test]
    async fn create_with_out_of_scope_project_id_is_rejected_without_spawning() {
        with_registry_env(|| async {
            let t = two_trees();
            let (app, backend) =
                app_and_backend_with_workspace_root(t.allowed.path().to_path_buf());
            // 越界历史项目（升级前遗留）直接落注册表。
            let legacy = crate::projects::add_on(
                crate::projects::LOCAL_NODE_ID,
                t.outside.path().to_str().unwrap(),
            )
            .unwrap();
            let body = serde_json::json!({ "project_id": legacy.id, "agent": "claude" });
            let (status, resp) =
                req(app.clone(), "POST", "/api/sessions", Some(body.to_string())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
            assert!(
                resp["error"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("超出允许范围"),
                "{resp}"
            );
            assert!(
                backend.snapshot().await.is_empty(),
                "携越界项目的 create 不得产生任何会话"
            );

            // 反例：界内项目照常创建 0-turn 占位。
            let dir = t.allowed.path().join("in-scope");
            std::fs::create_dir_all(&dir).unwrap();
            let inside =
                crate::projects::add_on(crate::projects::LOCAL_NODE_ID, dir.to_str().unwrap())
                    .unwrap();
            let body = serde_json::json!({ "project_id": inside.id, "agent": "claude" });
            let (status, resp) = req(app, "POST", "/api/sessions", Some(body.to_string())).await;
            assert_eq!(status, StatusCode::CREATED, "{resp}");
        })
        .await;
    }

    #[tokio::test]
    async fn branch_probe_reports_out_of_scope_project_as_inaccessible() {
        with_registry_env(|| async {
            let t = two_trees();
            let app = app_with_workspace_root(t.allowed.path().to_path_buf());
            // 界内 git 项目（对照：可达 + 分支可读）。
            let inside = t.allowed.path().join("repo");
            std::fs::create_dir_all(inside.join(".git")).unwrap();
            std::fs::write(inside.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            let in_scope =
                crate::projects::add_on(crate::projects::LOCAL_NODE_ID, inside.to_str().unwrap())
                    .unwrap();
            // 越界遗留项目：目录真实存在，但探测必须按不可达处理（不暴露存在性）。
            let out_scope = crate::projects::add_on(
                crate::projects::LOCAL_NODE_ID,
                t.outside.path().to_str().unwrap(),
            )
            .unwrap();

            let uri = format!("/api/projects/{}/branch", encode(&out_scope.id));
            let (status, body) = req(app.clone(), "GET", &uri, None).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["accessible"], false, "越界项目按不可达处理: {body}");
            assert_eq!(body["branch"], serde_json::Value::Null, "{body}");

            let uri = format!("/api/projects/{}/branch", encode(&in_scope.id));
            let (status, body) = req(app, "GET", &uri, None).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["accessible"], true, "{body}");
            assert_eq!(body["branch"], "main", "{body}");
        })
        .await;
    }

    /// 两个互不为邻的 tempdir 树：`allowed/`（有子目录）与 `outside/`。
    struct Trees {
        allowed: tempfile::TempDir,
        outside: tempfile::TempDir,
    }

    fn two_trees() -> Trees {
        let allowed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(allowed.path().join("sub")).unwrap();
        Trees { allowed, outside }
    }

    #[tokio::test]
    async fn browse_dirs_rejects_root_outside_workspace_root() {
        let t = two_trees();
        let app = app_with_workspace_root(t.allowed.path().to_path_buf());
        let uri = format!(
            "/api/fs/browse-dirs?root={}",
            urlencoding_encode(t.outside.path().to_str().unwrap())
        );
        let (status, body) = req(app, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let msg = body["error"].as_str().unwrap_or_default();
        assert!(msg.contains("超出允许范围"), "got: {msg}");
        assert!(
            !msg.contains(t.outside.path().to_str().unwrap()),
            "resolved path must not leak: {msg}"
        );
    }

    #[tokio::test]
    async fn browse_dirs_accepts_root_inside_workspace_root() {
        let t = two_trees();
        let app = app_with_workspace_root(t.allowed.path().to_path_buf());
        let uri = format!(
            "/api/fs/browse-dirs?root={}",
            urlencoding_encode(t.allowed.path().to_str().unwrap())
        );
        let (status, body) = req(app, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<&str> = body["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert!(names.contains(&"sub"), "entries: {body}");
    }

    /// spec「browse-dirs defaults to the workspace root」：无 root 参数的
    /// 浏览起点就是 workspace root，回显其 canonical 形。
    #[tokio::test]
    async fn browse_dirs_without_root_starts_at_workspace_root() {
        let t = two_trees();
        let app = app_with_workspace_root(t.allowed.path().to_path_buf());
        let (status, body) = req(app, "GET", "/api/fs/browse-dirs", None).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(
            body["path"].as_str(),
            Some(
                dunce::simplified(t.allowed.path())
                    .to_string_lossy()
                    .as_ref()
            )
        );
    }

    fn urlencoding_encode(s: &str) -> String {
        // 最小 percent-encode：只编码查询串里非法的字符（测试路径均为
        // tempdir 生成的安全 ASCII）。
        s.replace(' ', "%20")
    }

    /// 就绪日志的引导 URL：通配 bind 呈现为 127.0.0.1（浏览器不可点
    /// 0.0.0.0），loopback / 具体网卡地址原样。
    #[test]
    fn access_url_maps_unspecified_bind_to_loopback() {
        use std::net::{Ipv4Addr, SocketAddr, SocketAddrV6};
        assert_eq!(
            access_url(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 9877))),
            "http://127.0.0.1:9877"
        );
        assert_eq!(
            access_url(SocketAddr::from((Ipv4Addr::LOCALHOST, 9877))),
            "http://127.0.0.1:9877"
        );
        assert_eq!(
            access_url(SocketAddr::from((Ipv4Addr::new(192, 168, 1, 10), 9877))),
            "http://192.168.1.10:9877"
        );
        assert_eq!(
            access_url(SocketAddr::V6(SocketAddrV6::new(
                std::net::Ipv6Addr::UNSPECIFIED,
                9877,
                0,
                0
            ))),
            "http://127.0.0.1:9877"
        );
    }
}

#[cfg(all(test, unix))]
mod system_dir_denylist_tests {
    //! add-system-dir-denylist 2.1/2.3 路由级验收：注册执法在 containment
    //! 之后追加名单判定；browse-dirs 树内隐藏名单子目录。unix 门控——名单
    //! 命中分支依赖真实系统目录（`/` 与其子目录），Windows 侧的名单语义由
    //! fs.rs 的 #[cfg(windows)] 单测覆盖。
    use super::*;
    use crate::models::RouterInfo;
    use crate::session_backend::FakeBackend;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use http_body_util::BodyExt;
    use sebas_feishu::cards::CardConfig;
    use serde_json::Value;
    use std::net::{IpAddr, SocketAddr};
    use tower::ServiceExt;

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 12345)
    }

    fn app_with_workspace_root(root: std::path::PathBuf) -> Router {
        build_router_with_workspace_root(
            Arc::new(FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
            Arc::new(crate::agent_kinds::ConfigAgentKindProvider::new(Vec::new())),
            Arc::new(AuthHandle::disabled()),
            root,
        )
    }

    async fn req(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<String>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()));
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let req = builder.body(Body::from(body.unwrap_or_default())).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, v)
    }

    /// 与 workspace_root_tests 同款：注册走文件注册表降级路径，env 进程级，
    /// 共用同一把串行锁，用完恢复。
    #[allow(clippy::await_holding_lock)]
    async fn with_registry_env<F, R>(f: F) -> R
    where
        F: AsyncFnOnce() -> R,
    {
        let _g = crate::projects::test_env_lock();
        let prev = std::env::var("SEBAS_PROJECTS_PATH").ok();
        let registry = tempfile::tempdir().unwrap();
        // SAFETY: test_env_lock 保证进程内注册表相关测试串行。
        unsafe {
            std::env::set_var("SEBAS_PROJECTS_PATH", registry.path().join("p.json"));
        }
        let r = f().await;
        // SAFETY: 同上。
        unsafe {
            match prev {
                Some(p) => std::env::set_var("SEBAS_PROJECTS_PATH", p),
                None => std::env::remove_var("SEBAS_PROJECTS_PATH"),
            }
        }
        r
    }

    /// 2.3 主场景：workspace root=`/` 时——名单目录精确命中 400 点名入参
    /// （不含服务端解析形），根内普通目录照常 201。tempdir 都在 /tmp 之下，
    /// 按「子树放行」语义天然是合法注册位。
    #[tokio::test]
    async fn project_add_rejects_denylisted_dir_but_accepts_subtree() {
        with_registry_env(|| async {
            let app = app_with_workspace_root(std::path::PathBuf::from("/"));
            let body = serde_json::json!({ "path": "/usr" });
            let (status, resp) =
                req(app.clone(), "POST", "/api/projects", Some(body.to_string())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "resp: {resp}");
            let msg = resp["error"].as_str().unwrap_or_default();
            assert!(msg.contains("系统目录不可注册"), "got: {msg}");
            assert!(msg.contains("/usr"), "must name the caller input: {msg}");

            let legit = tempfile::tempdir().unwrap();
            let body = serde_json::json!({ "path": legit.path().to_str().unwrap() });
            let (status, _) = req(app, "POST", "/api/projects", Some(body.to_string())).await;
            assert_eq!(
                status,
                StatusCode::CREATED,
                "subtree of /tmp registers fine"
            );
        })
        .await;
    }

    /// 2.3 判定先行次序：root 收窄到普通 tempdir 后，root 外的系统目录
    /// （/usr）先撞 containment——返回越界文案而非名单文案。
    #[tokio::test]
    async fn project_add_containment_precedes_denylist() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("proj")).unwrap();
        let app = app_with_workspace_root(t.path().to_path_buf());
        let body = serde_json::json!({ "path": "/usr" });
        let (status, resp) = req(app, "POST", "/api/projects", Some(body.to_string())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let msg = resp["error"].as_str().unwrap_or_default();
        assert!(
            msg.contains("超出允许范围"),
            "containment first, got: {msg}"
        );
    }

    /// 2.1/1.3 联动：browse-dirs 树内隐藏——root=`/` 时顶层条目不含名单
    /// 目录（usr、etc、bin…），普通目录照旧；不误伤 /tmp 之下的 tempdir。
    #[tokio::test]
    async fn browse_dirs_hides_denylisted_top_level_entries() {
        let app = app_with_workspace_root(std::path::PathBuf::from("/"));
        let (status, body) = req(app, "GET", "/api/fs/browse-dirs", None).await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<&str> = body["entries"]
            .as_array()
            .map(|a| a.iter().filter_map(|e| e["name"].as_str()).collect())
            .unwrap_or_default();
        for deny in ["usr", "etc", "bin", "var", "proc", "tmp"] {
            assert!(
                !names.contains(&deny),
                "denylisted top-level dir must be hidden: {names:?}"
            );
        }
    }

    /// 2.1 遍历/别名形（spec「alias and traversal resolve before the
    /// comparison」API 级）：`..` 段、双斜杠别名与 workspace root 自身都要被
    /// 名单拦下，且文案点名**原始入参**——`/tmp/../etc` 原样出现，而非服务端
    /// 解析形替换后的文案。
    #[tokio::test]
    async fn project_add_traversal_and_root_forms_rejected_naming_raw_input() {
        with_registry_env(|| async {
            let app = app_with_workspace_root(std::path::PathBuf::from("/"));
            for raw in ["/tmp/../etc", "/tmp/..", "//usr", "/"] {
                let body = serde_json::json!({ "path": raw });
                let (status, resp) =
                    req(app.clone(), "POST", "/api/projects", Some(body.to_string())).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "raw={raw} resp: {resp}");
                let msg = resp["error"].as_str().unwrap_or_default();
                assert!(msg.contains("系统目录不可注册"), "raw={raw} got: {msg}");
                assert!(msg.contains(raw), "must name the raw input {raw}: {msg}");
            }
        })
        .await;
    }

    /// 2.1 副作用（spec「no project SHALL be registered」）：名单 400 之外
    /// 列表必须保持空——拒绝路径不得在任何降级分支落注册记录。
    #[tokio::test]
    async fn project_add_denylist_rejection_creates_no_project() {
        with_registry_env(|| async {
            let app = app_with_workspace_root(std::path::PathBuf::from("/"));
            let body = serde_json::json!({ "path": "/usr" });
            let (status, _) =
                req(app.clone(), "POST", "/api/projects", Some(body.to_string())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            let (_, list) = req(app, "GET", "/api/projects", None).await;
            let projects = list["projects"].as_array().expect("projects array");
            assert!(
                projects.is_empty(),
                "rejected registration must leave no project: {list}"
            );
        })
        .await;
    }

    /// 1.x round-trip 契约在「起点自身在名单上」的退化形态（root=`/`）下仍
    /// 成立：回显的 `/` 原样回传必须 200——过滤只作用于子目录条目，不改写
    /// 回显语义。
    #[tokio::test]
    async fn browse_dirs_round_trip_at_denylisted_root_still_works() {
        let app = app_with_workspace_root(std::path::PathBuf::from("/"));
        let (status, first) = req(app.clone(), "GET", "/api/fs/browse-dirs", None).await;
        assert_eq!(status, StatusCode::OK);
        let echo = first["path"].as_str().expect("echo path must be present");
        assert_eq!(echo, "/");
        let (status2, _) = req(app, "GET", "/api/fs/browse-dirs?path=/", None).await;
        assert_eq!(status2, StatusCode::OK, "echoed root must round-trip");
    }
}

#[cfg(test)]
mod restore_route_tests {
    //! 归档恢复的路由级验收（fix-webui-qa-defects 2.2，design D1）：
    //! 重建成功后才消费归档条目；重建失败（core 不可达）时 archive.json
    //! 原样保留——「消费归档 ⇄ 重建会话」同事务语义。
    use super::*;
    use crate::archive::test_env_lock;
    use crate::models::RouterInfo;
    use crate::session_backend::FakeBackend;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use http_body_util::BodyExt;
    use sebas_dispatch::TurnEntry;
    use sebas_feishu::cards::CardConfig;
    use serde_json::Value;
    use std::net::{IpAddr, SocketAddr};
    use tower::ServiceExt;

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 12345)
    }

    fn app(backend: Arc<FakeBackend>) -> Router {
        build_router_with_auth(
            backend,
            RouterInfo::default(),
            CardConfig::default(),
            None,
            Arc::new(crate::agent_kinds::ConfigAgentKindProvider::new(Vec::new())),
            30,
            Arc::new(AuthHandle::disabled()),
        )
    }

    async fn req(app: Router, method: &str, uri: &str) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:12345")
            .extension(ConnectInfo(test_addr()));
        let req = builder.body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, v)
    }

    /// archive env 的进程级串行临界区（与 archive.rs 测试共用一把锁）。
    /// 返回 (归档文件路径, 归档条目 key, URL 段)。条目 key 是 axum Path
    /// 解码后的原形（含 NUL）——与生产 archive handler 的存储形一致；URL
    /// 段是它的百分位编码。持锁跨 await 是刻意的——整个异步测试体都要串行。
    #[allow(clippy::await_holding_lock)]
    fn seed_archive(
        dir: &std::path::Path,
        session_id: Option<String>,
    ) -> (std::path::PathBuf, String, String) {
        // SAFETY: test_env_lock 保证进程内归档相关测试串行。
        unsafe {
            std::env::set_var("SEBAS_ARCHIVE_PATH", dir.join("archive.json"));
        }
        let raw = "web\0web-restore-1";
        // 会话键编码走唯一实现（add-domain-layer 2.3）。
        let url_segment = sebas_channels::key::encode_channel_key("web", "web-restore-1");
        crate::archive::archive_session(
            raw,
            "/proj",
            "archived label",
            session_id,
            sebas_dispatch::SessionIdentity::default(),
            30,
            vec![TurnEntry::prompt(0, "hello"), TurnEntry::markdown(1, "world")],
            None,
            None,
        )
        .expect("seed archive entry");
        (dir.join("archive.json"), raw.to_string(), url_segment)
    }

    #[allow(clippy::await_holding_lock)]
    fn clear_archive_env() {
        // SAFETY: test_env_lock 保证进程内归档相关测试串行。
        unsafe {
            std::env::remove_var("SEBAS_ARCHIVE_PATH");
        }
    }

    fn archive_snapshot(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    /// 主契约：重建成功（fake 可达）→ 200、归档条目被消费、重建请求确实
    /// 带着原 session_id 与完整转写到达了 backend 缝。
    #[tokio::test]
    async fn restore_rebuilds_first_and_only_then_consumes_the_archive() {
        let _lock = test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let (_path, _raw_key, url) = seed_archive(dir.path(), Some("old-sid".into()));

        let backend = Arc::new(FakeBackend::new());
        let (status, resp) = req(app(backend.clone()), "POST", &format!("/api/sessions/{url}/restore")).await;
        assert_eq!(status, StatusCode::OK, "resp: {resp}");
        assert_eq!(resp["status"], "restored");

        // 归档条目已被消费（History 清空）。
        assert!(
            crate::archive::list().is_empty(),
            "the archive entry must be consumed after a successful rebuild"
        );

        // 重建请求到达缝上：原 key、原 session_id、项目、完整转写。
        let restores = backend.restores().await;
        assert_eq!(restores.len(), 1, "exactly one rebuild call: {restores:?}");
        let (key, session_id, project_dir, entries, _identity, _label, _preview) = &restores[0];
        assert_eq!(key.reference, "web-restore-1");
        assert_eq!(session_id.as_deref(), Some("old-sid"));
        assert_eq!(project_dir.as_deref(), Some("/proj"));
        assert_eq!(*entries, 2, "the full transcript rides the rebuild");
        clear_archive_env();
    }

    /// 失败路径：core 不可达 → 503，且 archive.json 一个字节都没动。
    #[tokio::test]
    async fn failed_rebuild_leaves_the_archive_untouched() {
        let _lock = test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let (path, raw_key, url) = seed_archive(dir.path(), None);
        let before = archive_snapshot(&path);

        let backend = Arc::new(FakeBackend::new());
        backend.set_reachable(false, "core down");
        let (status, resp) =
            req(app(backend.clone()), "POST", &format!("/api/sessions/{url}/restore")).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "resp: {resp}");

        // 归档原样保留：文件未动、条目仍在（数据没有可逆丢失）。
        assert_eq!(archive_snapshot(&path), before, "archive.json must be byte-identical");
        assert!(
            crate::archive::entry(raw_key.as_str()).is_some(),
            "the archive entry must survive a failed rebuild"
        );
        assert!(backend.restores().await.is_empty());
        clear_archive_env();
    }

    /// 未知 key 照旧 404（无条目可恢复，不触碰任何状态）。
    #[tokio::test]
    async fn restore_unknown_key_is_a_not_found() {
        let _lock = test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: test_env_lock。
        unsafe {
            std::env::set_var("SEBAS_ARCHIVE_PATH", dir.path().join("archive.json"));
        }
        // 会话键编码走唯一实现（add-domain-layer 2.3）。
        let encoded = sebas_channels::key::encode_channel_key("web", "web-ghost");
        let backend = Arc::new(FakeBackend::new());
        let (status, _) = req(app(backend), "POST", &format!("/api/sessions/{encoded}/restore")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // SAFETY: test_env_lock。
        unsafe {
            std::env::remove_var("SEBAS_ARCHIVE_PATH");
        }
    }
}
