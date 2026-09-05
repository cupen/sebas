//! Integration test for the WebSocket realtime channel `/ws`.
//!
//! Binds a real listener on an ephemeral port, drives the server with
//! tokio, connects a `tokio-tungstenite` client, and asserts that session
//! mutations broadcast the tagged JSON events over the wire.

use sebas_feishu::cards::CardConfig;
use futures_util::{SinkExt, StreamExt};
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use sebas_webui::models::RouterInfo;
use sebas_webui::build_router;

async fn spawn_server() -> (String, tokio::sync::mpsc::Receiver<sebas_dispatch::engine::Out>) {
    let map = SessionMap::new();
    let (router, rx) = DispatchHandle::new(map);
    let backend: Arc<dyn sebas_webui::SessionBackend> = Arc::new(
        sebas_webui::session_backend::InProcessBackend::new(router.clone()),
    );
    let app = build_router(backend, RouterInfo::default(), CardConfig::default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    // `http_base` for plain HTTP calls; `ws_url` builds ws:// URLs from it
    // (tungstenite requires the ws scheme explicitly).
    (format!("http://{addr}"), rx)
}

fn ws_url(base: &str) -> tokio_tungstenite::tungstenite::http::Request<()> {
    format!("{}/ws", base.replacen("http://", "ws://", 1))
        .into_client_request()
        .expect("ws request")
}

/// Read the next WS text frame as JSON, with a hard timeout so a bug
/// surfaces as a test failure rather than a hang.
async fn next_event(
    ws: &mut (impl StreamExt<Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>>
              + Unpin),
) -> Value {
    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("timed out waiting for a WebSocket event")
        .expect("websocket stream ended")
        .expect("websocket error");
    match msg {
        tokio_tungstenite::tungstenite::Message::Text(text) => {
            serde_json::from_str(&text).expect("event must be tagged JSON")
        }
        // Pings and other control frames are skipped; the next read gets
        // the event.
        _ => Box::pin(next_event(ws)).await,
    }
}

#[tokio::test]
async fn create_session_broadcasts_over_websocket() {
    let (base, _rx) = spawn_server().await;

    // Connect a WebSocket client.
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url(&base))
        .await
        .expect("ws connect failed");
    let (mut writer, mut reader) = ws_stream.split();

    // Create a session over the JSON API; the client must observe it live.
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{base}/api/sessions"))
        .json(&serde_json::json!({ "prompt": "hello" }))
        .send()
        .await
        .expect("create request failed");
    assert_eq!(resp.status(), 201);
    let created: Value = resp.json().await.unwrap();
    let key = created["key"].as_str().unwrap().to_string();

    let event = next_event(&mut reader).await;
    assert_eq!(event["type"], "session.created", "event: {event}");
    assert_eq!(event["session_id"], key.as_str());
    // Writer is kept so the connection stays open for the assertions above.
    let _ = writer.send(tokio_tungstenite::tungstenite::Message::Close(None)).await;
}

#[tokio::test]
async fn one_client_disconnecting_does_not_starve_others() {
    let (base, _rx) = spawn_server().await;

    // Two clients; then the first disconnects.
    let (ws1, _) = tokio_tungstenite::connect_async(ws_url(&base)).await.unwrap();
    let (ws2, _) = tokio_tungstenite::connect_async(ws_url(&base)).await.unwrap();
    let (_w2, mut r2) = ws2.split();

    // Client 1 hangs up.
    {
        let (mut w1, mut r1) = ws1.split();
        let _ = w1.send(tokio_tungstenite::tungstenite::Message::Close(None)).await;
        let _ = r1.next().await; // drain the close echo
    }

    // A session close must still reach client 2. Seed a dormant session via
    // the API path: create (spawning) then close it.
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{base}/api/sessions"))
        .json(&serde_json::json!({ "prompt": "doomed" }))
        .send()
        .await
        .unwrap();
    let created: Value = resp.json().await.unwrap();
    let key = created["key"].as_str().unwrap().to_string();

    let resp = http
        .post(format!("{base}/api/sessions/{key}/close"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Client 2 receives both the created and removed events in order.
    let created_ev = next_event(&mut r2).await;
    assert_eq!(created_ev["type"], "session.created");
    let removed_ev = next_event(&mut r2).await;
    assert_eq!(removed_ev["type"], "session.removed", "event: {removed_ev}");
}
