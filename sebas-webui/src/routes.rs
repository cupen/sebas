//! Route handlers: pure helpers for the JSON API + the router BFF proxies.

use crate::models::{SessionRow, SessionStatus};
use crate::server::WebUiState;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use sebas_channels::key::ChannelKey;
use sebas_dispatch::SessionInfo;

// ---- Helper functions ----

/// Build `SessionRow`s from backend session info, returning counts. Marks
/// each row with `is_active` so the client can render the active indicator.
pub(crate) fn build_session_rows(
    infos: &[SessionInfo],
    focused: Option<&ChannelKey>,
) -> (Vec<SessionRow>, usize, usize, usize) {
    let mut active = 0usize;
    let mut dormant = 0usize;
    let mut spawning = 0usize;

    let mut rows: Vec<SessionRow> = infos
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
            let is_active = focused
                .map(|a| a.channel.as_str() == info.channel && a.reference == info.key)
                .unwrap_or(false);
            let derived =
                SessionStatus::derive(status, info.phase.as_deref().unwrap_or(""));
            SessionRow {
                project_dir: info.project_dir.clone(),
                prompt_preview: info.user_prompt.clone(),
                current_model: info.current_model.clone(),
                available_models: info.available_models.clone(),
                agent_kind: info.agent_kind.clone(),
                encoded_key: encode_channel_key(&info.channel, &info.key),
                channel: info.channel.clone(),
                reference: info.key.clone(),
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
                last_active_unix: info.last_active_unix,
                is_active,
            }
        })
        .collect();

    // Sort: focused first, then by most-recent activity. Activity compares
    // the underlying unix timestamps — the rendered relative-time string is
    // for humans, and text-comparing "20694d ago" > "0s ago" inverted the
    // intended order.
    rows.sort_by(|a, b| {
        b.is_active
            .cmp(&a.is_active)
            .then_with(|| b.last_active_unix.cmp(&a.last_active_unix))
    });
    (rows, active, dormant, spawning)
}

/// Compact summary used by the dashboard's focused-session banner.
pub(crate) fn session_summary(info: &SessionInfo) -> serde_json::Value {
    let derived = SessionStatus::derive(&info.status, info.phase.as_deref().unwrap_or(""));
    serde_json::json!({
        "channel": info.channel,
        "reference": info.key,
        "session_id": info.session_id,
        "status": info.status,
        "status_label": derived.label(),
        "status_slug": derived.slug(),
        "status_glyph": derived.glyph(),
        "encoded_key": encode_channel_key(&info.channel, &info.key),
        "current_model": info.current_model,
        "available_models": info.available_models,
        "agent_kind": info.agent_kind,
    })
}

/// Encode a (channel, reference) pair for use in URLs.
pub(crate) fn encode_channel_key(channel: &str, reference: &str) -> String {
    let raw = format!("{channel}\0{reference}");
    urlencoding::encode(&raw).into_owned()
}

/// Encode a ChannelKey for use in URLs.
pub(crate) fn encode_session_key(key: &ChannelKey) -> String {
    encode_channel_key(key.channel.as_str(), &key.reference)
}

/// Decode a URL-encoded ChannelKey.
pub(crate) fn decode_session_key(encoded: &str) -> Option<ChannelKey> {
    let decoded = urlencoding::decode(encoded).ok()?;
    let (channel, reference) = decoded.split_once('\0')?;
    Some(ChannelKey::new(channel, reference))
}

/// Format a unix timestamp as a relative time string.
pub(crate) fn format_relative_time(unix_ts: i64) -> String {
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
pub(crate) fn format_uptime(d: std::time::Duration) -> String {
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

// ---- router BFF mutation 路由（Task 6.3）：全部转发 admin API ----

fn router_client_of(state: &WebUiState) -> crate::router_client::RouterClient {
    let listen = state.router.listen.clone().unwrap_or_default();
    crate::router_client::RouterClient::new(&listen)
}

/// 无 listen 或无 secret → 503（mutation 面不可用；只读 loopback 仍可看页）。
fn mutation_available(client: &crate::router_client::RouterClient, state: &WebUiState) -> bool {
    state.router.listen.is_some() && client.has_secret()
}

pub async fn router_api_provider_create(
    State(state): State<WebUiState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let client = router_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.create_provider(&body).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn router_api_provider_update(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let client = router_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.update_provider(&name, &body).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn router_api_provider_delete(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> axum::response::Response {
    let client = router_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.delete_provider(&name).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn router_api_provider_probe(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> axum::response::Response {
    let client = router_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.probe_provider(&name, true).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn router_api_alias_create(
    State(state): State<WebUiState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let client = router_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.create_alias(&body).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn router_api_alias_update(
    State(state): State<WebUiState>,
    Path(alias): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let client = router_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.update_alias(&alias, &body).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn router_api_alias_delete(
    State(state): State<WebUiState>,
    Path(alias): Path<String>,
) -> axum::response::Response {
    let client = router_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.delete_alias(&alias).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

pub async fn router_api_reload(State(state): State<WebUiState>) -> axum::response::Response {
    let client = router_client_of(&state);
    if !mutation_available(&client, &state) {
        return err_503_no_secret();
    }
    match client.reload().await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => err_json(e.0),
    }
}

/// router mutation 守卫（Task 6.3，语义与 admin_mutation_guard 一致但
/// 不依赖 AdminState）：POST-only（405）+ loopback origin 检查（403）。
pub async fn router_mutation_guard(
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
        .unwrap_or(true); // 无 origin（CLI/curl）放行——router 侧另有 bearer 鉴权
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
        axum::Json(serde_json::json!({"error": "router admin mutation 不可用（未配置 SEBAS_CONTROL_SECRET 或 router 未启动）"})),
    )
        .into_response()
}

/// Health probe: `GET /health`.
pub async fn health() -> &'static str {
    "ok\n"
}
