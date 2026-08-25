//! Smoke test for the gateway server skeleton (Task 2, updated for Task 7).
//!
//! Starts a real axum server on an OS-assigned port (127.0.0.1:0) using
//! `build_router`, then asserts:
//!   - GET /healthz -> 200 "ok\n" (no auth, liveness probe).
//!   - GET /v1/messages (valid key, no model, no default_provider) -> 502
//!     `no_route` in Anthropic shape. Task 7 replaced the 501 placeholder
//!     with `proxy::handle`; with this two-provider cfg there is no route for
//!     a model-less request (no implicit default — that only applies to a
//!     single-provider config), so the proxy returns 502 NoRoute.
//!
//! The real `run` path (graceful shutdown via ctrl_c/SIGTERM) is exercised by
//! manual smoke (`cargo run -- gateway --config …` + curl), not here — it
//! blocks forever and is covered by build_router/build_state being correct.

mod support;

use gateway::config::GatewayConfig;
use gateway::server;

/// Minimal valid [gateway] config: two providers with plaintext api_keys
/// (test-only; never touches the network). listen=0 lets the OS pick a port.
/// No `default_provider` so a model-less request yields NoRoute (502) — a
/// single-provider config would implicitly default.
const CFG: &str = r#"
[gateway]
listen = "127.0.0.1:0"
auth_token = "sk-gw-test"
# 隔离：不合并开发机 ~/.sebas/providers.json（其 openai 条目与 preset
# 校验冲突导致 parse 失败）。
provider_overlay = "__sebas_server_smoke_no_overlay__.json"
[provider.anthropic]
base_url_anthropic = "https://api.anthropic.com"
api_key = "test-key"
[provider.openai]
base_url_openai = "https://api.openai.com/v1"
api_key = "test-key-oai"
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

#[tokio::test]
async fn modelless_get_models_without_default_provider_returns_502() {
    // Claude Code 的 /model 选择器会请求 GET /v1/models（无 model、带 query）。
    // 两个 provider 且无 default_provider → 默认链落空 → 502 no_route。
    // 这是 /model 报错的回归测试：模型列表请求必须在无默认时给出明确错误。
    let cfg = r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

[[gateway.keys]]
key = "sk-gw-test"

[provider.anthropic]
base_url_anthropic = "https://api.anthropic.com"
api_key = "test-key"

[provider.openai]
base_url_openai = "https://api.openai.com/v1"
api_key = "test-key-oai"
"#;
    let gw = support::start_gateway(cfg).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("client");
    let resp = client
        .get(format!("http://{}/v1/models?limit=1000", gw.addr))
        .header("authorization", "Bearer sk-gw-test")
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .expect("GET /v1/models?limit=1000");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    let body: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "no_route");
}
