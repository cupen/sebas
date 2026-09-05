//! JSON API for the WebUI frontend: HTTP endpoints under `/api/*` and the
//! WebSocket realtime channel at `/ws`.
//!
//! This module is the client-agnostic contract documented in the
//! `webui-api` capability: every SPA view (and any future local client)
//! consumes it. All session data flows through the `SessionBackend` seam —
//! handlers never know whether the session authority is in-process
//! (`run --webui`) or across the core session channel (standalone webui).

use crate::events::WebUiEvent;
use crate::models::{CardConfigInfo, CardElementView, SessionStatus};
use crate::routes::{
    build_session_rows, decode_session_key, encode_channel_key, encode_session_key,
    format_relative_time, format_uptime, session_summary,
};
use crate::server::WebUiState;
use crate::session_backend::{Reachability, SessionRejection};
use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use sebas_router::SessionEvent;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use tokio::time::MissedTickBehavior;

/// How often the server pings connected WebSocket clients to keep the
/// connection (and intermediaries) alive.
const WS_PING_INTERVAL: Duration = Duration::from_secs(15);

/// Uniform JSON error body: `{ "error": "..." }` with a status code.
fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// Map a typed backend rejection onto the HTTP surface. Rejections never
/// mutate state, so the error text can be the rejection's own wording.
fn rejection_response(rej: SessionRejection) -> Response {
    let status = match &rej {
        SessionRejection::UnknownSession { .. } => StatusCode::NOT_FOUND,
        SessionRejection::Unavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        SessionRejection::UnusableProjectDir | SessionRejection::Capacity { .. } => {
            StatusCode::BAD_REQUEST
        }
    };
    api_error(status, rej.to_string())
}

// ---- Read endpoints ----

/// GET /api/summary — dashboard overview: counts, uptime, focused session.
pub async fn summary(State(state): State<WebUiState>) -> Response {
    let infos = state.backend.snapshot().await;
    let focused = state.backend.focused().await;
    let reachability = state.backend.reachability().await;
    let (rows, active, dormant, spawning) = build_session_rows(&infos, focused.as_ref());
    let active_session = focused.as_ref().and_then(|f| {
        infos
            .iter()
            .find(|i| i.channel == f.channel.as_str() && i.key == f.reference)
            .map(session_summary)
    });

    let data = json!({
        "active_count": active,
        "dormant_count": dormant,
        "spawning_count": spawning,
        "total_sessions": active + dormant + spawning,
        "uptime": format_uptime(state.started_at.elapsed()),
        "recent_sessions": rows,
        "active_session": active_session,
        "active_session_key": focused.as_ref().map(encode_session_key),
        "reachability": reachability_payload(&reachability),
    });
    Json(data).into_response()
}

/// Serialize the reachability report for the composer gate. Only the
/// "unreachable" branch carries a cause — reachable is just `{}`.
fn reachability_payload(r: &Reachability) -> serde_json::Value {
    match r {
        Reachability::Reachable => json!({ "ok": true }),
        Reachability::Unreachable { cause } => json!({ "ok": false, "cause": cause }),
    }
}

/// GET /api/sessions — every session row plus counts, focused-first.
/// Runs archive cleanup before returning.
pub async fn sessions_list(State(state): State<WebUiState>) -> Response {
    crate::archive::cleanup_expired();
    let infos = state.backend.snapshot().await;
    let focused = state.backend.focused().await;
    let (rows, active, dormant, spawning) = build_session_rows(&infos, focused.as_ref());

    let data = json!({
        "recent_sessions": rows,
        "active_count": active,
        "dormant_count": dormant,
        "spawning_count": spawning,
        "total_sessions": active + dormant + spawning,
        "active_session_key": focused.as_ref().map(encode_session_key),
    });
    Json(data).into_response()
}

