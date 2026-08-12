//! SSE event stream for real-time WebUI updates.

use crate::server::WebUiState;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive};
use axum::response::{IntoResponse, Sse};
use serde::Serialize;
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

/// Events that the WebUI can push to connected clients.
#[derive(Debug, Clone, Serialize)]
pub enum WebUiEvent {
    /// A new session was created.
    SessionCreated { session_id: String },
    /// A session's state was updated.
    SessionUpdated { session_id: String, status: String },
    /// A session was removed.
    SessionRemoved { session_id: String },
    /// A card was updated (new content streamed).
    CardUpdated { session_id: String },
    /// Configuration was updated.
    ConfigUpdated,
}

/// SSE handler: subscribes to the broadcast channel and streams events.
pub async fn event_stream(State(state): State<WebUiState>) -> impl IntoResponse {
    let rx = state.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        match result {
            Ok(event) => {
                let data = serde_json::to_string(&event).unwrap_or_default();
                Some(Ok::<_, Infallible>(Event::default().event("update").data(data)))
            }
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}