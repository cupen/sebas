//! JSON API for the WebUI frontend: HTTP endpoints under `/api/*` and the
//! WebSocket realtime channel at `/ws`.
//!
//! This module is the client-agnostic contract documented in the
//! `webui-api` capability: every SPA view (and any future local client)
//! consumes it. All session data flows through the `SessionBackend` seam —
//! handlers never know whether the session authority is in-process
//! (`core --webui`) or across the core session channel (standalone webui).

use crate::events::WebUiEvent;
use crate::models::{CardConfigInfo, ConversationEntryView, SessionStatus};
use crate::routes::{
    build_session_rows, decode_session_key, encode_channel_key, encode_session_key,
    format_relative_time, format_uptime, session_summary,
};
use crate::server::WebUiState;
use crate::session_backend::{Reachability, SessionRejection};
use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use sebas_dispatch::SessionEvent;
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
        SessionRejection::BackendUnavailable { .. } => StatusCode::CONFLICT,
        SessionRejection::UnusableProjectDir | SessionRejection::Capacity { .. } => {
            StatusCode::BAD_REQUEST
        }
        // workbench-turn-queue 5.1/D7：满队列 409（4xx 点名上限）；pending
        // 管理拒绝按原因映射——未知 404、已开始/越优先 409、越界 400。
        SessionRejection::QueueFull { .. } => StatusCode::CONFLICT,
        SessionRejection::PendingRejected { reason } => match reason {
            crate::session_backend::PendingReason::Unknown => StatusCode::NOT_FOUND,
            crate::session_backend::PendingReason::AlreadyStarted
            | crate::session_backend::PendingReason::PriorityConflict => StatusCode::CONFLICT,
            crate::session_backend::PendingReason::OutOfRange => StatusCode::BAD_REQUEST,
        },
        // workbench-interaction-polish 1.1：空闲会话的取消是可重试的冲突——
        // 会话还在、只是没有在飞 turn 可停（409 而非 404，与「未知会话」区分）。
        SessionRejection::Idle { .. } => StatusCode::CONFLICT,
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
    // workbench-conversation-view 1.4（design D1）：聚焦会话与 detail 同形状
    // ——条目序列随 summary 下发，客户端不必再为对话内容发第二个请求；
    // 取不到（瞬时失败）给空序列而不是失败 payload。
    let active_session = match focused.as_ref() {
        Some(f) => {
            let turns = state.backend.turns(f.clone(), 0).await.unwrap_or_default();
            infos
                .iter()
                .find(|i| i.channel == f.channel.as_str() && i.key == f.reference)
                .map(|info| session_summary(info, &turns))
        }
        None => None,
    };

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
        // wire-webui-sebas-agent-e2e：双执行体的逐体可用性。前端 composer 按
        // 此渲染（缺凭据的 native 禁选 + cause 标注）；后端不区分执行体时
        // 省略该段（前端降级为只看整体 reachability）。
        "execution_bodies": state
            .backend
            .execution_bodies()
            .await
            .map(|bodies| serde_json::to_value(bodies).unwrap_or_default()),
    });
    Json(data).into_response()
}