/// GET /api/sessions/{key} — session detail with the rendered transcript.
/// A successful read focuses the session, same as the former detail page
/// visit (a display pointer only; it never changes message routing).
pub async fn session_detail(State(state): State<WebUiState>, Path(key): Path<String>) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };

    let infos = state.backend.snapshot().await;
    let info = match infos
        .iter()
        .find(|i| i.channel == session_key.channel.as_str() && i.key == session_key.reference)
    {
        Some(i) => i,
        None => return api_error(StatusCode::NOT_FOUND, "Session not found"),
    };

    // Reading the detail focuses this session in the dashboard.
    state.backend.set_focus(Some(session_key.clone())).await;

    // A known session with no readable transcript yet renders empty
    // rather than failing the view.
    let entries: Vec<sebas_router::TurnEntry> =
        state.backend.turns(session_key.clone(), 0).await.unwrap_or_default();
    // The transcript carries the agent/tool output blocks; the current
    // turn's prompt travels in `user_prompt`. The SPA renders markdown
    // blocks and hides the rest (thinking blocks stay in the payload for
    // forward-compatible clients). The wall-clock stamp travels with each
    // entry so the SPA can render a flush-left timestamp and anchor the
    // seen-boundary seam to a stable identity that doesn't change when an
    // earlier card refreshes in place.
    let body: Vec<CardElementView> = entries
        .iter()
        .filter(|e| e.kind != "prompt")
        .map(|e| CardElementView {
            element_type: match e.element_type.as_str() {
                "thinking" => "thinking",
                _ => "markdown",
            },
            content: e.content.clone(),
            created_at_unix: e.created_at_unix,
        })
        .collect();

    let derived = SessionStatus::derive(&info.status, info.phase.as_deref().unwrap_or(""));

    let data = json!({
        "channel": info.channel,
        "reference": info.key,
        "session_id": info.session_id,
        "status": info.status,
        "status_label": derived.label(),
        "status_slug": derived.slug(),
        "status_glyph": derived.glyph(),
        "user_prompt": info.user_prompt,
        "body": body,
        // The seam does not transport the core's msg-id bookkeeping; the
        // SPA tolerates a null here.
        "msg_id": Option::<String>::None,
        "last_active": format_relative_time(info.last_active_unix),
        "encoded_key": encode_session_key(&session_key),
        // （add-acp-model-selection）会话模型面：无模型选项的 agent（如
        // Claude）这两个字段为 null —— 前端不显示模型 UI。
        "current_model": info.current_model,
        "available_models": info.available_models,
        // （add-composer-agent-binding）创建时绑定的 agent kind；null = 默认。
        "agent_kind": info.agent_kind,
    });
    Json(data).into_response()
}

