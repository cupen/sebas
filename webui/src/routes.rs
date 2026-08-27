//! Route handlers for the WebUI dashboard.

use crate::models::{CardConfigInfo, CardElementView, DashboardData, SessionRow};
use crate::server::WebUiState;
use crate::sse::WebUiEvent;
use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Json};
use feishu::cards::CardElement;
use feishu::events::SessionKey;
use router::card_state::CardState;
use router::router::CloseOutcome;
use router::state::{Mapping, MappingState};
use serde::Deserialize;

/// Dashboard overview: session counts, recent sessions, uptime, active session.
pub async fn dashboard(State(state): State<WebUiState>) -> impl IntoResponse {
    let sessions = state.router.session_snapshot().await;
    let card_states = state.router.card_state_snapshot().await;
    let active_key = state.router.active_session_snapshot().await;
    let active_encoded = active_key.as_ref().map(encode_session_key);

    let (rows, active, dormant, spawning) =
        build_session_rows(&sessions, &card_states, active_key.as_ref());

    let data = DashboardData {
        active_count: active,
        dormant_count: dormant,
        spawning_count: spawning,
        total_sessions: active + dormant + spawning,
        uptime_seconds: state.started_at.elapsed().as_secs() as i64,
        recent_sessions: rows,
        active_session: active_key.as_ref().map(|k| session_summary(k, &sessions)),
        active_session_key: active_encoded,
    };

    render_template(&state, "index.html", "dashboard", &data).await
}

/// Full session list (full page with action bar).
pub async fn session_list(State(state): State<WebUiState>) -> impl IntoResponse {
    let sessions = state.router.session_snapshot().await;
    let card_states = state.router.card_state_snapshot().await;
    let active_key = state.router.active_session_snapshot().await;
    let (rows, active, dormant, spawning) =
        build_session_rows(&sessions, &card_states, active_key.as_ref());

    let data = serde_json::json!({
        "recent_sessions": rows,
        "active_count": active,
        "dormant_count": dormant,
        "spawning_count": spawning,
        "total_sessions": active + dormant + spawning,
        "active_session_key": active_key.as_ref().map(encode_session_key),
    });
    render_template(&state, "sessions.html", "sessions", &data).await
}

/// htmx partial: just the table body + counts. Used by SSE-driven refresh
/// so the list updates live without a full page reload.
pub async fn session_list_partial(State(state): State<WebUiState>) -> impl IntoResponse {
    let sessions = state.router.session_snapshot().await;
    let card_states = state.router.card_state_snapshot().await;
    let active_key = state.router.active_session_snapshot().await;
    let (rows, active, dormant, spawning) =
        build_session_rows(&sessions, &card_states, active_key.as_ref());

    let data = serde_json::json!({
        "recent_sessions": rows,
        "active_count": active,
        "dormant_count": dormant,
        "spawning_count": spawning,
        "total_sessions": active + dormant + spawning,
        "active_session_key": active_key.as_ref().map(encode_session_key),
    });
    render_template(&state, "sessions_partial.html", "sessions", &data).await
}

/// Session detail: card content, message_id, events. Visiting this page
/// also marks the session as the focused WebUI session.
pub async fn session_detail(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let session_key = decode_session_key(&key);

    let sessions = state.router.session_snapshot().await;
    let (mapping, raw_key) = match session_key {
        Some(ref sk) => match sessions
            .into_iter()
            .find(|(k, _)| k.chat_id == sk.chat_id && k.thread_id == sk.thread_id)
        {
            Some((k, m)) => (m, k),
            None => return Html("Session not found".to_string()),
        },
        None => return Html("Invalid session key".to_string()),
    };

    // Visiting the detail page focuses this session in the dashboard.
    state.router.web_set_active(raw_key.clone()).await;

    let card_states = state.router.card_state_snapshot().await;
    let encoded_key = encode_session_key(&raw_key);

    let (status, session_id) = match &mapping.state {
        MappingState::Active { session_id } => ("active", Some(session_id.clone())),
        MappingState::Dormant { session_id } => ("dormant", Some(session_id.clone())),
        MappingState::Spawning { .. } => ("spawning", None),
    };

    let (phase, user_prompt, body_view) = match &session_id {
        Some(sid) => card_states
            .get(sid)
            .map(|st| {
                let phase = st.status_emoji.clone();
                let body: Vec<CardElementView> = st.body.iter().map(card_element_to_view).collect();
                (phase, st.user_prompt.clone(), body)
            })
            .unwrap_or_default(),
        None => Default::default(),
    };

    let msg_id = if let Some(sid) = session_id.as_ref() {
        state.router.msgid_snapshot().await.get(sid).cloned()
    } else {
        None
    };

    let data = serde_json::json!({
        "chat_id": raw_key.chat_id,
        "thread_id": raw_key.thread_id,
        "session_id": session_id,
        "status": status,
        "phase": phase,
        "user_prompt": user_prompt,
        "body": body_view,
        "msg_id": msg_id,
        "last_active": format_relative_time(mapping.last_active_unix),
        "encoded_key": encoded_key,
    });

    render_template(&state, "session_detail.html", "sessions", &data).await
}

