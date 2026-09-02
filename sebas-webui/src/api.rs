//! JSON API for the WebUI frontend: HTTP endpoints under `/api/*` and the
//! WebSocket realtime channel at `/ws`.
//!
//! This module is the client-agnostic contract documented in the
//! `webui-api` capability: every SPA view (and any future local client)
//! consumes it. All session data flows through the [`SessionBackend`] seam —
//! the handlers never know whether the core is in-process or across a
//! socket. Typed backend rejections map onto the API's status codes:
//! `UnknownSession` → 404, `CoreUnreachable` → 503, `InvalidRequest` → 400.

use crate::backend::{CloseOutcome, Reachability, Rejection};
use crate::models::SessionRow;
use crate::routes::{decode_session_key, encode_session_key, format_uptime};
use crate::server::WebUiState;
use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
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

/// Map a typed backend rejection onto the API's status codes.
fn rejection_response(rejection: Rejection) -> Response {
    match rejection {
        Rejection::UnknownSession { key } => {
            api_error(StatusCode::NOT_FOUND, format!("Session not found: {key}"))
        }
        Rejection::CoreUnreachable { cause } => {
            api_error(StatusCode::SERVICE_UNAVAILABLE, cause)
        }
        Rejection::InvalidRequest { reason } => api_error(StatusCode::BAD_REQUEST, reason),
    }
}

/// Snapshot rows with the WebUI's focus pointer applied, focused first.
/// Rows leave the backend recency-sorted with `is_active: false`; a stable
/// sort by focus preserves recency within each group.
async fn focused_rows(state: &WebUiState) -> Vec<SessionRow> {
    let mut rows = state.backend.snapshot().await;
    let focus = state.focus.read().await.clone();
    for row in &mut rows {
        row.is_active = focus.as_deref() == Some(row.encoded_key.as_str());
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.is_active));
    rows
}

/// Session counts derived from the backend's own status projection.
fn counts(rows: &[SessionRow]) -> (usize, usize, usize) {
    let count = |want: &str| rows.iter().filter(|r| r.status == want).count();
    (count("active"), count("dormant"), count("spawning"))
}

/// The path segment is percent-decoded by axum; the canonical wire form is
/// its re-encoded value — exactly what rows and WS events carry.
fn wire_key(path_key: &str) -> Option<String> {
    let decoded = decode_session_key(path_key)?;
    Some(encode_session_key(&decoded))
}

// ---- Read endpoints ----

/// GET /api/summary — dashboard overview: counts, uptime, focused session,
/// and the backend's honest-degradation report.
pub async fn summary(State(state): State<WebUiState>) -> Response {
    let rows = focused_rows(&state).await;
    let (active, dormant, spawning) = counts(&rows);
    let focus = state.focus.read().await.clone();
    let active_row = rows.iter().find(|r| r.is_active);
    let (core_connected, core_cause) = match state.backend.reachability() {
        Reachability::Connected => (true, None),
        Reachability::Unreachable { cause } => (false, Some(cause)),
    };

    let data = json!({
        "active_count": active,
        "dormant_count": dormant,
        "spawning_count": spawning,
        "total_sessions": active + dormant + spawning,
        "uptime": format_uptime(state.started_at.elapsed()),
        "core_connected": core_connected,
        "core_cause": core_cause,
        "recent_sessions": rows,
        "active_session": active_row,
        "active_session_key": focus,
    });
    Json(data).into_response()
}

/// GET /api/sessions — every session row plus counts, focused first.
pub async fn sessions_list(State(state): State<WebUiState>) -> Response {
    let rows = focused_rows(&state).await;
    let (active, dormant, spawning) = counts(&rows);
    let focus = state.focus.read().await.clone();

    let data = json!({
        "recent_sessions": rows,
        "active_count": active,
        "dormant_count": dormant,
        "spawning_count": spawning,
        "total_sessions": active + dormant + spawning,
        "active_session_key": focus,
    });
    Json(data).into_response()
}

/// GET /api/sessions/{key} — session detail. A successful read focuses the
/// session (a display pointer only; it never changes message routing).
pub async fn session_detail(State(state): State<WebUiState>, Path(key): Path<String>) -> Response {
    let Some(wire) = wire_key(&key) else {
        return api_error(StatusCode::BAD_REQUEST, "Invalid session key");
    };
    let rows = state.backend.snapshot().await;
    let Some(row) = rows.iter().find(|r| r.encoded_key == wire).cloned() else {
        return api_error(StatusCode::NOT_FOUND, "Session not found");
    };

    // Reading the detail focuses this session.
    *state.focus.write().await = Some(wire.clone());

    let turns = match state.backend.turns(&wire, 0).await {
        Ok(content) => content,
        Err(rejection) => return rejection_response(rejection),
    };
    // Turn items and the former card-element view share one wire shape:
    // `{ element_type, content }`.
    let body: Vec<serde_json::Value> = turns
        .items
        .iter()
        .map(|item| json!({ "element_type": item.kind, "content": item.content }))
        .collect();

    let data = json!({
        "chat_id": row.chat_id,
        "thread_id": row.thread_id,
        "session_id": row.session_id,
        "status": row.status,
        "status_label": row.status_label,
        "status_slug": row.status_slug,
        "status_glyph": row.status_glyph,
        "user_prompt": row.prompt_preview,
        "body": body,
        // Feishu-era metadata (the root card's message id): not part of the
        // session channel, so the field stays for client compatibility but
        // no longer carries a value.
        "msg_id": serde_json::Value::Null,
        "last_active": row.last_active,
        "encoded_key": row.encoded_key,
    });
    Json(data).into_response()
}