/// GET /api/settings — card config and basic gateway info. 状态库可用时（
/// add-state-store 5.2）card config 从 backend 实时读取；否则回退启动快照。
pub async fn settings(State(state): State<WebUiState>) -> Response {
    // 尝试从后端状态库读最新 settings（CardConfig 形状）。
    let card_config_info = match state.backend.state_snapshot("settings").await {
        Some(v) => {
            // v 是 CardConfig 的形状；转成 CardConfigInfo。
            let theme_color = v
                .get("theme_color")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&state.card_config.theme_color)
                .to_string();
            let fold_long_output = v
                .get("fold_long_output")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(state.card_config.fold_long_output);
            let thinking = v
                .get("thinking")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{:?}", state.card_config.thinking));
            let max_user_text_chars = v
                .get("max_user_text_chars")
                .and_then(serde_json::Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(state.card_config.max_user_text_chars);
            let max_tool_output_chars = v
                .get("max_tool_output_chars")
                .and_then(serde_json::Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(state.card_config.max_tool_output_chars);
            CardConfigInfo {
                theme_color,
                fold_long_output,
                thinking_display: thinking,
                max_user_text_chars,
                max_tool_output_chars,
            }
        }
        None => CardConfigInfo {
            theme_color: state.card_config.theme_color.clone(),
            fold_long_output: state.card_config.fold_long_output,
            thinking_display: format!("{:?}", state.card_config.thinking),
            max_user_text_chars: state.card_config.max_user_text_chars,
            max_tool_output_chars: state.card_config.max_tool_output_chars,
        },
    };

    let data = json!({
        "card_config": card_config_info,
        "gateway": state.gateway,
    });
    Json(data).into_response()
}

/// GET /api/gateway — detailed provider status.
pub async fn gateway(State(state): State<WebUiState>) -> Response {
    Json(json!({ "gateway": state.gateway })).into_response()
}

/// GET /api/about — version info and system status.
pub async fn about(State(state): State<WebUiState>) -> Response {
    let data = json!({
        "uptime": format_uptime(state.started_at.elapsed()),
        "version": env!("CARGO_PKG_VERSION"),
        "rustc_version": env!("CARGO_PKG_RUST_VERSION"),
        "gateway_listen": state.gateway.listen,
        "provider_count": state.gateway.provider_count,
    });
    Json(data).into_response()
}

/// GET /api/agent-kinds — the reachable third-party agent kinds for the
/// create-session dropdown, plus their failure causes when unreachable. The
/// client lists only `reachable` kinds alongside the built-in `native` entry.
pub async fn agent_kinds(State(state): State<WebUiState>) -> Response {
    let kinds = state.agent_kinds.agent_kinds().await;
    Json(json!({ "kinds": kinds })).into_response()
}

// ---- Mutation endpoints ----

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    /// Optional prompt for the new session. When omitted, the session is
    /// created as a 0-turn placeholder (no ACP child is spawned until the
    /// first message is sent).
    #[serde(default)]
    pub prompt: Option<String>,
    /// Optional project directory for the new session's working dir. When
    /// omitted (or null), the session is bound to the workbench inbox
    /// (`project_dir = None`). The backend may reject a non-directory
    /// path with `UnusableProjectDir` — the client surfaces that verbatim.
    #[serde(default)]
    pub project_dir: Option<String>,
    /// Optional execution-backend hint (composite seams route on it, e.g.
    /// `"acp"` for the Claude Code bridge vs `"native"` for the built-in
    /// agent). Single-backend seams ignore it.
    #[serde(default)]
    pub backend: Option<String>,
    /// Optional model id for the new session (add-acp-model-selection D3):
    /// applied after the session is established, before the first prompt.
    /// Only agents that expose a `model` config option honor it; for others
    /// the field is a silent no-op (the session uses its default model and
    /// no model UI is shown).
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Deserialize)]
pub struct SetModelRequest {
    pub model_id: String,
}

#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub message: String,
}

/// POST /api/sessions — create a session. Returns 201 with
/// the encoded key. When `prompt` is provided, the session is spawned
/// with that first message (ACP child starts immediately). When omitted,
/// a 0-turn placeholder session is created (no ACP child until the first
/// message).
pub async fn create_session(
    State(state): State<WebUiState>,
    Json(req): Json<CreateSessionRequest>,
) -> Response {
    let prompt = req.prompt.unwrap_or_default();
    // 0-turn 占位（P2）：无 prompt 时只建行、不 spawn 子进程、不把空串当
    // prompt 发给 agent（opencode 收到 `session/prompt ""` 会挂起）。kind/
    // model/project_dir 记在 mapping 上，首条消息到达时才 spawn。
    let key = if prompt.trim().is_empty() {
        match state
            .backend
            .create_placeholder(req.project_dir.clone(), req.backend.clone(), req.model.clone())
            .await
        {
            Ok(k) => k,
            Err(rej) => return rejection_response(rej),
        }
    } else {
        match state
            .backend
            .spawn_with(prompt, req.project_dir, req.backend.as_deref(), req.model)
            .await
        {
            Ok(k) => k,
            Err(rej) => return rejection_response(rej),
        }
    };
    state.backend.set_focus(Some(key.clone())).await;
    let encoded = encode_session_key(&key);
    (StatusCode::CREATED, Json(json!({ "key": encoded }))).into_response()
}

/// POST /api/sessions/{key}/model — set the session's model mid-session
/// (add-acp-model-selection 2.3). The command is delivered to the session
/// driver；wire 层接受与否经事件流反馈（`ModelChanged` = 成功；非 terminal
/// `Error` = 模型被 agent 拒绝，UI 显示错误、模型不变）。
pub async fn set_session_model(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
    Json(req): Json<SetModelRequest>,
) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };
    if let Err(rej) = state.backend.set_session_model(session_key, req.model_id).await {
        return rejection_response(rej);
    }
    (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response()
}