/// Serialize the reachability report for the composer gate. Only the
/// unreachable branches carry a cause plus the machine-readable `kind`
/// discriminator (cover-core-channel-test-gaps A1.1, design D1: the frontend
/// picks its banner wording from `kind`, never from cause string matching);
/// reachable is just `{}`.
fn reachability_payload(r: &Reachability) -> serde_json::Value {
    match r {
        Reachability::Reachable => json!({ "ok": true }),
        Reachability::StartupFailed { cause } => {
            json!({ "ok": false, "kind": "startup_failed", "cause": cause })
        }
        Reachability::AuthRejected { cause } => {
            json!({ "ok": false, "kind": "auth_rejected", "cause": cause })
        }
        Reachability::Disconnected { cause } => {
            json!({ "ok": false, "kind": "disconnected", "cause": cause })
        }
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
    let entries: Vec<sebas_dispatch::TurnEntry> = state
        .backend
        .turns(session_key.clone(), 0)
        .await
        .unwrap_or_default();
    // workbench-conversation-view 1.1/1.2（design D1/D2）：payload 是一条
    // 有序条目序列（提交与 agent 输出同序，position 单调），`user_prompt` 与
    // `body` 退役。`kind` 随条目透传（prompt = 操作员提交），渲染类型
    // `element_type` 保留 core 的 thinking/tool/error 标记，让前端把回合内
    // 的 thinking 折叠、工具收组、错误单独呈现。时间戳随条目走，前端据此
    // 渲染 flush-left 时间并把未读 seam 锚到稳定身份上。
    let conversation: Vec<ConversationEntryView> = entries
        .iter()
        .map(|e| ConversationEntryView {
            position: e.position,
            kind: e.kind.clone(),
            element_type: match e.element_type.as_str() {
                // thinking / tool / error 按原类型透传（conversation-view
                // 1.1/1.3：前端靠它折叠 thinking、收工具组、渲染错误气泡）；
                // 未知遗留值归一为 markdown，内容不丢。
                "thinking" | "tool" | "error" => e.element_type.clone(),
                _ => "markdown".to_string(),
            },
            content: e.content.clone(),
            created_at_unix: e.created_at_unix,
            // workbench-agent-identity-and-process-folds 1.1：工具条目标题
            // 原样透传（None = 旧条目，前端回退通用标签）。
            title: e.title.clone(),
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
        "entries": conversation,
        // The seam does not transport the core's msg-id bookkeeping; the
        // SPA tolerates a null here.
        "msg_id": Option::<String>::None,
        "last_active": format_relative_time(info.last_active_unix),
        "encoded_key": encode_session_key(&session_key),
        // （add-acp-model-selection）会话模型面：无模型选项的 agent（如
        // Claude）这两个字段为 null —— 前端不显示模型 UI。
        "current_model": info.current_model,
        "available_models": info.available_models,
        // （add-agent-mode-selection）权限模式：desired = 操作者期望（创建/
        // 切换立即反映），effective = 执行体回报的实际生效值；两者都可能为
        // null（agent 默认行为 / 执行体未声称生效）。远端会话另有 remote
        // 视图的同名字段，值同源。
        "desired_mode": info.desired_mode,
        "effective_mode": info.effective_mode,
        // （add-composer-agent-binding）创建时绑定的 agent kind；null = 默认。
        "agent_kind": info.agent_kind,
        // 绑定的项目，按稳定 id（workbench-agent-wire-fix 2.5）；null = inbox。
        // 8.1：id 由 `(节点, 路径)` 派生（远端会话不属于本机同路径项目）。
        "project_id": crate::projects::project_id_for_session(info),
        // 8.2/8.3/8.4/8.5：节点/状态/成因/mode/悬空审批整块透传（null = 本机）。
        "remote": info.remote,
        // workbench-turn-queue 6.1：待生效提交全量视图（投递序，staging 先于
        // turn；每条带稳定 id/文本/位置/处置/优先标记）。
        "pending": serde_json::to_value(&info.pending).unwrap_or_default(),
        // rail-declutter-unread：会话的可见回复段数——transcript 标记已读时
        // 以它推进浏览器读锚（seam 与徽标共用，D3）。
        "msg_count": info.msg_count,
    });
    Json(data).into_response()
}

/// GET /api/settings — card config and basic router info. 状态库可用时（
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

    // fix-webui-detached-status：provider 列表取状态库真源（两种部署形态
    // 同源，运行期经 admin API 的增删改免重启可见）；真源不可达时如实标注
    // `providers_available: false`，不把空集冒充"未配置"。
    let (providers_value, providers_available) =
        match state.backend.state_snapshot("providers").await {
            Some(v) if v.get("error").is_none() => {
                let list: Vec<serde_json::Value> = v
                    .get("providers")
                    .and_then(serde_json::Value::as_object)
                    .map(|cards| {
                        cards
                        .iter()
                        .map(|(id, card)| {
                            json!({
                                "name": id,
                                "preset": card.get("preset"),
                                "base_url_anthropic": card.get("base_url_anthropic"),
                                "base_url_openai_chat": card.get("base_url_openai_chat"),
                                "base_url_openai_responses": card.get("base_url_openai_responses"),
                            })
                        })
                        .collect()
                    })
                    .unwrap_or_default();
                (serde_json::Value::Array(list), true)
            }
            _ => (json!([]), false),
        };
    let mut router = serde_json::to_value(&state.router).unwrap_or_else(|_| json!({}));
    if let Some(obj) = router.as_object_mut() {
        obj.insert("providers".into(), providers_value.clone());
        obj.insert(
            "provider_count".into(),
            json!(providers_value.as_array().map(|a| a.len()).unwrap_or(0)),
        );
        obj.insert("providers_available".into(), json!(providers_available));
    }

    let data = json!({
        "card_config": card_config_info,
        "router": router,
    });
    Json(data).into_response()
}

/// GET /api/router — detailed provider status.
pub async fn router(State(state): State<WebUiState>) -> Response {
    Json(json!({ "router": state.router })).into_response()
}

/// GET /api/about — version info and system status.
pub async fn about(State(state): State<WebUiState>) -> Response {
    let data = json!({
        "uptime": format_uptime(state.started_at.elapsed()),
        "version": env!("CARGO_PKG_VERSION"),
        "rustc_version": env!("CARGO_PKG_RUST_VERSION"),
        "router_listen": state.router.listen,
        "provider_count": state.router.provider_count,
    });
    Json(data).into_response()
}

/// GET /api/agents — the agent catalog（workbench-agent-wire-fix 3.2），
/// agent 可用性的唯一真源：每个配置的 agent 一行（id/display/reachable/
/// cause?/version?）+ 内置内核 `"native"` 一行（可用性来自执行体自身的
/// 凭据上报，与 ACP 的 binary 探测语义不同源，如实区分）。`driver` 是
/// 配置层概念，不在响应中出现。
pub async fn agent_kinds(State(state): State<WebUiState>) -> Response {
    let mut agents = state.agent_kinds.agent_kinds().await;
    let native = state
        .backend
        .execution_bodies()
        .await
        .and_then(|bodies| bodies.into_iter().find(|b| b.name == "native"));
    agents.push(crate::agent_kinds::AgentKindInfo {
        id: "native".into(),
        display: "Native Kernel".into(),
        reachable: native.as_ref().map(|n| n.ok).unwrap_or(false),
        cause: native.and_then(|n| n.cause),
        version: None,
    });
    Json(json!({ "agents": agents })).into_response()
}

// ---- Auth endpoints（webui 登录鉴权，见 `auth` 模块） ----

/// GET /api/auth/me — 探明鉴权状态。始终 200：`enabled` = 服务端是否配置了
/// 凭据；`authenticated` = 当前请求是否携带有效会话；`username` 仅在已
/// 认证时给出。前端据此决定渲染登录页还是工作台。
pub async fn auth_me(State(state): State<WebUiState>, headers: axum::http::HeaderMap) -> Response {
    if !state.auth.enabled() {
        return Json(json!({ "enabled": false, "authenticated": false, "username": null }))
            .into_response();
    }
    let authenticated = match crate::server::extract_webui_session_cookie(&headers) {
        Some(sid) => state.auth.session_store.validate(&sid).await.is_ok(),
        None => false,
    };
    let username = if authenticated {
        state.auth.username()
    } else {
        None
    };
    Json(json!({
        "enabled": true,
        "authenticated": authenticated,
        "username": username,
    }))
    .into_response()
}