/// GET /api/settings — card config (via the backend) and basic gateway info.
pub async fn settings(State(state): State<WebUiState>) -> Response {
    let card_config = state.backend.card_config().await;
    let data = json!({
        "card_config": card_config,
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

// ---- Mutation endpoints ----

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub prompt: String,
}

#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub message: String,
}

/// POST /api/sessions — create a session from a prompt. Returns 201 with
/// the encoded key; the session is still spawning, so the client can
/// navigate to its detail view immediately. Fails with 503 while the core
/// is unreachable — nothing is mutated.
pub async fn create_session(
    State(state): State<WebUiState>,
    Json(req): Json<CreateSessionRequest>,
) -> Response {
    let key = match state.backend.spawn(req.prompt, None).await {
        Ok(key) => key,
        Err(rejection) => return rejection_response(rejection),
    };
    *state.focus.write().await = Some(key.clone());
    (StatusCode::CREATED, Json(json!({ "key": key }))).into_response()
}

/// POST /api/sessions/{key}/message — send a message into the session.
/// An unknown key is a 404: the backend must not silently accept into the
/// void.
pub async fn send_message(
    State(state): State<WebUiState>,
    Path(key): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Response {
    let Some(wire) = wire_key(&key) else {
        return api_error(StatusCode::BAD_REQUEST, "Invalid session key");
    };
    match state.backend.message(&wire, req.message).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response(),
        Err(rejection) => rejection_response(rejection),
    }
}

/// POST /api/sessions/{key}/close — kill and remove a session. Returns 200
/// with the focused key (cleared if the closed session had it); 404 if the
/// key mapped to nothing.
pub async fn close_session(State(state): State<WebUiState>, Path(key): Path<String>) -> Response {
    let Some(wire) = wire_key(&key) else {
        return api_error(StatusCode::BAD_REQUEST, "Invalid session key");
    };
    match state.backend.close(&wire).await {
        Ok(CloseOutcome::NotFound) => api_error(StatusCode::NOT_FOUND, "Session not found"),
        Ok(CloseOutcome::Closed) => {
            let mut focus = state.focus.write().await;
            if focus.as_deref() == Some(wire.as_str()) {
                *focus = None;
            }
            let active_session_key = focus.clone();
            (
                StatusCode::OK,
                Json(json!({
                    "status": "closed",
                    "active_session_key": active_session_key,
                })),
            )
                .into_response()
        }
        Err(rejection) => rejection_response(rejection),
    }
}

/// POST /api/sessions/{key}/switch — move the focused-session pointer.
/// Returns the client route target so the SPA can navigate; 404 for an
/// unknown key so the client does not navigate to a dead view.
///
/// The redirect re-encodes the key so the client receives a usable URL
/// segment, not one containing a raw NUL byte.
pub async fn switch_session(State(state): State<WebUiState>, Path(key): Path<String>) -> Response {
    let Some(wire) = wire_key(&key) else {
        return api_error(StatusCode::BAD_REQUEST, "Invalid session key");
    };
    let rows = state.backend.snapshot().await;
    if !rows.iter().any(|r| r.encoded_key == wire) {
        return api_error(StatusCode::NOT_FOUND, "Session not found");
    }

    *state.focus.write().await = Some(wire.clone());
    (
        StatusCode::OK,
        Json(json!({
            "status": "switched",
            "redirect": format!("/sessions/{}", wire),
            "active_session_key": wire,
        })),
    )
        .into_response()
}

// ---- WebSocket realtime channel ----

/// GET /ws — upgrade to a WebSocket and stream the backend's session events
/// as self-describing JSON frames. The backend subscription is the only
/// event source: WebUI-local publishes no longer exist, so events from any
/// origin (web mutations, Feishu chat, watchdog) reach every client
/// identically. Unknown event types are forward-compatible additions
/// clients must tolerate.
pub async fn ws_handler(State(state): State<WebUiState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| ws_connection(state, socket))
}

/// Per-connection loop: forwards backend events, answers the protocol
/// keep-alive with server pings, and drains client frames (only Close is
/// meaningful) until either side hangs up.
async fn ws_connection(state: WebUiState, socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.backend.subscribe();
    let mut ping = tokio::time::interval(WS_PING_INTERVAL);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        let Ok(text) = serde_json::to_string(&event) else {
                            continue;
                        };
                        if sender.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    // A slow client lagged the broadcast: skip what it missed
                    // rather than killing the connection.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = ping.tick() => {
                if sender.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
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
