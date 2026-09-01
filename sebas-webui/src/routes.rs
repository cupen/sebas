//! Route handlers for the WebUI dashboard.
//!
//! Every session read and mutation flows through the `SessionBackend` seam
//! (openspec/changes/add-core-session-channel): the routes render whatever
//! the backend reports, and typed rejections map onto the pre-existing HTTP
//! status codes. When the backend is unreachable the pages render the cause
//! and the mutations return 503 — no control ever reports success falsely.

use crate::models::{CardConfigInfo, DashboardData, SessionRow, SessionStatus};
use crate::server::WebUiState;
use crate::session_backend::{Reachability, SessionRejection};
use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Json};
use sebas_feishu::events::SessionKey;
use sebas_router::{SessionInfo, TurnEntry};
use serde::Deserialize;

/// Dashboard overview: session counts, recent sessions, uptime, active session.
pub async fn dashboard(State(state): State<WebUiState>) -> impl IntoResponse {
    let sessions = state.backend.snapshot().await;
    let active_key = state.backend.focused().await;

    let (rows, active, dormant, spawning) = build_session_rows(&sessions, active_key.as_ref());

    let data = DashboardData {
        active_count: active,
        dormant_count: dormant,
        spawning_count: spawning,
        total_sessions: active + dormant + spawning,
        uptime: format_uptime(state.started_at.elapsed()),
        recent_sessions: rows,
        active_session: active_key
            .as_ref()
            .and_then(|k| find_info(&sessions, k))
            .map(|info| session_summary(info)),
        active_session_key: active_key.as_ref().map(encode_session_key),
    };

    render_template(&state, "index.html", "dashboard", &data).await
}

/// Full session list (full page with action bar).
pub async fn session_list(State(state): State<WebUiState>) -> impl IntoResponse {
    let sessions = state.backend.snapshot().await;
    let active_key = state.backend.focused().await;
    let (rows, active, dormant, spawning) = build_session_rows(&sessions, active_key.as_ref());
    let unreachable = unreachable_cause(&state).await;

    let data = serde_json::json!({
        "recent_sessions": rows,
        "active_count": active,
        "dormant_count": dormant,
        "spawning_count": spawning,
        "total_sessions": active + dormant + spawning,
        "active_session_key": active_key.as_ref().map(encode_session_key),
        "core_unreachable_cause": unreachable,
    });
    render_template(&state, "sessions.html", "sessions", &data).await
}

/// htmx partial: just the table body + counts. Used by SSE-driven refresh
/// so the list updates live without a full page reload.
pub async fn session_list_partial(State(state): State<WebUiState>) -> impl IntoResponse {
    let sessions = state.backend.snapshot().await;
    let active_key = state.backend.focused().await;
    let (rows, active, dormant, spawning) = build_session_rows(&sessions, active_key.as_ref());
    let unreachable = unreachable_cause(&state).await;

    let data = serde_json::json!({
        "recent_sessions": rows,
        "active_count": active,
        "dormant_count": dormant,
        "spawning_count": spawning,
        "total_sessions": active + dormant + spawning,
        "active_session_key": active_key.as_ref().map(encode_session_key),
        "core_unreachable_cause": unreachable,
    });
    render_template(&state, "sessions_partial.html", "sessions", &data).await
}

/// Session detail: transcript content and session facts. Visiting this page
/// also marks the session as the focused WebUI session.
pub async fn session_detail(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return Html("Invalid session key".to_string()),
    };

    let sessions = state.backend.snapshot().await;
    let Some(info) = find_info(&sessions, &session_key) else {
        return Html("Session not found".to_string());
    };

    // Visiting the detail page focuses this session in the dashboard.
    state.backend.set_focus(Some(session_key.clone())).await;

    // Transcript: full fetch (from position 0). The page is a full render —
    // incremental fetch matters for the timeline, not here.
    let body_view: Vec<crate::models::CardElementView> =
        match state.backend.turns(session_key.clone(), 0).await {
            Ok(entries) => entries.iter().map(turn_entry_to_view).collect(),
            Err(_) => Vec::new(),
        };

    let derived = SessionStatus::derive(&info.status, info.phase.as_deref().unwrap_or(""));

    let unreachable = unreachable_cause(&state).await;
    let data = serde_json::json!({
        "chat_id": info.chat_id,
        "thread_id": info.thread_id,
        "session_id": info.session_id,
        "status": info.status,
        "status_label": derived.label(),
        "status_slug": derived.slug(),
        "status_glyph": derived.glyph(),
        "user_prompt": info.user_prompt,
        "body": body_view,
        "last_active": format_relative_time(info.last_active_unix),
        "encoded_key": key,
        "project_dir": info.project_dir,
        "core_unreachable_cause": unreachable,
    });

    render_template(&state, "session_detail.html", "sessions", &data).await
}