/// POST /api/auth/login — 单字段 `{ secret }`（token 或密码，服务端自动
/// 识别）或旧格式 `{ username, password }` → 会话 cookie。
/// 限速按来源 IP（SessionStore），失败统一 401（不区分凭据类型/对错）。
#[derive(Deserialize)]
pub struct AuthLoginForm {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
}

pub async fn auth_login(
    State(state): State<WebUiState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    Json(form): Json<AuthLoginForm>,
) -> Response {
    let client_ip = addr.ip().to_string();
    let (display_name, login) = if let Some(secret) = form.secret.as_deref() {
        let login = state.auth.login_secret(&client_ip, secret).await;
        (
            login
                .as_ref()
                .ok()
                .and_then(|_| state.auth.username())
                .unwrap_or_else(|| "admin".into()),
            login,
        )
    } else if let (Some(username), Some(password)) =
        (form.username.as_deref(), form.password.as_deref())
    {
        let username = username.to_string();
        (
            username.clone(),
            state.auth.login(&client_ip, &username, password).await,
        )
    } else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "login requires {\"secret\"} or {\"username\", \"password\"}.",
        );
    };
    match login {
        Ok(session_id) => {
            let cookie = format!(
                "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400",
                crate::auth::SESSION_COOKIE_NAME,
                session_id
            );
            (
                StatusCode::OK,
                [(axum::http::header::SET_COOKIE, cookie)],
                Json(json!({ "status": "ok", "username": display_name })),
            )
                .into_response()
        }
        Err(crate::auth::LoginError::RateLimited) => api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many login attempts. Try again later.",
        ),
        Err(_) => api_error(StatusCode::UNAUTHORIZED, "Invalid username or password."),
    }
}

/// POST /api/auth/logout — 结束当前会话并清 cookie。
pub async fn auth_logout(
    State(state): State<WebUiState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Some(session_id) = crate::server::extract_webui_session_cookie(&headers) {
        state.auth.logout(&session_id).await;
    }
    let cookie = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        crate::auth::SESSION_COOKIE_NAME
    );
    (
        StatusCode::OK,
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(json!({ "status": "ok" })),
    )
        .into_response()
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
    /// Project to bind, referenced by its stable id (`proj-<12hex>`,
    /// workbench-agent-wire-fix D2). `None` = inbox (no project). The raw
    /// directory path is resolved server-side from the registry — the path
    /// is never a wire identifier.
    #[serde(default)]
    pub project_id: Option<String>,
    /// 必填：目标 agent id——`[acp.agents.*]` 配置键名（如 `claudecode`、
    /// `codex`）或保留值 `"native"`（内置内核）。旧 `backend` 字段与
    /// driver 名不再是合法 wire（会话创建后 agent 不可变，故创建时必须
    /// 显式选定）。
    pub agent: String,
    /// Optional model id for the new session (add-acp-model-selection D3):
    /// applied after the session is established, before the first prompt.
    /// Only agents that expose a `model` config option honor it; for others
    /// the field is a silent no-op (the session uses its default model and
    /// no model UI is shown).
    #[serde(default)]
    pub model: Option<String>,
    /// Optional permission mode for the new session
    /// （add-agent-mode-selection）：控制面词汇 `ask`/`edit`/`allow`/`auto`
    /// （与节点链路 SessionMode 一致）。缺省/None = agent 默认行为（wire 不
    /// 携带 mode）。未知词汇 400 拒绝，不静默降级。只有能执行它的执行体
    /// （本机 claude、远端节点）会生效；其它 agent 接受但不生效（非致命，
    /// 同 model 语义）。
    #[serde(default)]
    pub mode: Option<String>,
}

/// （add-agent-mode-selection）创建与切换共用的 mode 词汇表。单一出处：
/// 与 `sebas-node-link::SessionMode` 的词汇一致（节点链路的既有门控词汇，
/// wire 上沿用，不另造一套）。
pub const SESSION_MODES: [&str; 4] = ["ask", "edit", "allow", "auto"];

/// 校验控制面 mode 词汇；未知值返回 `None`（调用方 400 如实拒绝）。
pub fn valid_session_mode(mode: &str) -> bool {
    SESSION_MODES.contains(&mode.trim().to_ascii_lowercase().as_str())
}

#[derive(Deserialize)]
pub struct SetModelRequest {
    pub model_id: String,
}