/// POST /api/sessions/{key}/message — send a message into the session.
/// Returns 400 if the session is archived.
pub async fn send_message(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Response {
    // Reject messages to archived sessions.
    if crate::archive::is_archived(&key) {
        return api_error(StatusCode::BAD_REQUEST, "Session is archived");
    }
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };
    if let Err(rej) = state.backend.message(session_key, req.message).await {
        return rejection_response(rej);
    }
    (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response()
}

/// POST /api/sessions/{key}/close — kill and remove a session. Returns 200
/// with the new focused session key (or null); 404 if the key mapped to
/// nothing.
pub async fn close_session(State(state): State<WebUiState>, Path(key): Path<String>) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };

    if let Err(rej) = state.backend.close(session_key).await {
        return rejection_response(rej);
    }
    let focused = state.backend.focused().await;
    (
        StatusCode::OK,
        Json(json!({
            "status": "closed",
            "active_session_key": focused.as_ref().map(encode_session_key),
        })),
    )
        .into_response()
}

/// POST /api/sessions/{key}/switch — move the focused-session pointer.
/// Returns the client route target so the SPA can navigate; 404 for an
/// unknown key so the client does not navigate to a dead view.
///
/// axum's `Path` extractor percent-decodes the key; the redirect re-encodes
/// it so the client receives a usable URL segment, not one containing a raw
/// NUL byte.
pub async fn switch_session(State(state): State<WebUiState>, Path(key): Path<String>) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };

    let infos = state.backend.snapshot().await;
    if !infos
        .iter()
        .any(|i| i.channel == session_key.channel.as_str() && i.key == session_key.reference)
    {
        return api_error(StatusCode::NOT_FOUND, "Session not found");
    }

    state.backend.set_focus(Some(session_key.clone())).await;
    let encoded = encode_session_key(&session_key);
    (
        StatusCode::OK,
        Json(json!({
            "status": "switched",
            "redirect": format!("/sessions/{}", encoded),
            "active_session_key": encoded,
        })),
    )
        .into_response()
}

/// GET /api/fs/browse-dirs?path=...&root=... — list only subdirectories for
/// the directory tree picker. `root` defaults to `/` (full filesystem).
/// All paths are resolved relative to and bounded within `root`.
pub async fn browse_dirs(
    axum::extract::Query(params): axum::extract::Query<crate::fs::BrowseParams>,
) -> Response {
    let path = params.path.as_deref().unwrap_or("");
    let root = params.root.as_deref();
    match crate::fs::browse_dirs(path, root) {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => api_error(StatusCode::BAD_REQUEST, e),
    }
}

// ---- Project API endpoints ----

/// 从 backend 读取项目列表（DB 引擎 / core 通道）。backend 不可达时回退
/// 本地文件注册表（webui 进程独占视图，spec 未约束其降级语义）。
/// 返回 JSON 数组（ProjectRow / ProjectEntry 形状，前端兼容）。
async fn projects_from_backend(
    state: &WebUiState,
) -> Result<Vec<serde_json::Value>, Response> {
    if let Some(v) = state.backend.state_snapshot("projects").await {
        let arr = v
            .get("projects")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        return Ok(arr);
    }
    // 回退：webui 本地文件注册表。
    Ok(crate::projects::list()
        .into_iter()
        .map(|e| serde_json::to_value(&e).unwrap_or_default())
        .collect())
}

/// GET /api/projects — list all registered projects（状态库优先，文件回退）。
pub async fn projects_list(State(state): State<WebUiState>) -> Response {
    match projects_from_backend(&state).await {
        Ok(projects) => Json(json!({ "projects": projects })).into_response(),
        Err(resp) => resp,
    }
}