/// Settings page: card config and basic gateway info.
pub async fn settings(State(state): State<WebUiState>) -> impl IntoResponse {
    let card_cfg = &state.card_config;
    let card_config_info = CardConfigInfo {
        theme_color: card_cfg.theme_color.clone(),
        fold_long_output: card_cfg.fold_long_output,
        thinking_display: format!("{:?}", card_cfg.thinking),
        max_user_text_chars: card_cfg.max_user_text_chars,
        max_tool_output_chars: card_cfg.max_tool_output_chars,
    };

    let data = serde_json::json!({
        "card_config": card_config_info,
        "gateway": state.gateway,
    });

    render_template(&state, "settings.html", "settings", &data).await
}

/// Gateway page: 实时数据（providers/aliases/stats 经 gateway admin API 拉取，
/// Task 6.2）。gateway 不可达/未启动 → 降级卡片（保底显示启动快照）。
pub async fn gateway_page(State(state): State<WebUiState>) -> impl IntoResponse {
    let listen = state.gateway.listen.clone().unwrap_or_default();
    let client = crate::gateway_client::GatewayClient::new(&listen);
    let (providers, aliases, stats) = if listen.is_empty() {
        (Err(crate::gateway_client::GatewayClientError("gateway 未启动".into())), Err(crate::gateway_client::GatewayClientError("gateway 未启动".into())), Err(crate::gateway_client::GatewayClientError("gateway 未启动".into())))
    } else {
        tokio::join!(client.providers(), client.model_aliases(), client.stats())
    };
    let degraded_reason = providers.as_ref().err().map(|e| e.0.clone());
    let live = degraded_reason.is_none();
    let data = serde_json::json!({
        "gateway": state.gateway,
        // 降级时 live=false：模板渲染降级提示 + 启动快照。
        "live": live,
        "degraded_reason": degraded_reason,
        "providers": providers.as_ref().ok().and_then(|v| v.get("providers").cloned()).unwrap_or(serde_json::Value::Null),
        "model_aliases": aliases.ok().and_then(|v| v.get("model_aliases").cloned()).unwrap_or(serde_json::Value::Null),
        "stats": stats.ok().unwrap_or(serde_json::Value::Null),
        "has_secret": client.has_secret(),
    });
    render_template(&state, "gateway.html", "gateway", &data).await
}

// ---- gateway BFF mutation 路由（Task 6.3）：全部转发 admin API ----

fn gateway_client_of(state: &WebUiState) -> crate::gateway_client::GatewayClient {
    let listen = state.gateway.listen.clone().unwrap_or_default();
    crate::gateway_client::GatewayClient::new(&listen)
}

/// 无 listen 或无 secret → 503（mutation 面不可用；只读 loopback 仍可看页）。
fn mutation_available(client: &crate::gateway_client::GatewayClient, state: &WebUiState) -> bool {
    state.gateway.listen.is_some() && client.has_secret()
}