/// （add-agent-mode-selection）`POST /api/sessions/{key}/mode` 的请求体。
#[derive(Deserialize)]
pub struct SetSessionModeRequest {
    pub mode: String,
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
    // wire 词汇（workbench-agent-wire-fix D2）：agent 必填且只认 agent id
    // （配置键名 / "native"）；旧 `backend` 字段显式拒绝，帮助调用方迁移。
    if req.agent.is_empty() || req.agent.contains(':') {
        return api_error(
            StatusCode::BAD_REQUEST,
            "agent 字段必填，且必须是配置的 agent id 或 \"native\"",
        );
    }
    // project_id → 内部工作目录路径 + 所属执行节点（path/node 都不是 wire
    // 标识，解析发生在服务端）。8.1：项目决定节点——选项目即选节点，本机项目
    // 带 `"local"`（行为与今日逐字一致）；无项目会话 node = None，由核心落到
    // 配置的默认执行节点。
    let (project_dir, node) = match &req.project_id {
        Some(id) => {
            let entry = projects_from_backend(&state)
                .await
                .into_iter()
                .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id.as_str()));
            match entry {
                Some(p) => {
                    let Some(dir) = p.get("path").and_then(|v| v.as_str()).map(str::to_string)
                    else {
                        return api_error(StatusCode::BAD_REQUEST, format!("未知 project_id: {id}"));
                    };
                    let node = p
                        .get("node_id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(crate::projects::LOCAL_NODE_ID)
                        .to_string();
                    (Some(dir), Some(node))
                }
                None => return api_error(StatusCode::BAD_REQUEST, format!("未知 project_id: {id}")),
            }
        }
        None => (None, None),
    };
    // （add-agent-mode-selection）mode 词汇校验：未知值 400 如实拒绝，不
    // 静默降级（同"agent 必填"的 wire 严格性）。
    if let Some(mode) = &req.mode
        && !valid_session_mode(mode)
    {
        return api_error(
            StatusCode::BAD_REQUEST,
            format!(
                "mode 非法：{mode:?}（合法词汇 {}）",
                SESSION_MODES.join("/")
            ),
        );
    }
    let prompt = req.prompt.unwrap_or_default();
    // 0-turn 占位（P2）：无 prompt 时只建行、不 spawn 子进程、不把空串当
    // prompt 发给 agent（opencode 收到 `session/prompt ""` 会挂起）。agent/
    // model/project 记在 mapping 上，首条消息到达时才 spawn。远端节点上的
    // 占位被如实拒绝（远端会话必须由首条真实输入建立）。
    let key = if prompt.trim().is_empty() {
        match state
            .backend
            .create_placeholder(
                project_dir.clone(),
                &req.agent,
                req.model.clone(),
                req.mode.clone(),
                node.clone(),
            )
            .await
        {
            Ok(k) => k,
            Err(rej) => return rejection_response(rej),
        }
    } else {
        match state
            .backend
            .spawn_with(
                prompt,
                project_dir.clone(),
                &req.agent,
                req.model,
                req.mode,
                node.clone(),
            )
            .await
        {
            Ok(k) => k,
            Err(rej) => return rejection_response(rej),
        }
    };
    state.backend.set_focus(Some(key.clone())).await;
    // 2.6：项目级默认 agent——该项目下最近一次创建会话所用的 agent，
    // 下次在该项目创建会话时 composer 预选它。状态库优先，文件注册表回退。
    if let Some(id) = &req.project_id {
        let payload = json!({ "op": "set_default_agent", "id": id, "agent": req.agent });
        if state
            .backend
            .state_mutate("projects", payload)
            .await
            .is_err()
        {
            crate::projects::set_default_agent(id, &req.agent);
        }
    }
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
    if let Err(rej) = state
        .backend
        .set_session_model(session_key, req.model_id)
        .await
    {
        return rejection_response(rej);
    }
    (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response()
}

/// POST /api/sessions/{key}/mode — 切换会话的权限模式
/// （add-agent-mode-selection）。命令送达 = 200；执行体接受与否经事件流
/// 反馈（`ModeChanged` = 成功、快照 effective 更新；非终态 `Error` = 拒绝，
/// UI 显示错误、mode 不变）。远端节点会话经核心通道走节点链路
/// `SessionOp::SetMode`。
pub async fn set_session_mode(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
    Json(req): Json<SetSessionModeRequest>,
) -> Response {
    if !valid_session_mode(&req.mode) {
        return api_error(
            StatusCode::BAD_REQUEST,
            format!(
                "mode 非法：{:?}（合法词汇 {}）",
                req.mode,
                SESSION_MODES.join("/")
            ),
        );
    }
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };
    if let Err(rej) = state
        .backend
        .set_session_mode(session_key, req.mode.trim().to_ascii_lowercase())
        .await
    {
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

/// POST /api/sessions/{key}/cancel — interrupt the session's in-flight turn
/// over the core channel (workbench-interaction-polish 1.2，design D5)。
/// 会话与子进程存活（interrupt-and-heal）；排队提交不随取消丢弃。类型化
/// 拒绝：未知 key 404、空闲会话（无在飞 turn）409、core 不可达 503。
pub async fn cancel_session(State(state): State<WebUiState>, Path(key): Path<String>) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };
    match state.backend.cancel(session_key).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "cancelled" }))).into_response(),
        Err(rej) => rejection_response(rej),
    }
}

/// POST /api/sessions/{key}/close — kill and remove a session. Returns 200
/// with the new focused session key (or null); 404 if the key mapped to
/// nothing.
pub async fn close_session(State(state): State<WebUiState>, Path(key): Path<String>) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };

    // workbench-turn-queue 5.2：close 响应携带随之丢弃的未执行提交条数
    // （discarded_pending），关闭带队列的会话不再静默丢队。
    let report = match state.backend.close(session_key).await {
        Ok(r) => r,
        Err(rej) => return rejection_response(rej),
    };
    let focused = state.backend.focused().await;
    (
        StatusCode::OK,
        Json(json!({
            "status": "closed",
            "active_session_key": focused.as_ref().map(encode_session_key),
            "discarded_pending": report.discarded_pending,
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

/// POST /api/sessions/{key}/pending/{pending_id}/remove — 移除一个未开始的
/// 待生效提交（workbench-turn-queue 6.2，design D7）。成功返回操作后的全量
/// pending（客户端据此对账）；拒绝类型化（未知 id 404 / 已开始 409 / 越界
/// 400），绝不静默。
pub async fn pending_remove(
    State(state): State<WebUiState>,
    Path((key, pending_id)): Path<(String, u64)>,
) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };
    match state.backend.remove_pending(session_key, pending_id).await {
        Ok(pending) => Json(json!({ "status": "removed", "pending": pending })).into_response(),
        Err(rej) => rejection_response(rej),
    }
}

/// POST /api/sessions/{key}/pending/{pending_id}/move — 把一个未开始的提交
/// 重排到其处置组内 `to_index` 位置（workbench-turn-queue 6.2，design D7）。
/// 成功返回操作后的全量 pending；越优先/越界/已开始均为类型化 4xx。
#[derive(Deserialize)]
pub struct MovePendingRequest {
    pub to_index: usize,
}

pub async fn pending_move(
    State(state): State<WebUiState>,
    Path((key, pending_id)): Path<(String, u64)>,
    Json(req): Json<MovePendingRequest>,
) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };
    match state
        .backend
        .move_pending(session_key, pending_id, req.to_index)
        .await
    {
        Ok(pending) => Json(json!({ "status": "moved", "pending": pending })).into_response(),
        Err(rej) => rejection_response(rej),
    }
}