/// POST /api/projects — register a new project directory（状态库优先）。
pub async fn projects_add(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return api_error(StatusCode::BAD_REQUEST, "missing 'path' field"),
    };
    // 本地校验：路径必须存在且是目录（canonicalize 后注册 canonical 路径）。
    let dir = std::path::Path::new(path);
    if !dir.exists() {
        return api_error(StatusCode::BAD_REQUEST, format!("路径不存在: {path}"));
    }
    if !dir.is_dir() {
        return api_error(StatusCode::BAD_REQUEST, format!("路径不是目录: {path}"));
    }
    let canonical = match dir.canonicalize() {
        Ok(c) => c.to_string_lossy().to_string(),
        Err(_) => return api_error(StatusCode::BAD_REQUEST, format!("无法解析路径: {path}")),
    };
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string());
    // 重复检查：列表里已有 canonical 路径 → 409。
    if let Ok(projects) = projects_from_backend(&state).await
        && projects
            .iter()
            .any(|p| p.get("path").and_then(|v| v.as_str()) == Some(canonical.as_str()))
    {
        return api_error(StatusCode::CONFLICT, format!("项目已注册: {path}"));
    }
    // backend 可用 → 状态库；不可用 → 文件注册表（降级）。
    if state
        .backend
        .state_mutate(
            "projects",
            json!({ "op": "add", "path": canonical.clone(), "name": name.clone() }),
        )
        .await
        .is_ok()
        || crate::projects::add(&canonical).is_ok()
    {
        // 返回新条目（从列表反查，保证与数据源一致）。
        match projects_from_backend(&state).await {
            Ok(list) => {
                let entry = list.into_iter().find(|p| {
                    p.get("path").and_then(|v| v.as_str()) == Some(canonical.as_str())
                });
                if let Some(entry) = entry {
                    return (StatusCode::CREATED, Json(entry)).into_response();
                }
            }
            Err(_) => {}
        }
        (StatusCode::CREATED, Json(json!({ "path": canonical, "name": name }))).into_response()
    } else {
        api_error(StatusCode::SERVICE_UNAVAILABLE, "无法注册项目（状态库与本地均失败）")
    }
}

/// POST /api/projects/{path}/remove — unregister a project（状态库优先）。
pub async fn projects_remove(
    State(state): State<WebUiState>,
    Path(path): Path<String>,
) -> Response {
    let decoded = match urlencoding::decode(&path) {
        Ok(d) => d.into_owned(),
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid path encoding"),
    };
    // 先试状态库；失败（不存在或不可达）再试文件。
    match state
        .backend
        .state_mutate("projects", json!({ "op": "remove", "path": decoded.clone() }))
        .await
    {
        Ok(()) => Json(json!({ "status": "removed" })).into_response(),
        Err(e) => {
            if e.contains("不存在") {
                // 状态库没有 → 试文件注册表。
                match crate::projects::remove(&decoded) {
                    Ok(true) => Json(json!({ "status": "removed" })).into_response(),
                    Ok(false) => api_error(StatusCode::NOT_FOUND, "project not found"),
                    Err(_) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "remove failed"),
                }
            } else {
                // 状态库不可达 → 回退文件。
                match crate::projects::remove(&decoded) {
                    Ok(true) => Json(json!({ "status": "removed" })).into_response(),
                    Ok(false) => api_error(StatusCode::NOT_FOUND, "project not found"),
                    Err(_) => api_error(StatusCode::SERVICE_UNAVAILABLE, e),
                }
            }
        }
    }
}

#[derive(Deserialize)]
pub struct ReorderRequest {
    /// Ordered list of canonical project paths; entries not listed are
    /// appended at the end (preserving their relative add-time order).
    pub paths: Vec<String>,
}