pub async fn gateway_api_provider_create(
    State(state): State<WebUiState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let client = gateway_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.create_provider(&body).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn gateway_api_provider_update(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let client = gateway_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.update_provider(&name, &body).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn gateway_api_provider_delete(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> axum::response::Response {
    let client = gateway_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.delete_provider(&name).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn gateway_api_provider_probe(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> axum::response::Response {
    let client = gateway_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.probe_provider(&name, true).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn gateway_api_alias_create(
    State(state): State<WebUiState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let client = gateway_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.create_alias(&body).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn gateway_api_alias_update(
    State(state): State<WebUiState>,
    Path(alias): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let client = gateway_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.update_alias(&alias, &body).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn gateway_api_alias_delete(
    State(state): State<WebUiState>,
    Path(alias): Path<String>,
) -> axum::response::Response {
    let client = gateway_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.delete_alias(&alias).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn gateway_api_reload(State(state): State<WebUiState>) -> axum::response::Response {
    let client = gateway_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.reload().await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn gateway_mutation_guard(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::Method;
    // 只放行变更语义（POST/PUT/DELETE）；GET/HEAD 等读方法 405。
    if !matches!(req.method(), &Method::POST | &Method::PUT | &Method::DELETE) {
        return (axum::http::StatusCode::METHOD_NOT_ALLOWED, "mutation only").into_response();
    }
    let origin_ok = req
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(|o| {
            o.is_empty()
                || o.strip_prefix("http://")
                    .map(|rest| {
                        let host = rest.split(':').next().unwrap_or(rest);
                        host == "127.0.0.1" || host == "localhost" || host == "::1"
                    })
                    .unwrap_or(false)
        })
        .unwrap_or(true); // 无 origin（CLI/curl）放行——gateway 侧另有 bearer 鉴权
    if !origin_ok {
        return (axum::http::StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    next.run(req).await
}

fn err_json(msg: String) -> axum::response::Response {
    (
        axum::http::StatusCode::BAD_GATEWAY,
        axum::Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn err_503_no_secret() -> axum::response::Response {
    (
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({"error": "gateway admin mutation 不可用（未配置 SEBAS_CONTROL_SECRET 或 gateway 未启动）"})),
    )
        .into_response()
}

/// Map a typed backend rejection onto the pre-existing HTTP status codes:
/// unknown key → 404, unusable project dir → 400, capacity/unavailable → 503.
fn rejection_response(rej: SessionRejection) -> axum::response::Response {
    let status = match &rej {
        SessionRejection::UnknownSession { .. } => axum::http::StatusCode::NOT_FOUND,
        SessionRejection::UnusableProjectDir => axum::http::StatusCode::BAD_REQUEST,
        SessionRejection::Capacity { .. } | SessionRejection::Unavailable { .. } => {
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        }
    };
    (
        status,
        Json(serde_json::json!({ "error": rej.to_string() })),
    )
        .into_response()
}

/// The reachability cause when the backend cannot reach the core, else `None`.
async fn unreachable_cause(state: &WebUiState) -> Option<String> {
    match state.backend.reachability().await {
        Reachability::Reachable => None,
        Reachability::Unreachable { cause } => Some(cause),
    }
}

/// About page: version info and system status.
pub async fn about(State(state): State<WebUiState>) -> impl IntoResponse {
    let uptime = state.started_at.elapsed();
    let data = serde_json::json!({
        "uptime": format_uptime(uptime),
        "version": env!("CARGO_PKG_VERSION"),
        "rustc_version": env!("CARGO_PKG_RUST_VERSION"),
        "gateway_listen": state.gateway.listen,
        "provider_count": state.gateway.provider_count,
    });
    render_template(&state, "about.html", "about", &data).await
}

/// Health check endpoint.
pub async fn health() -> &'static str {
    "ok\n"
}

// ---- API endpoints ----

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub prompt: String,
}

pub async fn api_create_session(
    State(state): State<WebUiState>,
    Form(req): Form<CreateSessionRequest>,
) -> axum::response::Response {
    match state.backend.spawn(req.prompt, None).await {
        Ok(key) => {
            let encoded = encode_session_key(&key);
            state.backend.set_focus(Some(key)).await;
            // Session is still spawning; client redirects to detail page.
            (
                axum::http::StatusCode::CREATED,
                Json(serde_json::json!({ "key": encoded })),
            )
                .into_response()
        }
        Err(rej) => rejection_response(rej),
    }
}

#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub message: String,
}

pub async fn api_send_message(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
    Form(req): Form<SendMessageRequest>,
) -> axum::response::Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid session key" })),
            )
                .into_response();
        }
    };
    match state.backend.message(session_key, req.message).await {
        Ok(()) => (
            axum::http::StatusCode::OK,
            Json(serde_json::json!({"status": "ok"})),
        )
            .into_response(),
        Err(rej) => rejection_response(rej),
    }
}

/// Close (kill) a session. Returns 200 + the new active session key (or
/// `null`) on success; 404 if the key didn't map to anything.
pub async fn api_close_session(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
) -> axum::response::Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session key"})),
            )
                .into_response();
        }
    };

    match state.backend.close(session_key).await {
        Ok(()) => {
            let active = state.backend.focused().await;
            (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({
                    "status": "closed",
                    "active_session_key": active.as_ref().map(encode_session_key),
                })),
            )
                .into_response()
        }
        Err(rej) => rejection_response(rej),
    }
}

/// Switch the focused WebUI session. Returns the redirect URL the client
/// should navigate to (the detail page for `key`). 404 if the session
/// doesn't exist so the client doesn't navigate to a dead page.
pub async fn api_switch_session(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid session key"})),
            );
        }
    };

    let sessions = state.backend.snapshot().await;
    if find_info(&sessions, &session_key).is_none() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Session not found"})),
        );
    }

    state.backend.set_focus(Some(session_key.clone())).await;
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "status": "switched",
            "redirect": format!("/sessions/{}", key),
            "active_session_key": key,
        })),
    )
}

