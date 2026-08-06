//! Smoke test for the gateway server skeleton (Task 2, updated for Task 7).
//!
//! Starts a real axum server on an OS-assigned port (127.0.0.1:0) using
//! `build_router`, then asserts:
//!   - GET /healthz -> 200 "ok\n" (no auth, liveness probe).
//!   - GET /v1/messages (valid key, no model, no default_provider) -> 502
//!     `no_route` in Anthropic shape. Task 7 replaced the 501 placeholder
//!     with `proxy::handle`; with this minimal cfg there is no route for a
//!     model-less request, so the proxy returns 502 NoRoute.
//!
//! The real `run` path (graceful shutdown via ctrl_c/SIGTERM) is exercised by
//! manual smoke (`cargo run -- gateway --config …` + curl), not here — it
//! blocks forever and is covered by build_router/build_state being correct.

use gateway::config::GatewayConfig;
use gateway::server;

/// Minimal valid [gateway] config: one provider with a plaintext api_key
/// (test-only; never touches the network). listen=0 lets the OS pick a port.
/// No `default_provider` so a model-less request yields NoRoute (502).
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
async fn healthz_ok_and_proxy_returns_502_no_route_for_modelless_request() {
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

    // Task 7 replaced the 501 placeholder with `proxy::handle`. A GET
    // /v1/messages carries no model (extract_model_from_path returns None for
    // non-`/v1/models/{id}`), and this minimal cfg has no default_provider,
    // so routing yields NoRoute → 502 `no_route` (Anthropic shape).
    let resp = client
        .get(format!("http://{addr}/v1/messages"))
        .header("authorization", "Bearer sk-gw-test")
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .expect("GET /v1/messages");
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    let body: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    // Anthropic error shape: {"type":"error","error":{"type":"no_route","message":...}}
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "no_route");
    assert!(body["error"]["message"].is_string());

    server_task.abort();
    let _ = server_task.await;
}