/// GET /api/fs/browse-dirs?path=...&root=... — list only subdirectories for
/// the directory tree picker. Root precedence: explicit `root` beats the
/// server-injected work root (add-webui-picker-workdir-start); with neither,
/// `fs::safe_path` errors. Path semantics (round-trip, bounds) live in
/// `fs::safe_path`.
pub async fn browse_dirs(
    axum::extract::Query(params): axum::extract::Query<crate::fs::BrowseParams>,
    State(state): State<WebUiState>,
) -> Response {
    let path = params.path.as_deref().unwrap_or("");
    match crate::fs::browse_dirs(
        path,
        params.root.as_deref(),
        state.work_root.as_deref(),
        &state.allowed_roots,
    ) {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => api_error(StatusCode::BAD_REQUEST, e),
    }
}

// ---- Project API endpoints ----

/// 从 backend 读取项目列表（DB 引擎 / core 通道）。backend 不可达时回退
/// 本地文件注册表（webui 进程独占视图，spec 未约束其降级语义）。
/// 返回 JSON 数组（ProjectRow / ProjectEntry 形状，前端兼容）。
///
/// add-remote-execution-node 8.1：**远端项目只在文件注册表里**。core 状态库的
/// `projects` 表（`ProjectRow`）没有节点列，`add_project(path,name,added_at)`
/// 也接不住节点维度——远端条目写进去就等于丢节点。因此这里把文件注册表里
/// **非本机**的条目并进列表（本机条目仍以状态库为准，避免复活已删除的项目）。
async fn projects_from_backend(state: &WebUiState) -> Vec<serde_json::Value> {
    let mut projects = if let Some(v) = state.backend.state_snapshot("projects").await {
        v.get("projects")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
    } else {
        eprintln!("[proj-debug] state_snapshot projects => None (file fallback)");
        // 回退：webui 本地文件注册表（list() 自带 id 回填）。
        return crate::projects::list()
            .into_iter()
            .map(|e| serde_json::to_value(&e).unwrap_or_default())
            .collect();
    };
    // 节点维度 + 稳定 id 回填（workbench-agent-wire-fix 2.4；8.1 起 id 按
    // `(节点, 路径)` 派生）：旧行没有 node_id → 本机；id 空 → 按 node 重算。
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for p in projects.iter_mut() {
        let node = p
            .get("node_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(crate::projects::LOCAL_NODE_ID)
            .to_string();
        let path = p.get("path").and_then(|v| v.as_str()).map(str::to_string);
        let needs_id = p
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::is_empty)
            .unwrap_or(true);
        if let Some(obj) = p.as_object_mut() {
            obj.entry("node_id")
                .or_insert_with(|| json!(node.clone()));
            if needs_id
                && let Some(path) = path.as_deref()
            {
                obj.insert(
                    "id".into(),
                    json!(crate::projects::project_id_for_on(&node, path)),
                );
            }
        }
        if let Some(path) = path {
            seen.insert((node, path));
        }
    }
    // 文件注册表里的远端条目并进来（状态库装不下节点维度）。
    for entry in crate::projects::list() {
        if entry.is_local() || seen.contains(&(entry.node_id.clone(), entry.path.clone())) {
            continue;
        }
        projects.push(serde_json::to_value(&entry).unwrap_or_default());
    }
    projects
}

/// GET /api/projects — list all registered projects（状态库优先，文件回退）。
pub async fn projects_list(State(state): State<WebUiState>) -> Response {
    let projects = projects_from_backend(&state).await;
    Json(json!({ "projects": projects })).into_response()
}

