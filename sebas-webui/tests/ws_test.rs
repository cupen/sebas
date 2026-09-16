//! Integration test for the WebSocket realtime channel `/ws`.
//!
//! Binds a real listener on an ephemeral port, drives the server with
//! tokio, connects a `tokio-tungstenite` client, and asserts that session
//! mutations broadcast `Notification` envelope frames over the wire
//! (add-ws-rpc-protocol: `method` = the legacy dotted type, `params` = the
//! legacy payload) and that client `Request`s round-trip through the RPC
//! handler registry.

use futures_util::{SinkExt, StreamExt};
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use sebas_feishu::cards::CardConfig;
use sebas_webui::build_router;
use sebas_webui::models::RouterInfo;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

async fn spawn_server() -> (
    String,
    tokio::sync::mpsc::Receiver<sebas_dispatch::engine::Out>,
) {
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

/// Read the next WS text frame as JSON (an envelope frame), with a hard
/// timeout so a bug surfaces as a test failure rather than a hang.
async fn next_event(
    ws: &mut (
             impl StreamExt<
        Item = Result<
            tokio_tungstenite::tungstenite::Message,
            tokio_tungstenite::tungstenite::Error,
        >,
    > + Unpin
         ),
) -> Value {
    let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("timed out waiting for a WebSocket event")
        .expect("websocket stream ended")
        .expect("websocket error");
    match msg {
        tokio_tungstenite::tungstenite::Message::Text(text) => {
            serde_json::from_str(&text).expect("event must be envelope JSON")
        }
        // Pings and other control frames are skipped; the next read gets
        // the event.
        _ => Box::pin(next_event(ws)).await,
    }
}

/// Send a client frame: a `Request` envelope (add-ws-rpc-protocol).
async fn send_request(
    ws: &mut (impl SinkExt<tokio_tungstenite::tungstenite::Message> + Unpin),
    id: u64,
    method: &str,
    params: Value,
) {
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::json!({ "id": id, "method": method, "params": params })
            .to_string()
            .into(),
    ))
    .await
    .ok()
    .expect("send request frame");
}

/// Send a raw text payload verbatim (for malformed-frame tests).
async fn send_raw(
    ws: &mut (impl SinkExt<tokio_tungstenite::tungstenite::Message> + Unpin),
    raw: &str,
) {
    ws.send(tokio_tungstenite::tungstenite::Message::Text(raw.into()))
        .await
        .ok()
        .expect("send raw frame");
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
        .json(&serde_json::json!({ "prompt": "hello", "agent": "claude" }))
        .send()
        .await
        .expect("create request failed");
    assert_eq!(resp.status(), 201);
    let created: Value = resp.json().await.unwrap();
    let key = created["key"].as_str().unwrap().to_string();

    let event = next_event(&mut reader).await;
    // add-ws-rpc-protocol：Notification 封套——method = 原 dotted type，
    // params = 原载荷；裸 {type, ...} 帧不再出现。
    assert_eq!(event["method"], "session.created", "event: {event}");
    assert!(
        event.get("type").is_none(),
        "bare frame must be gone: {event}"
    );
    assert_eq!(event["params"]["session_id"], key.as_str());
    // Writer is kept so the connection stays open for the assertions above.
    let _ = writer
        .send(tokio_tungstenite::tungstenite::Message::Close(None))
        .await;
}

