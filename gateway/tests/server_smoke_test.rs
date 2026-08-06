//! Smoke test for the gateway server skeleton (Task 2).
//!
//! Starts a real axum server on an OS-assigned port (127.0.0.1:0) using
//! `build_router`, then asserts:
//!   - GET /healthz -> 200 "ok\n" (no auth, liveness probe).
//!   - GET /v1/messages (unimplemented, valid key) -> 501 with a protocol-shaped
//!     error body (Anthropic shape). Task 5 added the auth layer in front of the
//!     placeholder, so a valid downstream key is required to reach it.
//!
//! The real `run` path (graceful shutdown via ctrl_c/SIGTERM) is exercised by
//! manual smoke (`cargo run -- gateway --config …` + curl), not here — it
//! blocks forever and is covered by build_router/build_state being correct.

use gateway::config::GatewayConfig;
use gateway::server;

/// Minimal valid [gateway] config: one provider with a plaintext api_key
/// (test-only; never touches the network). listen=0 lets the OS pick a port.
const CFG: &str = r#"
[gateway]
listen = "127.0.0.1:0"
[[gateway.keys]]
key = "sk-gw-test"
[gateway.providers.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "test-key"
"#;

#[tokio::test]
async fn healthz_ok_and_placeholder_returns_501() {
    let cfg = GatewayConfig::parse(CFG).expect("parse test config");
    let state = server::build_state(cfg).expect("build_state");
    let app = server::build_router(state);

    // Bind before spawning so the port is accepting into the OS backlog
    // before the client connects — avoids a startup race without sleeping.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server ran");
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("client");

    let healthz = client
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .expect("GET /healthz");
    assert_eq!(healthz.status(), reqwest::StatusCode::OK);
    assert_eq!(healthz.text().await.expect("healthz body"), "ok\n");

    let placeholder = client
        .get(format!("http://{addr}/v1/messages"))
        // Task 5 auth layer gates the placeholder; send the configured key to
        // reach it and still assert 501 (Task 7 swaps it for proxy::handle).
        .header("authorization", "Bearer sk-gw-test")
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .expect("GET /v1/messages");
    assert_eq!(placeholder.status(), reqwest::StatusCode::NOT_IMPLEMENTED);
    let body: serde_json::Value =
        serde_json::from_str(&placeholder.text().await.expect("placeholder body"))
            .expect("placeholder body is valid JSON");
    // Anthropic error shape (placeholder hardcodes Anthropic pre-Task-7):
    // {"type":"error","error":{"type":..,"message":..}}
    assert_eq!(body["type"], "error");
    assert!(body["error"]["type"].is_string());
    assert!(body["error"]["message"].is_string());

    server_task.abort();
    let _ = server_task.await;
}