/// Settings page: card config and basic gateway info.
pub async fn settings(State(state): State<WebUiState>) -> impl IntoResponse {
    let card_cfg = state.router.card_config().await;
    let card_config_info = CardConfigInfo {
        theme_color: card_cfg.theme_color,
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

/// gateway mutation 守卫（Task 6.3，语义与 admin_mutation_guard 一致但
/// 不依赖 AdminState）：POST-only（405）+ loopback origin 检查（403）。
pub async fn gateway_mutation_guard(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::Method;
    // 只放行变更语义（POST/PUT/DELETE）；GET/HEAD 等读方法 405。
    if !matches!(
        req.method(),
        &Method::POST | &Method::PUT | &Method::DELETE
    ) {
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
) -> impl IntoResponse {
    let key = state.router.web_spawn(req.prompt, None).await;
    let encoded = encode_session_key(&key);
    state.router.web_set_active(key.clone()).await;
    let _ = state.event_tx.send(WebUiEvent::SessionCreated {
        session_id: encoded.clone(),
    });
    // Session is still spawning; client redirects to detail page.
    (
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "key": encoded })),
    )
}

#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub message: String,
}

pub async fn api_send_message(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
    Form(req): Form<SendMessageRequest>,
) -> impl IntoResponse {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid session key" })),
            );
        }
    };
    state
        .router
        .web_send_message(session_key, req.message)
        .await;
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({"status": "ok"})),
    )
}

/// Close (kill) a session. Returns 200 + the new active session key (or
/// `null`) on success; 404 if the key didn't map to anything.
pub async fn api_close_session(
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

    let outcome = state.router.web_close_session(session_key.clone()).await;
    match outcome {
        CloseOutcome::NotFound => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Session not found"})),
        ),
        CloseOutcome::Closed => {
            let active = state.router.active_session_snapshot().await;
            let _ = state.event_tx.send(WebUiEvent::SessionRemoved {
                session_id: key.clone(),
            });
            (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({
                    "status": "closed",
                    "active_session_key": active.as_ref().map(encode_session_key),
                })),
            )
        }
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

    let sessions = state.router.session_snapshot().await;
    if !sessions
        .iter()
        .any(|(k, _)| k.chat_id == session_key.chat_id && k.thread_id == session_key.thread_id)
    {
        return (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Session not found"})),
        );
    }

    state.router.web_set_active(session_key.clone()).await;
    let _ = state.event_tx.send(WebUiEvent::SessionUpdated {
        session_id: key.clone(),
        status: "active".into(),
    });
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
    let sessions = state.router.session_snapshot().await;
    let card_states = state.router.card_state_snapshot().await;
    let active_key = state.router.active_session_snapshot().await;
    let (rows, _, _, _) = build_session_rows(&sessions, &card_states, active_key.as_ref());
    let active = active_key.as_ref().map(|k| session_agent_summary(k, &sessions, &card_states));

    let data = serde_json::json!({
        "sessions": rows,
        "active_session": active,
        "active_key": active_key.as_ref().map(encode_session_key),
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
    let sessions = state.router.session_snapshot().await;
    let exists = sessions
        .iter()
        .any(|(k, _)| k.chat_id == session_key.chat_id && k.thread_id == session_key.thread_id);
    if !exists {
        return Html("Agent session not found".to_string());
    }
    // 聚焦到该 session。
    state.router.web_set_active(session_key.clone()).await;

    let card_states = state.router.card_state_snapshot().await;
    let active_key = Some(&session_key);
    let (rows, _, _, _) = build_session_rows(&sessions, &card_states, active_key.as_deref());
    let active = active_key.map(|k| session_agent_summary(k, &sessions, &card_states));
    let data = serde_json::json!({
        "sessions": rows,
        "active_session": active,
        "active_key": encode_session_key(&session_key),
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
    let sessions = state.router.session_snapshot().await;
    let card_states = state.router.card_state_snapshot().await;
    let active = Some(&session_key).map(|k| session_agent_summary(k, &sessions, &card_states));
    let data = serde_json::json!({ "active_session": active });
    render_template(&state, "agent_timeline.html", "agent", &data).await
}

/// 创建项目 session：接受 git 仓库路径，展开 `~`，校验存在且为目录，
/// 以自动生成的 prompt 在该目录下 spawn 一个 agent 会话。
pub async fn api_create_project(
    State(state): State<WebUiState>,
    Form(req): Form<CreateProjectRequest>,
) -> impl IntoResponse {
    let raw = req.path.trim().to_string();
    let expanded = expand_home_tilde(&raw);
    let project_dir = std::path::Path::new(&expanded);
    if !project_dir.exists() || !project_dir.is_dir() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("路径不存在或不是目录: {expanded}") })),
        );
    }
    let prompt = format!("Work in {expanded} — understand the project structure and help the user with their tasks.");
    let key = state
        .router
        .web_spawn(prompt, Some(expanded))
        .await;
    let encoded = encode_session_key(&key);
    state.router.web_set_active(key.clone()).await;
    let _ = state.event_tx.send(WebUiEvent::SessionCreated {
        session_id: encoded.clone(),
    });
    (
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "key": encoded })),
    )
}