// ---- Agent 项目工作台 handlers (webui/projects) ----

/// Agent 项目页：侧栏会话列表 + 主区 (focused session chat 或 empty state)。
pub async fn agent_page(State(state): State<WebUiState>) -> impl IntoResponse {
    let sessions = state.backend.snapshot().await;
    let active_key = state.backend.focused().await;
    let (rows, _, _, _) = build_session_rows(&sessions, active_key.as_ref());
    let active = active_key
        .as_ref()
        .and_then(|k| find_info(&sessions, k))
        .map(|info| session_agent_summary(info));
    let unreachable = unreachable_cause(&state).await;

    let data = serde_json::json!({
        "sessions": rows,
        "active_session": active,
        "active_key": active_key.as_ref().map(encode_session_key),
        "core_unreachable_cause": unreachable,
    });
    render_template(&state, "agent.html", "agent", &data).await
}

/// Agent 会话详情：focused session = 这条，侧栏高亮。
pub async fn agent_detail(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return Html("Invalid session key".to_string()),
    };
    let sessions = state.backend.snapshot().await;
    let Some(info) = find_info(&sessions, &session_key) else {
        return Html("Agent session not found".to_string());
    };
    // 聚焦到该 session。
    state.backend.set_focus(Some(session_key.clone())).await;

    let (rows, _, _, _) = build_session_rows(&sessions, Some(&session_key));
    let active = Some(session_agent_summary(info));
    let unreachable = unreachable_cause(&state).await;
    let data = serde_json::json!({
        "sessions": rows,
        "active_session": active,
        "active_key": encode_session_key(&session_key),
        "core_unreachable_cause": unreachable,
    });
    render_template(&state, "agent.html", "agent", &data).await
}

/// Agent timeline 片段：HTMX `hx-get` 每 3s 轮询的增量更新。
pub async fn agent_timeline(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return Html("".to_string()),
    };
    let sessions = state.backend.snapshot().await;
    let Some(info) = find_info(&sessions, &session_key) else {
        return Html("".to_string());
    };
    let mut summary = session_agent_summary(info);
    if let Ok(entries) = state.backend.turns(session_key, 0).await {
        summary["body"] = serde_json::json!(entries.iter().map(turn_entry_to_view).collect::<Vec<_>>());
    }
    let data = serde_json::json!({ "active_session": summary });
    render_template(&state, "agent_timeline.html", "agent", &data).await
}

/// 创建项目 session：接受 git 仓库路径，展开 `~`，校验存在且为目录，
/// 以自动生成的 prompt 在该目录下 spawn 一个 agent 会话。
pub async fn api_create_project(
    State(state): State<WebUiState>,
    Form(req): Form<CreateProjectRequest>,
) -> axum::response::Response {
    let raw = req.path.trim().to_string();
    let expanded = expand_home_tilde(&raw);
    let project_dir = std::path::Path::new(&expanded);
    if !project_dir.exists() || !project_dir.is_dir() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("路径不存在或不是目录: {expanded}") })),
        )
            .into_response();
    }
    let prompt = format!("Work in {expanded} — understand the project structure and help the user with their tasks.");
    match state.backend.spawn(prompt, Some(expanded)).await {
        Ok(key) => {
            let encoded = encode_session_key(&key);
            state.backend.set_focus(Some(key)).await;
            (
                axum::http::StatusCode::CREATED,
                Json(serde_json::json!({ "key": encoded })),
            )
                .into_response()
        }
        Err(rej) => rejection_response(rej),
    }
}

