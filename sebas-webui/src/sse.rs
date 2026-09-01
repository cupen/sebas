//! SSE event stream for real-time WebUI updates.
//!
//! Subscribes to the backend's session event stream (task 3.4) instead of a
//! WebUI-local broadcast: events originate at the session authority (the
//! router inside the core), so the console reacts to every session change
//! whether it was driven from this WebUI, the Feishu bot, or another client.
//!
//! Wire contract: any session event is surfaced as an `update` SSE event.
//! The frontend debounces `update` into a `/sessions/partial` refetch, so
//! the payload is informational, not authoritative.

use crate::server::WebUiState;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive};
use axum::response::{IntoResponse, Sse};
use std::convert::Infallible;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

/// SSE handler: subscribes to the backend's session event stream and
/// forwards every event (including `Resync` after a reconnect) as an
/// `update` event. Lagged receivers are dropped from the stream silently —
/// the browser's own EventSource reconnect plus the debounced partial
/// refetch converges the view.
pub async fn event_stream(State(state): State<WebUiState>) -> impl IntoResponse {
    let rx = state.backend.subscribe();
    let stream = BroadcastStream::new(rx).map(|result| match result {
        Ok(event) => {
            let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
            Ok::<_, Infallible>(Event::default().event("update").data(data))
        }
        // Lagged / closed: emit nothing; the client keeps its connection and
        // the next event (or a page navigation) re-renders from the snapshot.
        Err(_) => Ok(Event::default().event("update").data("{\"resync\":true}")),
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