/// 给 agent 会话发消息。
pub async fn api_agent_message(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
    Form(req): Form<SendMessageRequest>,
) -> impl IntoResponse {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return Html("".to_string()),
    };
    state
        .router
        .web_send_message(session_key.clone(), req.message)
        .await;
    // 返回 timeline 片段以便 HTMX 立即刷新。
    let sessions = state.router.session_snapshot().await;
    let card_states = state.router.card_state_snapshot().await;
    let active = Some(&session_key).map(|k| session_agent_summary(k, &sessions, &card_states));
    let data = serde_json::json!({ "active_session": active });
    render_template(&state, "agent_timeline.html", "agent", &data).await
}

#[derive(Deserialize)]
pub struct CreateProjectRequest {
    pub path: String,
}

/// Agent 页 focused session 的完整渲染数据（含 prompt/body/phase_display）。
fn session_agent_summary(
    key: &SessionKey,
    sessions: &[(SessionKey, Mapping)],
    card_states: &std::collections::HashMap<String, CardState>,
) -> serde_json::Value {
    let mapping = sessions
        .iter()
        .find(|(k, _)| k.chat_id == key.chat_id && k.thread_id == key.thread_id)
        .map(|(_, m)| m);
    let (status, session_id, project_dir) = match mapping {
        Some(Mapping { state, project_dir, .. }) => match state {
            MappingState::Active { session_id } => ("active", Some(session_id.clone()), project_dir.clone()),
            MappingState::Dormant { session_id } => ("dormant", Some(session_id.clone()), project_dir.clone()),
            MappingState::Spawning { .. } => ("spawning", None, project_dir.clone()),
        },
        None => ("dormant", None, None),
    };
    let (phase, prompt, phase_display, body_view) = match &session_id {
        Some(sid) => card_states
            .get(sid)
            .map(|st| {
                let phase = st.status_emoji.clone();
                let body: Vec<CardElementView> = st.body.iter().map(card_element_to_view).collect();
                let phase_display = emoji_to_display(&phase).to_string();
                (phase, st.user_prompt.clone(), phase_display, body)
            })
            .unwrap_or_default(),
        None => Default::default(),
    };
    serde_json::json!({
        "chat_id": key.chat_id,
        "thread_id": key.thread_id,
        "session_id": session_id,
        "status": status,
        "phase": phase,
        "phase_display": phase_display,
        "prompt": prompt,
        "body": body_view,
        "project_dir": project_dir,
        "last_active": mapping.map(|m| format_relative_time(m.last_active_unix)).unwrap_or_default(),
        "encoded_key": encode_session_key(key),
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

/// Build `SessionRow`s from session snapshots, returning counts. Marks each
/// row with `is_active` so the template can render the active indicator.
fn build_session_rows(
    sessions: &[(SessionKey, Mapping)],
    card_states: &std::collections::HashMap<String, CardState>,
    active_key: Option<&SessionKey>,
) -> (Vec<SessionRow>, usize, usize, usize) {
    let mut active = 0usize;
    let mut dormant = 0usize;
    let mut spawning = 0usize;

    let mut rows: Vec<SessionRow> = sessions
        .iter()
        .map(|(key, mapping)| {
            let (status, phase) = match &mapping.state {
                MappingState::Active { session_id } => {
                    active += 1;
                    let phase = card_states
                        .get(session_id)
                        .map(|st| st.status_emoji.clone())
                        .unwrap_or_default();
                    ("active", phase)
                }
                MappingState::Dormant { .. } => {
                    dormant += 1;
                    ("dormant", String::new())
                }
                MappingState::Spawning { .. } => {
                    spawning += 1;
                    ("spawning", String::new())
                }
            };
            let is_active = active_key
                .map(|a| a.chat_id == key.chat_id && a.thread_id == key.thread_id)
                .unwrap_or(false);
            // Agent page 标签：project_dir 优先（📁）；否则用首条用户消息
            // 预览（💬），取前 80 字符。
            let prompt_preview = mapping.session_id().and_then(|sid| {
                card_states
                    .get(sid)
                    .map(|st| {
                        let p = st.user_prompt.trim();
                        if p.is_empty() {
                            None
                        } else if p.chars().count() > 80 {
                            let short: String = p.chars().take(80).collect();
                            Some(format!("{short}…"))
                        } else {
                            Some(p.to_string())
                        }
                    })
                    .flatten()
            });
            SessionRow {
                encoded_key: encode_session_key(key),
                chat_id: key.chat_id.clone(),
                thread_id: key.thread_id.clone(),
                session_id: mapping.session_id().map(|s| s.to_string()),
                status,
                phase,
                last_active: format_relative_time(mapping.last_active_unix),
                is_active,
                project_dir: mapping.project_dir.clone(),
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

/// Compact summary used by the dashboard's "active session" panel.
fn session_summary(key: &SessionKey, sessions: &[(SessionKey, Mapping)]) -> serde_json::Value {
    let mapping = sessions
        .iter()
        .find(|(k, _)| k.chat_id == key.chat_id && k.thread_id == key.thread_id)
        .map(|(_, m)| m);
    let (status, session_id) = match mapping {
        Some(Mapping { state, .. }) => match state {
            MappingState::Active { session_id } => ("active", Some(session_id.clone())),
            MappingState::Dormant { session_id } => ("dormant", Some(session_id.clone())),
            MappingState::Spawning { .. } => ("spawning", None),
        },
        None => ("dormant", None),
    };
    serde_json::json!({
        "chat_id": key.chat_id,
        "thread_id": key.thread_id,
        "session_id": session_id,
        "status": status,
        "encoded_key": encode_session_key(key),
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
    let active_key = state.router.active_session_snapshot().await;
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

/// Convert a CardElement to a view model for template rendering.
fn card_element_to_view(el: &CardElement) -> CardElementView {
    match el {
        CardElement::Markdown { content } => CardElementView {
            element_type: "markdown",
            content: content.clone(),
        },
        CardElement::Div { text } => CardElementView {
            element_type: "div",
            content: text.content.clone(),
        },
        CardElement::CollapsiblePanel(panel) => {
            let header = panel.header.title.content.as_str();
            let body: Vec<String> = panel
                .elements
                .iter()
                .map(|e| match e {
                    CardElement::Markdown { content } => content.clone(),
                    CardElement::Div { text } => text.content.clone(),
                    _ => String::new(),
                })
                .collect();
            let content = format!(
                "<details><summary>{header}</summary>{}</details>",
                body.join("\n")
            );
            CardElementView {
                element_type: "collapsible",
                content,
            }
        }
        CardElement::Hr => CardElementView {
            element_type: "hr",
            content: String::new(),
        },
        _ => CardElementView {
            element_type: "other",
            content: String::new(),
        },
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
