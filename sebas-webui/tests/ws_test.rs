//! Integration test for the WebSocket realtime channel `/ws`.
//!
//! Binds a real listener on an ephemeral port, drives the server with
//! tokio, connects a `tokio-tungstenite` client, and asserts that backend
//! events broadcast over the wire — whether triggered through the API or
//! arising inside the backend itself (e.g. from the Feishu chat side).

use futures_util::{SinkExt, StreamExt};
use sebas_webui::backend::FakeBackend;
use sebas_webui::events::WebUiEvent;
use sebas_webui::models::GatewayInfo;
use sebas_webui::build_router;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

async fn spawn_server() -> (String, Arc<FakeBackend>) {
    let backend = Arc::new(FakeBackend::connected());
    let app = build_router(backend.clone(), GatewayInfo::default());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), backend)
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
    let (base, _backend) = spawn_server().await;

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
    let (base, backend) = spawn_server().await;

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

    // Seed a row and close it through the API: client 2 must receive both
    // the created (from the spawn) and removed events in order.
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

    let created_ev = next_event(&mut r2).await;
    assert_eq!(created_ev["type"], "session.created");
    let removed_ev = next_event(&mut r2).await;
    assert_eq!(removed_ev["type"], "session.removed", "event: {removed_ev}");
    assert_eq!(removed_ev["session_id"], key.as_str());

    // A backend-originated event (no API call involved — e.g. a Feishu-side
    // update) reaches the client too: the subscription is the only source.
    backend.emit(WebUiEvent::SessionUpdated {
        session_id: "oc_chat%00".into(),
        status: "working".into(),
    });
    let updated_ev = next_event(&mut r2).await;
    assert_eq!(updated_ev["type"], "session.updated", "event: {updated_ev}");
    assert_eq!(updated_ev["status"], "working");
}
