//! Route handlers: key/element/time helpers shared by the JSON API and the
//! backend implementations, plus the gateway BFF proxies.

use crate::models::CardElementView;
use crate::server::WebUiState;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use sebas_feishu::cards::CardElement;
use sebas_feishu::events::SessionKey;

// ---- Helper functions ----

/// Encode a SessionKey for use in URLs.
pub fn encode_session_key(key: &SessionKey) -> String {
    let raw = format!(
        "{}\0{}",
        key.chat_id,
        key.thread_id.as_deref().unwrap_or("")
    );
    urlencoding::encode(&raw).into_owned()
}

/// Decode a URL-encoded SessionKey.
pub fn decode_session_key(encoded: &str) -> Option<SessionKey> {
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
pub fn card_element_to_view(el: &CardElement) -> CardElementView {
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
pub fn format_relative_time(unix_ts: i64) -> String {
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

/// Health probe: `GET /health`.
pub async fn health() -> &'static str {
    "ok\n"
}