#[tokio::test]
async fn one_client_disconnecting_does_not_starve_others() {
    let (base, _rx) = spawn_server().await;

    // Two clients; then the first disconnects.
    let (ws1, _) = tokio_tungstenite::connect_async(ws_url(&base))
        .await
        .unwrap();
    let (ws2, _) = tokio_tungstenite::connect_async(ws_url(&base))
        .await
        .unwrap();
    let (_w2, mut r2) = ws2.split();

    // Client 1 hangs up.
    {
        let (mut w1, mut r1) = ws1.split();
        let _ = w1
            .send(tokio_tungstenite::tungstenite::Message::Close(None))
            .await;
        let _ = r1.next().await; // drain the close echo
    }

    // A session close must still reach client 2. Seed a dormant session via
    // the API path: create (spawning) then close it.
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{base}/api/sessions"))
        .json(&serde_json::json!({ "prompt": "doomed", "agent": "claude" }))
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
    assert_eq!(created_ev["method"], "session.created");
    let removed_ev = next_event(&mut r2).await;
    assert_eq!(
        removed_ev["method"], "session.removed",
        "event: {removed_ev}"
    );
    assert_eq!(removed_ev["params"]["session_id"], key.as_str());
}

/// add-ws-rpc-protocol 2.2 / spec「ping 往返」：Request ping → id 匹配的
/// pong Response，result 与 error 互斥。
#[tokio::test]
async fn ping_request_roundtrips_with_matching_id() {
    let (base, _rx) = spawn_server().await;
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url(&base))
        .await
        .unwrap();
    let (mut writer, mut reader) = ws_stream.split();

    send_request(&mut writer, 7, "ping", serde_json::json!({})).await;
    let reply = next_event(&mut reader).await;
    assert_eq!(reply["id"], 7, "response id must echo the request: {reply}");
    assert_eq!(reply["result"], "pong");
    assert!(
        reply.get("error").is_none(),
        "ok reply carries no error: {reply}"
    );
}

/// add-ws-rpc-protocol 2.1 / spec「未知 method 拒单不断连」：unknown_method
/// error Response（id 匹配、连接保持），后续 ping 正常。
#[tokio::test]
async fn unknown_method_is_rejected_without_dropping_the_connection() {
    let (base, _rx) = spawn_server().await;
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url(&base))
        .await
        .unwrap();
    let (mut writer, mut reader) = ws_stream.split();

    send_request(&mut writer, 1, "core.does_not_exist", serde_json::json!({})).await;
    let reply = next_event(&mut reader).await;
    assert_eq!(
        reply["id"], 1,
        "error reply must echo the request id: {reply}"
    );
    assert_eq!(reply["error"]["code"], "unknown_method");
    assert!(
        reply["error"]["message"].is_string(),
        "error carries a message: {reply}"
    );
    assert!(
        reply.get("result").is_none(),
        "error reply carries no result: {reply}"
    );

    // 连接未断：同一 socket 上的后续 Request 正常处理。
    send_request(&mut writer, 2, "ping", serde_json::json!({})).await;
    let reply = next_event(&mut reader).await;
    assert_eq!(
        reply["id"], 2,
        "connection must stay up after a rejection: {reply}"
    );
    assert_eq!(reply["result"], "pong");
}

/// add-ws-rpc-protocol 2.2：畸形帧（非 JSON、非三态）与客户端发来的
/// Response/Notification 一律忽略——不回写、不断连，后续 ping 正常。
#[tokio::test]
async fn malformed_and_non_request_frames_are_ignored_connection_stays() {
    let (base, _rx) = spawn_server().await;
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url(&base))
        .await
        .unwrap();
    let (mut writer, mut reader) = ws_stream.split();

    send_raw(&mut writer, "not json at all").await;
    send_raw(&mut writer, "42").await; // JSON but not a frame
    send_raw(&mut writer, "[1, 2, 3]").await; // array shape
    send_raw(&mut writer, "{}").await; // object matching none of the three
    // Well-formed but meaningless client → server frames.
    send_raw(&mut writer, r#"{"method":"session.created","params":{}}"#).await;
    send_raw(&mut writer, r#"{"id":99,"result":"unsolicited"}"#).await;

    // 任何被忽略的帧若产生了回写，这里先读到的就不会是 id=9 的 pong。
    send_request(&mut writer, 9, "ping", serde_json::json!({})).await;
    let reply = next_event(&mut reader).await;
    assert_eq!(reply["id"], 9, "ignored frames must not reply: {reply}");
    assert_eq!(reply["result"], "pong");
}