/// POST /api/projects/reorder — persist the user's rail ordering（状态库优先）。
pub async fn projects_reorder(
    State(state): State<WebUiState>,
    Json(req): Json<ReorderRequest>,
) -> Response {
    // 读当前列表 → 按新顺序重排（未知路径落地为 add_time 顺序尾部）→ save。
    let mut projects = match projects_from_backend(&state).await {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    let mut by_path: std::collections::HashMap<String, serde_json::Value> =
        projects.drain(..).map(|p| {
            let path = p
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            (path, p)
        }).collect();
    let mut next: Vec<serde_json::Value> = Vec::with_capacity(req.paths.len());
    let mut seen = std::collections::HashSet::new();
    for path in &req.paths {
        if seen.insert(path.clone())
            && let Some(entry) = by_path.remove(path)
        {
            next.push(entry);
        }
    }
    // 未提及的项目追加尾部（按 added_at 稳定）。
    let mut tail: Vec<(i64, serde_json::Value)> = by_path
        .into_values()
        .map(|v| {
            let t = v
                .get("added_at")
                .and_then(|x| x.as_i64())
                .unwrap_or(0);
            (t, v)
        })
        .collect();
    tail.sort_by_key(|(t, _)| *t);
    next.extend(tail.into_iter().map(|(_, v)| v));
    // 重写 sort_order 为列表序号（状态库 save 语义）。
    for (i, entry) in next.iter_mut().enumerate() {
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("sort_order".into(), json!(i as i64));
        }
    }
    // 状态库优先；不可达时回退文件注册表 reorder。
    let via_backend = state
        .backend
        .state_mutate("projects", json!({ "op": "save", "projects": next.clone() }))
        .await
        .is_ok();
    if via_backend {
        return Json(json!({ "projects": next })).into_response();
    }
    // 文件回退：把 next 形状转回 ProjectEntry 数组。
    let entries: Vec<crate::projects::ProjectEntry> = next
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();
    match crate::projects::save_ordered(&entries) {
        Ok(()) => Json(json!({ "projects": entries })).into_response(),
        Err(e) => api_error(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

/// GET /api/projects/{path}/branch — current git branch (TTL-cached server-side)。
/// Git 探测是本地文件系统操作；backend 不可达时列表来自文件回退，语义一致。
pub async fn projects_branch(
    State(state): State<WebUiState>,
    Path(path): Path<String>,
) -> Response {
    let decoded = match urlencoding::decode(&path) {
        Ok(d) => d.into_owned(),
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid path encoding"),
    };
    let projects = match projects_from_backend(&state).await {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    let entry = projects
        .iter()
        .find(|p| p.get("path").and_then(|v| v.as_str()) == Some(decoded.as_str()));
    let Some(entry) = entry else {
        return api_error(StatusCode::NOT_FOUND, "project not found");
    };
    let accessible = crate::projects::is_accessible(&decoded);
    // TTL 缓存：branch_at 距今 < 30s 用缓存。
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let branch_at = entry.get("branch_at").and_then(|v| v.as_i64()).unwrap_or(0);
    let cached_branch = entry.get("branch").and_then(|v| v.as_str()).map(str::to_string);
    let branch = if branch_at != 0 && now.saturating_sub(branch_at) < 30 && cached_branch.is_some() {
        cached_branch
    } else {
        let fresh = crate::projects::probe_git_branch(std::path::Path::new(&decoded));
        // 简化：返回探测值，不强制写回（分支缓存不是共享真源）。
        fresh
    };
    Json(json!({
        "path": decoded,
        "branch": branch,
        "accessible": accessible,
    }))
    .into_response()
}

// ---- Archive API endpoints ----

/// GET /api/archive — list archived sessions. Runs cleanup before returning.
pub async fn archive_list(State(_state): State<WebUiState>) -> Response {
    crate::archive::cleanup_expired();
    let entries = crate::archive::list();
    Json(json!({ "archived_sessions": entries })).into_response()
}

/// POST /api/sessions/{key}/archive — archive a session.
/// Moves it from the active session list into the archive. The session is
/// closed (child killed if active) and set to read-only.
pub async fn archive_session(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };

    // Look up the session to get its project_dir and label.
    let infos = state.backend.snapshot().await;
    let info = match infos.iter().find(|i| {
        i.channel == session_key.channel.as_str() && i.key == session_key.reference
    }) {
        Some(i) => i,
        None => return api_error(StatusCode::NOT_FOUND, "Session not found"),
    };

    let project_path = info.project_dir.clone().unwrap_or_default();
    let label = info.user_prompt.clone().unwrap_or_else(|| info.session_id.clone().unwrap_or_else(|| "unnamed".to_string()));

    // Close the session first (kills child if active).
    if let Err(_rej) = state.backend.close(session_key).await {
        // If close fails (unknown, unavailable), we still proceed with the archive.
    }

    match crate::archive::archive_session(&key, &project_path, &label, state.archive_retention_days) {
        Ok(entry) => (StatusCode::OK, Json(json!({ "status": "archived", "entry": entry }))).into_response(),
        Err(e) => api_error(StatusCode::BAD_REQUEST, e),
    }
}

/// POST /api/sessions/{key}/restore — restore an archived session to its
/// original project.
pub async fn restore_session(
    State(_state): State<WebUiState>,
    Path(key): Path<String>,
) -> Response {
    match crate::archive::restore_session(&key) {
        Some(entry) => {
            // The session key is the same, so it reappears in the next snapshot
            // fetch. The frontend will re-fetch the session list.
            (StatusCode::OK, Json(json!({ "status": "restored", "entry": entry }))).into_response()
        }
        None => api_error(StatusCode::NOT_FOUND, "Archived session not found"),
    }
}

/// GET /ws — upgrade to a WebSocket and stream session events as
/// self-describing JSON frames. Every connected client receives every
/// event via its own backend subscription; one client disconnecting never
/// affects the others. Unknown event types are forward-compatible
/// additions clients must tolerate.
pub async fn ws_handler(State(state): State<WebUiState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| ws_connection(state, socket))
}

/// Translate a backend session event into the WS frame vocabulary the SPA
/// keys off (`session.*`, dotted tags). `Resync` carries no frame: the
/// client's next fetch converges the view, and the frame contract has no
/// resync type.
fn session_event_to_frame(ev: SessionEvent) -> Option<WebUiEvent> {
    match ev {
        SessionEvent::Created { session } => Some(WebUiEvent::SessionCreated {
            session_id: encode_channel_key(&session.channel, &session.key),
        }),
        SessionEvent::Updated { session } => Some(WebUiEvent::SessionUpdated {
            session_id: encode_channel_key(&session.channel, &session.key),
            status: session.status,
        }),
        SessionEvent::Removed {
            channel,
            key,
        } => Some(WebUiEvent::SessionRemoved {
            session_id: encode_channel_key(&channel, &key),
        }),
        SessionEvent::Resync => None,
    }
}

/// Per-connection loop: forwards backend session events, answers the
/// protocol keep-alive with server pings, and drains client frames (only
/// Close is meaningful) until either side hangs up.
async fn ws_connection(state: WebUiState, socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let mut events = state.backend.subscribe();
    // Review-card feed (gated tool calls). Backends without permission
    // interaction yield `None`; the select leg below then never fires.
    let mut permissions = state.backend.permission_requests();
    let mut ping = tokio::time::interval(WS_PING_INTERVAL);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ping.reset();

    loop {
        tokio::select! {
            _ = ping.tick() => {
                if sender.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if let Some(frame) = session_event_to_frame(event) {
                            let text = serde_json::to_string(&frame).unwrap_or_default();
                            if sender.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    // A slow client lagged the broadcast: skip what it missed
                    // rather than killing the connection.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            notice = async {
                match permissions.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok(notice) = notice {
                    let frame = WebUiEvent::PermissionRequested {
                        request_id: notice.request_id,
                        session_id: notice.session_id,
                        tool_name: notice.tool_name,
                        args: notice.args,
                        reason: notice.reason,
                    };
                    let text = serde_json::to_string(&frame).unwrap_or_default();
                    if sender.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
            frame = receiver.next() => {
                match frame {
                    Some(Ok(msg)) => {
                        if matches!(msg, Message::Close(_)) {
                            break;
                        }
                        // Any other client frame is drained and ignored: the
                        // channel is server-push only.
                    }
                    _ => break,
                }
            }
        }
    }
}

/// POST /api/permissions/{request_id}/answer — deliver the operator's
/// decision for a gated tool call (the review card). `404` when no pending
/// request carries that id (already answered, timed out, or unknown — the
/// client may retry briefly).
#[derive(Deserialize)]
pub struct AnswerPermissionRequest {
    pub decision: crate::session_backend::PermissionDecision,
}

pub async fn answer_permission(
    State(state): State<WebUiState>,
    Path(request_id): Path<String>,
    Json(req): Json<AnswerPermissionRequest>,
) -> Response {
    let delivered = state
        .backend
        .answer_permission(&request_id, req.decision)
        .await;
    if delivered {
        Json(json!({ "status": "delivered" })).into_response()
    } else {
        api_error(StatusCode::NOT_FOUND, "no pending permission request with that id")
    }
}