/// POST /api/projects — register a new project directory（状态库优先）。
///
/// add-remote-execution-node 8.1：body 可带 `node_id`。缺省 = 本机节点（隐式
/// 注册，行为与既有注册完全一致）。带远端节点时，路径可用性由**那台节点**判定
/// （经 `SessionBackend::check_node_path` → `SessionOp::CheckPath`），主控绝不做
/// 本地 `stat`。
pub async fn projects_add(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return api_error(StatusCode::BAD_REQUEST, "missing 'path' field"),
    };
    let node_id = body
        .get("node_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(crate::projects::LOCAL_NODE_ID);
    if node_id != crate::projects::LOCAL_NODE_ID {
        return projects_add_remote(&state, node_id, path).await;
    }
    // 本地校验：路径必须存在且是目录（canonicalize 后注册 canonical 路径）。
    // 范围判定先行于存在性判定（add-webui-allowed-roots）：fail-closed，
    // 越界与无法解析同罪，避免借 400 文案差异探测白名单外目录的存在性；
    // 错误信息不回显服务端解析后的路径（与 fs.rs 同一防泄露姿态）。
    let dir = std::path::Path::new(path);
    if !state.allowed_roots.is_empty()
        && !crate::fs::within_allowed_roots(dir, &state.allowed_roots)
    {
        return api_error(
            StatusCode::BAD_REQUEST,
            "路径超出允许范围: 不在 allowed_roots 白名单内",
        );
    }
    if !dir.exists() {
        return api_error(StatusCode::BAD_REQUEST, format!("路径不存在: {path}"));
    }
    if !dir.is_dir() {
        return api_error(StatusCode::BAD_REQUEST, format!("路径不是目录: {path}"));
    }
    // canonicalize_plain：解析真实路径但还原 Windows verbatim 前缀——
    // `\\?\C:\…` 直接入库会经 API 泄漏进 UI，且与请求侧普通路径判等永假。
    let canonical = match crate::fs::canonicalize_plain(dir) {
        Ok(c) => c,
        Err(_) => return api_error(StatusCode::BAD_REQUEST, format!("无法解析路径: {path}")),
    };
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string());
    // 重复检查按 `(节点, 路径)`：同一路径在另一台机器上是另一个项目。
    if projects_from_backend(&state)
        .await
        .iter()
        .any(|p| {
            p.get("path").and_then(|v| v.as_str()) == Some(canonical.as_str())
                && p.get("node_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(crate::projects::LOCAL_NODE_ID)
                    == crate::projects::LOCAL_NODE_ID
        })
    {
        return api_error(StatusCode::CONFLICT, format!("项目已注册: {path}"));
    }
    // backend 可用 → 状态库（响应不带 degraded）；不可用 → 文件注册表降级，
    // 响应携带 `degraded: {cause}`（harden-core-channel-deployment 4.2/D7：
    // 老前端忽略新字段，无破坏；两者皆失败的 503 语义不变）。cause 取
    // reachability 的如实上报。
    let mut degraded: Option<serde_json::Value> = None;
    if state
        .backend
        .state_mutate(
            "projects",
            json!({ "op": "add", "path": canonical.clone(), "name": name.clone() }),
        )
        .await
        .is_err()
    {
        // 状态库路径失败：先探因（核心不可达？），再落本地注册表。
        // A1.1：三类不可达的 cause 都如实透传（kind 判别由 payload 层承担，
        // 这里只需要人类可读原因）。
        let cause = match state.backend.reachability().await {
            crate::session_backend::Reachability::Reachable => "状态库写入失败".into(),
            crate::session_backend::Reachability::StartupFailed { cause }
            | crate::session_backend::Reachability::AuthRejected { cause }
            | crate::session_backend::Reachability::Disconnected { cause } => cause,
        };
        if crate::projects::add(&canonical).is_ok() {
            degraded = Some(json!({ "cause": cause }));
        } else {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "无法注册项目（状态库与本地均失败）",
            );
        }
    }
    // 返回新条目（从列表反查，保证与数据源一致；按 `(节点, 路径)` 命中本机那条）。
    let entry = projects_from_backend(&state)
        .await
        .into_iter()
        .find(|p| {
            p.get("path").and_then(|v| v.as_str()) == Some(canonical.as_str())
                && p.get("node_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(crate::projects::LOCAL_NODE_ID)
                    == crate::projects::LOCAL_NODE_ID
        })
        .unwrap_or_else(|| json!({ "path": canonical, "name": name }));
    let mut entry = entry;
    if let Some(d) = degraded {
        entry["degraded"] = d;
    }
    (StatusCode::CREATED, Json(entry)).into_response()
}

/// 远端项目注册（add-remote-execution-node 8.1）。
///
/// 顺序是刻意的：先确认节点**在线且已知**（不然「校验失败」会被误读成「路径不
/// 对」），再请节点判定路径。节点判定不了（链路未接入 / 离线）时如实拒绝，绝不
/// 回退本地 `stat`——那会把「主控上恰好同名」伪装成「节点上存在」。
async fn projects_add_remote(state: &WebUiState, node_id: &str, path: &str) -> Response {
    let path = path.trim();
    if path.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "远端项目的路径不能为空");
    }
    // 未知节点：如实拒绝并点名（不把路径判定当作节点不存在的借口）。
    match state.backend.nodes().await {
        Ok(nodes) => match nodes.iter().find(|n| n.id == node_id) {
            Some(n) if n.status == "online" => {}
            Some(n) => {
                return api_error(
                    StatusCode::CONFLICT,
                    format!(
                        "执行节点 {node_id} 当前不可用（{}），无法在该节点上校验或注册项目",
                        n.status
                    ),
                );
            }
            None => {
                return api_error(
                    StatusCode::BAD_REQUEST,
                    format!("未知执行节点: {node_id}"),
                );
            }
        },
        Err(cause) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                crate::projects::node_check_unavailable(node_id, path, &cause),
            );
        }
    }
    // 路径可用性由节点判定（`SessionOp::CheckPath`）。
    match state.backend.check_node_path(node_id, path).await {
        Ok(check) => {
            if let Err(msg) = crate::projects::validate_remote_path(
                node_id,
                path,
                crate::projects::NodePathCheck {
                    exists: check.exists,
                    is_dir: check.is_dir,
                },
            ) {
                return api_error(StatusCode::BAD_REQUEST, msg);
            }
        }
        Err(cause) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                crate::projects::node_check_unavailable(node_id, path, &cause),
            );
        }
    }
    // 重复检查按 `(节点, 路径)`。
    if projects_from_backend(state).await.iter().any(|p| {
        p.get("path").and_then(|v| v.as_str()) == Some(path)
            && p.get("node_id").and_then(|v| v.as_str()) == Some(node_id)
    }) {
        return api_error(
            StatusCode::CONFLICT,
            format!("项目已注册: {node_id}:{path}"),
        );
    }
    // 远端条目只能落 webui 文件注册表：core 状态库的 projects 表没有节点列
    // （`add_project` 接不住 node_id），写进去会变成一条本机幻影项目。
    match crate::projects::add_on(node_id, path) {
        Ok(entry) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(&entry).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => api_error(StatusCode::BAD_REQUEST, e),
    }
}