/// 给 agent 会话发消息。
pub async fn api_agent_message(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
    Form(req): Form<SendMessageRequest>,
) -> Html<String> {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return Html("".to_string()),
    };
    if let Err(rej) = state
        .backend
        .message(session_key.clone(), req.message)
        .await
    {
        // 返回错误片段（HTMX swap 目标是 timeline 容器）——保持诚实：失败
        // 不渲染成功的时间线。
        return Html(format!(
            "<div class=\"alert alert-error\">{}</div>",
            rej
        ));
    }
    // 返回 timeline 片段以便 HTMX 立即刷新。
    let sessions = state.backend.snapshot().await;
    let Some(info) = find_info(&sessions, &session_key) else {
        return Html("".to_string());
    };
    let mut summary = session_agent_summary(info);
    if let Ok(entries) = state.backend.turns(session_key, 0).await {
        summary["body"] = serde_json::json!(entries.iter().map(turn_entry_to_view).collect::<Vec<_>>());
    }
    let data = serde_json::json!({ "active_session": summary });
    render_template(&state, "agent_timeline.html", "agent", &data).await
}

#[derive(Deserialize)]
pub struct CreateProjectRequest {
    pub path: String,
}

/// Agent 页 focused session 的完整渲染数据（含 prompt/body/phase_display）。
/// Body 由调用方按需填充（turns 拉取）。
fn session_agent_summary(info: &SessionInfo) -> serde_json::Value {
    let phase_display = emoji_to_display(info.phase.as_deref().unwrap_or("")).to_string();
    serde_json::json!({
        "chat_id": info.chat_id,
        "thread_id": info.thread_id,
        "session_id": info.session_id,
        "status": info.status,
        "phase": info.phase,
        "phase_display": phase_display,
        "prompt": info.user_prompt,
        "body": [],
        "project_dir": info.project_dir,
        "last_active": format_relative_time(info.last_active_unix),
        "encoded_key": encode_session_key(&SessionKey {
            chat_id: info.chat_id.clone(),
            thread_id: info.thread_id.clone(),
        }),
    })
}

/// 展开首字符 `~`（本地实现；webui crate 不依赖 sebas 的 expand_tilde）。
fn expand_home_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::Path::new(&home).join(rest).to_string_lossy().into();
    }
    p.to_string()
}

/// 把 Feishu reaction `status_emoji`（OnIt/DONE/CrossMark/Get）映射为
/// agent 模板期望的 phase_display 字符串（Working/Done/Failed/Waiting）。
fn emoji_to_display(emoji: &str) -> &'static str {
    match emoji {
        "OnIt" => "Working",
        "DONE" => "Done",
        "CrossMark" => "Failed",
        "Get" | "SEED" => "Waiting",
        _ => "Waiting",
    }
}

// ---- Helper functions ----

/// Find a session's `SessionInfo` in a snapshot by key identity.
fn find_info<'a>(sessions: &'a [SessionInfo], key: &SessionKey) -> Option<&'a SessionInfo> {
    sessions
        .iter()
        .find(|s| s.chat_id == key.chat_id && s.thread_id == key.thread_id)
}

/// Build `SessionRow`s from a backend snapshot, returning counts. Marks each
/// row with `is_active` so the template can render the active indicator.
fn build_session_rows(
    sessions: &[SessionInfo],
    active_key: Option<&SessionKey>,
) -> (Vec<SessionRow>, usize, usize, usize) {
    let mut active = 0usize;
    let mut dormant = 0usize;
    let mut spawning = 0usize;

    let mut rows: Vec<SessionRow> = sessions
        .iter()
        .map(|info| {
            let status: &'static str = match info.status.as_str() {
                "active" => {
                    active += 1;
                    "active"
                }
                "dormant" => {
                    dormant += 1;
                    "dormant"
                }
                _ => {
                    spawning += 1;
                    "spawning"
                }
            };
            let phase = info.phase.clone().unwrap_or_default();
            let is_active = active_key
                .map(|a| a.chat_id == info.chat_id && a.thread_id == info.thread_id)
                .unwrap_or(false);
            // Agent page 标签：project_dir 优先（📁）；否则用首条用户消息
            // 预览（💬），取前 80 字符。
            let prompt_preview = info.user_prompt.as_ref().and_then(|p| {
                let p = p.trim();
                if p.is_empty() {
                    None
                } else if p.chars().count() > 80 {
                    let short: String = p.chars().take(80).collect();
                    Some(format!("{short}…"))
                } else {
                    Some(p.to_string())
                }
            });
            let derived = SessionStatus::derive(status, &phase);
            SessionRow {
                encoded_key: encode_session_key(&SessionKey {
                    chat_id: info.chat_id.clone(),
                    thread_id: info.thread_id.clone(),
                }),
                chat_id: info.chat_id.clone(),
                thread_id: info.thread_id.clone(),
                session_id_short: info
                    .session_id
                    .as_deref()
                    .map(|s| crate::models::middle_truncate(s, 18)),
                session_id: info.session_id.clone(),
                status,
                status_label: derived.label(),
                status_slug: derived.slug(),
                status_glyph: derived.glyph(),
                last_active: format_relative_time(info.last_active_unix),
                is_active,
                project_dir: info.project_dir.clone(),
                prompt_preview,
            }
        })
        .collect();

    // Sort: active first, then by most-recent activity (string sort is
    // good enough since `last_active` is a fixed-width relative time).
    rows.sort_by(|a, b| {
        b.is_active
            .cmp(&a.is_active)
            .then_with(|| b.last_active.cmp(&a.last_active))
    });
    (rows, active, dormant, spawning)
}

