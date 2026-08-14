//! Route handlers for the WebUI dashboard.

use crate::models::{
    CardConfigInfo, CardElementView, DashboardData, SessionRow,
};
use crate::server::WebUiState;
use crate::sse::WebUiEvent;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Json};
use axum::Form;
use feishu::cards::CardElement;
use feishu::events::SessionKey;
use router::card_state::CardState;
use router::router::CloseOutcome;
use router::state::{Mapping, MappingState};
use serde::Deserialize;

/// Dashboard overview: session counts, recent sessions, uptime, active session.
pub async fn dashboard(
    State(state): State<WebUiState>,
) -> impl IntoResponse {
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
pub async fn session_list(
    State(state): State<WebUiState>,
) -> impl IntoResponse {
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
pub async fn session_list_partial(
    State(state): State<WebUiState>,
) -> impl IntoResponse {
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
                let body: Vec<CardElementView> =
                    st.body.iter().map(card_element_to_view).collect();
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
pub async fn settings(
    State(state): State<WebUiState>,
) -> impl IntoResponse {
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

/// Gateway page: detailed provider status.
pub async fn gateway_page(
    State(state): State<WebUiState>,
) -> impl IntoResponse {
    let data = serde_json::json!({
        "gateway": state.gateway,
    });
    render_template(&state, "gateway.html", "gateway", &data).await
}

/// About page: version info and system status.
pub async fn about(
    State(state): State<WebUiState>,
) -> impl IntoResponse {
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

    let outcome = state
        .router
        .web_close_session(session_key.clone())
        .await;
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
            SessionRow {
                encoded_key: encode_session_key(key),
                chat_id: key.chat_id.clone(),
                thread_id: key.thread_id.clone(),
                session_id: mapping.session_id().map(|s| s.to_string()),
                status,
                phase,
                last_active: format_relative_time(mapping.last_active_unix),
                is_active,
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
fn session_summary(
    key: &SessionKey,
    sessions: &[(SessionKey, Mapping)],
) -> serde_json::Value {
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