/// GET /api/nodes — 执行节点可用性（add-remote-execution-node 8.2）。
///
/// 本机节点**永远在列且在线**：它在回答这个请求，这就是它可达的证据。远端
/// 节点来自 core 的注册表（`NodeLinkOp::ListNodes`）——`remote_available` 明确
/// 区分「注册表可达但没节点」与「注册表不可得」，前端不许把后者说成前者。
pub async fn nodes(State(state): State<WebUiState>) -> Response {
    let mut nodes = vec![json!({
        "id": crate::projects::LOCAL_NODE_ID,
        "status": "online",
        "created_unix": 0,
        "local": true,
    })];
    let (remote_available, cause) = match state.backend.nodes().await {
        Ok(list) => {
            for n in list {
                if n.id == crate::projects::LOCAL_NODE_ID {
                    continue;
                }
                nodes.push(serde_json::to_value(&n).unwrap_or_default());
            }
            (true, serde_json::Value::Null)
        }
        Err(cause) => (false, json!(cause)),
    };
    Json(json!({
        "nodes": nodes,
        "remote_available": remote_available,
        "cause": cause,
    }))
    .into_response()
}

/// POST /api/projects/{id}/remove — unregister a project（状态库优先）。
/// 路径参数是稳定项目 id（workbench-agent-wire-fix 2.5）：先解析 id →
/// 内部 path（状态库以 path 为键），再走既有删除。
///
/// 8.1：远端条目只存在于文件注册表，且**绝不能按 path 删状态库**——同路径的
/// 本机项目会被顺手删掉（那正是 `(节点, 路径)` 要防的串味）。
///
/// rail-declutter-unread D5：项目下还有非归档会话时**拒绝移除**（typed
/// rejection，带会话数）——废除「存活会话迁移 Inbox」的承诺，操作员须先
/// 归档/关闭。
pub async fn projects_remove(State(state): State<WebUiState>, Path(id): Path<String>) -> Response {
    let id = match urlencoding::decode(&id) {
        Ok(d) => d.into_owned(),
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid id encoding"),
    };
    let Some(entry) = projects_from_backend(&state)
        .await
        .into_iter()
        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
    else {
        return api_error(StatusCode::NOT_FOUND, "project not found");
    };
    let node = entry
        .get("node_id")
        .and_then(|v| v.as_str())
        .unwrap_or(crate::projects::LOCAL_NODE_ID)
        .to_string();
    let Some(path) = entry
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        return api_error(StatusCode::NOT_FOUND, "project not found");
    };
    // rail-declutter-unread 1.3：非归档会话存在 → 拒绝（远端项目同样受此
    // 门禁约束——行投影的 project_id 就是 wire 标识，直接按行数）。会话数
    // 以当前后端快照为准（归档会话已离开快照，不计入）。
    let live_sessions = state
        .backend
        .snapshot()
        .await
        .iter()
        .filter(|info| crate::projects::project_id_for_session(info).as_deref() == Some(id.as_str()))
        .count();
    if live_sessions > 0 {
        return (
            StatusCode::CONFLICT,
            axum::Json(json!({
                "error": format!(
                    "项目下仍有 {live_sessions} 个未归档会话，请先归档或关闭它们再移除项目"
                ),
                "code": "project_has_live_sessions",
                "session_count": live_sessions,
            })),
        )
            .into_response();
    }
    // 远端：只处理文件注册表。
    if node != crate::projects::LOCAL_NODE_ID {
        return match crate::projects::remove_by_id(&id) {
            Ok(true) => Json(json!({ "status": "removed" })).into_response(),
            Ok(false) => api_error(StatusCode::NOT_FOUND, "project not found"),
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
    }
    // 先试状态库；失败（不存在或不可达）再试文件。
    match state
        .backend
        .state_mutate("projects", json!({ "op": "remove", "path": path }))
        .await
    {
        Ok(()) => Json(json!({ "status": "removed" })).into_response(),
        Err(e) => {
            if e.contains("不存在") {
                // 状态库没有 → 试文件注册表。
                match crate::projects::remove_by_id(&id) {
                    Ok(true) => Json(json!({ "status": "removed" })).into_response(),
                    Ok(false) => api_error(StatusCode::NOT_FOUND, "project not found"),
                    Err(_) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "remove failed"),
                }
            } else {
                // 状态库不可达 → 回退文件。
                match crate::projects::remove_by_id(&id) {
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
    /// Ordered list of stable project ids (workbench-agent-wire-fix 2.5);
    /// entries not listed are appended at the end (preserving their
    /// relative add-time order).
    pub ids: Vec<String>,
}

/// POST /api/projects/reorder — persist the user's rail ordering（状态库优先）。
pub async fn projects_reorder(
    State(state): State<WebUiState>,
    Json(req): Json<ReorderRequest>,
) -> Response {
    // 读当前列表 → 按新顺序重排（未知 id 落地为 add_time 顺序尾部）→ save。
    let mut projects = projects_from_backend(&state).await;
    let mut by_id: std::collections::HashMap<String, serde_json::Value> = projects
        .drain(..)
        .map(|p| {
            let id = p
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            (id, p)
        })
        .collect();
    let mut next: Vec<serde_json::Value> = Vec::with_capacity(req.ids.len());
    let mut seen = std::collections::HashSet::new();
    for id in &req.ids {
        if seen.insert(id.clone())
            && let Some(entry) = by_id.remove(id)
        {
            next.push(entry);
        }
    }
    // 未提及的项目追加尾部（按 added_at 稳定）。
    let mut tail: Vec<(i64, serde_json::Value)> = by_id
        .into_values()
        .map(|v| {
            let t = v.get("added_at").and_then(|x| x.as_i64()).unwrap_or(0);
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
    //
    // 8.1：只把**本机**条目发给状态库。core 的 projects 表没有节点列
    // （`ProjectRow` / `add_project` 都装不下 node_id），把远端条目一起 save 会
    // 把它们落成同路径的**本机**项目——一条凭空出现的本地条目。远端顺序改由
    // 文件注册表承载（它本来就是远端条目的家）。
    let local_entries: Vec<serde_json::Value> = next
        .iter()
        .filter(|v| {
            v.get("node_id").and_then(|x| x.as_str()).unwrap_or(crate::projects::LOCAL_NODE_ID)
                == crate::projects::LOCAL_NODE_ID
        })
        .cloned()
        .collect();
    let remote_ids: Vec<String> = next
        .iter()
        .filter(|v| {
            v.get("node_id").and_then(|x| x.as_str()).unwrap_or(crate::projects::LOCAL_NODE_ID)
                != crate::projects::LOCAL_NODE_ID
        })
        .filter_map(|v| v.get("id").and_then(|x| x.as_str()).map(str::to_string))
        .collect();
    let via_backend = state
        .backend
        .state_mutate(
            "projects",
            json!({ "op": "save", "projects": local_entries.clone() }),
        )
        .await
        .is_ok();
    if !remote_ids.is_empty() {
        // 顺序落文件注册表；失败不改变响应（顺序是呈现细节，不静默改状态）。
        if let Err(e) = crate::projects::reorder(&remote_ids) {
            tracing::warn!(error = %e, "远端项目顺序落文件注册表失败");
        }
    }
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

/// GET /api/projects/{id}/branch — current git branch (TTL-cached server-side)。
/// Git 探测是本地文件系统操作；backend 不可达时列表来自文件回退，语义一致。
/// 路径参数是稳定项目 id（workbench-agent-wire-fix 2.5）。
pub async fn projects_branch(State(state): State<WebUiState>, Path(id): Path<String>) -> Response {
    let id = match urlencoding::decode(&id) {
        Ok(d) => d.into_owned(),
        Err(_) => return api_error(StatusCode::BAD_REQUEST, "invalid id encoding"),
    };
    let projects = projects_from_backend(&state).await;
    let entry = projects
        .iter()
        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(id.as_str()));
    let Some(entry) = entry else {
        return api_error(StatusCode::NOT_FOUND, "project not found");
    };
    let Some(project_path) = entry
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        return api_error(StatusCode::NOT_FOUND, "project not found");
    };
    let node_id = entry
        .get("node_id")
        .and_then(|v| v.as_str())
        .unwrap_or(crate::projects::LOCAL_NODE_ID)
        .to_string();
    // 8.2：远端项目不做本地 git 探测（路径在那台机器上），其「可用」等于
    // 「节点在线」——对本地路径 stat 一次只会得到一个与本机无关的 false。
    if node_id != crate::projects::LOCAL_NODE_ID {
        let accessible = matches!(
            state.backend.nodes().await,
            Ok(nodes) if nodes.iter().any(|n| n.id == node_id && n.status == "online")
        );
        return Json(json!({
            "project_id": id,
            "branch": serde_json::Value::Null,
            "accessible": accessible,
            "node_id": node_id,
        }))
        .into_response();
    }
    let accessible = crate::projects::is_accessible(&project_path);
    // TTL 缓存：branch_at 距今 < 30s 用缓存。
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let branch_at = entry.get("branch_at").and_then(|v| v.as_i64()).unwrap_or(0);
    let cached_branch = entry
        .get("branch")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let branch = if branch_at != 0 && now.saturating_sub(branch_at) < 30 && cached_branch.is_some()
    {
        cached_branch
    } else {
        let fresh = crate::projects::probe_git_branch(std::path::Path::new(&project_path));
        // 简化：返回探测值，不强制写回（分支缓存不是共享真源）。
        fresh
    };
    Json(json!({
        "project_id": id,
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
pub async fn archive_session(State(state): State<WebUiState>, Path(key): Path<String>) -> Response {
    let session_key = match decode_session_key(&key) {
        Some(k) => k,
        None => return api_error(StatusCode::BAD_REQUEST, "Invalid session key"),
    };

    // Look up the session to get its project_dir and label.
    let infos = state.backend.snapshot().await;
    let info = match infos
        .iter()
        .find(|i| i.channel == session_key.channel.as_str() && i.key == session_key.reference)
    {
        Some(i) => i,
        None => return api_error(StatusCode::NOT_FOUND, "Session not found"),
    };

    let project_path = info.project_dir.clone().unwrap_or_default();
    let label = info.user_prompt.clone().unwrap_or_else(|| {
        info.session_id
            .clone()
            .unwrap_or_else(|| "unnamed".to_string())
    });

    // Close the session first (kills child if active).
    if let Err(_rej) = state.backend.close(session_key).await {
        // If close fails (unknown, unavailable), we still proceed with the archive.
    }

    match crate::archive::archive_session(&key, &project_path, &label, state.archive_retention_days)
    {
        Ok(entry) => (
            StatusCode::OK,
            Json(json!({ "status": "archived", "entry": entry })),
        )
            .into_response(),
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
            (
                StatusCode::OK,
                Json(json!({ "status": "restored", "entry": entry })),
            )
                .into_response()
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
        SessionEvent::Removed { channel, key } => Some(WebUiEvent::SessionRemoved {
            session_id: encode_channel_key(&channel, &key),
        }),
        // workbench-turn-queue 5.2/7.3：会话终结时未执行的待生效提交——
        // 逐条标注转发给前端，供一次性「未执行」提示。
        SessionEvent::PendingDropped {
            channel,
            key,
            dropped,
        } => Some(WebUiEvent::SessionPendingDropped {
            session_id: encode_channel_key(&channel, &key),
            dropped: dropped
                .into_iter()
                .map(|d| crate::events::PendingSubmissionView {
                    id: d.id,
                    text: d.text,
                    disposition: d.disposition,
                    priority: d.priority,
                })
                .collect(),
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
        api_error(
            StatusCode::NOT_FOUND,
            "no pending permission request with that id",
        )
    }
}