/// Compact summary used by the dashboard's focused-session banner.
fn session_summary(info: &SessionInfo) -> serde_json::Value {
    let derived = SessionStatus::derive(&info.status, info.phase.as_deref().unwrap_or(""));
    serde_json::json!({
        "chat_id": info.chat_id,
        "thread_id": info.thread_id,
        "session_id": info.session_id,
        "status": info.status,
        "status_label": derived.label(),
        "status_slug": derived.slug(),
        "status_glyph": derived.glyph(),
        "encoded_key": encode_session_key(&SessionKey {
            chat_id: info.chat_id.clone(),
            thread_id: info.thread_id.clone(),
        }),
    })
}

/// Render a MiniJinja template with the given context, including the current
/// page and the WebUI-active session key (so the sidebar can render its
/// focused-session indicator on every page).
async fn render_template<T: serde::Serialize>(
    state: &WebUiState,
    template_name: &str,
    page: &str,
    context: &T,
) -> Html<String> {
    let tmpl = state
        .templates
        .get_template(template_name)
        .expect("template should exist");
    let mut map = serde_json::Map::new();
    map.insert("page".into(), serde_json::Value::String(page.into()));
    if let Some(obj) = serde_json::to_value(context)
        .ok()
        .and_then(|v| v.as_object().cloned())
    {
        map.extend(obj);
    }
    // Inject the active session key so the sidebar's `sidebar_active.html`
    // partial can render its focused-session card on every page.
    let active_key = state.backend.focused().await;
    map.insert(
        "active_session_key".into(),
        match active_key {
            Some(k) => serde_json::Value::String(encode_session_key(&k)),
            None => serde_json::Value::Null,
        },
    );
    let rendered = tmpl
        .render(minijinja::Value::from_serialize(&map))
        .unwrap_or_else(|e| format!("Template error: {e}"));
    Html(rendered)
}

/// Encode a SessionKey for use in URLs.
fn encode_session_key(key: &SessionKey) -> String {
    let raw = format!(
        "{}\0{}",
        key.chat_id,
        key.thread_id.as_deref().unwrap_or("")
    );
    urlencoding::encode(&raw).into_owned()
}

/// Decode a URL-encoded SessionKey.
fn decode_session_key(encoded: &str) -> Option<SessionKey> {
    let decoded = urlencoding::decode(encoded).ok()?;
    let (chat_id, thread_id) = decoded.split_once('\0')?;
    Some(SessionKey {
        chat_id: chat_id.to_string(),
        thread_id: if thread_id.is_empty() {
            None
        } else {
            Some(thread_id.to_string())
        },
    })
}

/// Map a transcript entry onto the card-element view model. The transcript
/// already carries rendered view shapes (`element_type` + `content`), so
/// this is a direct mapping.
fn turn_entry_to_view(entry: &TurnEntry) -> crate::models::CardElementView {
    crate::models::CardElementView {
        element_type: match entry.element_type.as_str() {
            "markdown" => "markdown",
            "thinking" => "thinking",
            _ => "other",
        },
        content: entry.content.clone(),
    }
}

/// Format a unix timestamp as a relative time string.
fn format_relative_time(unix_ts: i64) -> String {
    let diff = chrono::Utc::now().timestamp() - unix_ts;
    if diff < 60 {
        format!("{diff}s ago")
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
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
