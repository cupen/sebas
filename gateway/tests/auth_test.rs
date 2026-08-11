//! 鉴权中间件测试（Task 5，spec §4.5）。
//!
//! 5 例：healthz 豁免／无 key 401 Anthropic 格式（带 anthropic-version）／
//! 无 key 401 OpenAI 格式／错 key 401／Bearer 与 x-api-key 都通过
//! （断言 ≠401，此时 fallback 仍 501，Task 7 后自然变 200/502，测试不受影响）。

mod support;

use std::time::Duration;

use support::start_gateway;

/// 双协议面最小 config：一个 anthropic provider + 一个 openai provider，
/// 均从测试 env key 取上游密钥；一个下游 key `sk-gw-test`。
/// `__USAGE__` 由 `start_gateway` 替换为 tempdir 内 usage.jsonl。
const CFG: &str = r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

auth_token = "sk-gw-test"

[provider.anthropic]
base_url_anthropic = "https://api.anthropic.com"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY"

[provider.openai]
base_url_openai = "https://api.openai.com"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI"
"#;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

#[tokio::test]
async fn healthz_exempt_returns_200() {
    let gw = start_gateway(CFG).await;
    let client = client();
    let resp = client
        .get(format!("http://{}/healthz", gw.addr))
        .send()
        .await
        .expect("GET /healthz");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.expect("body"), "ok\n");
}

#[tokio::test]
async fn no_key_anthropic_format_401() {
    let gw = start_gateway(CFG).await;
    let client = client();
    let resp = client
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[]}"#)
        .send()
        .await
        .expect("POST /v1/messages");
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    // Anthropic shape: {"type":"error","error":{"type":"authentication_error","message":...}}
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "authentication_error");
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn no_auth_token_configured_skips_authentication() {
    // 未配置 auth_token：不校验 token（裸奔）。无 key 请求应直达路由层
    // （此处无 default_provider + 双 provider → 502 no_route），而非 401。
    let cfg = CFG.replace("auth_token = \"sk-gw-test\"\n", "");
    let gw = start_gateway(&cfg).await;
    let client = client();
    let resp = client
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[]}"#)
        .send()
        .await
        .expect("POST /v1/messages without auth_token");
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "no auth_token configured must not 401"
    );
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::BAD_GATEWAY,
        "no auth_token → request reaches routing (502 no_route)"
    );
}

#[tokio::test]
async fn no_key_openai_format_401() {
    let gw = start_gateway(CFG).await;
    let client = client();
    let resp = client
        .post(format!("http://{}/v1/chat/completions", gw.addr))
        .header("content-type", "application/json")
        .body(r#"{"model":"gpt-4","messages":[]}"#)
        .send()
        .await
        .expect("POST /v1/chat/completions");
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    // OpenAI shape: {"error":{"message":...,"type":"invalid_request_error","code":null}}
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(body["error"]["message"].is_string());
    assert!(body["error"]["code"].is_null());
}

#[tokio::test]
async fn wrong_key_401() {
    let gw = start_gateway(CFG).await;
    let client = client();
    let resp = client
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("anthropic-version", "2023-06-01")
        .header("authorization", "Bearer sk-wrong-key")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[]}"#)
        .send()
        .await
        .expect("POST /v1/messages");
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    // 401 message must NEVER echo the presented key.
    let text = resp.text().await.expect("body");
    assert!(
        !text.contains("sk-wrong-key"),
        "401 body must not echo the presented key: {text}"
    );
}

#[tokio::test]
async fn bearer_and_x_api_key_both_pass() {
    let gw = start_gateway(CFG).await;
    let client = client();
    // Bearer 形式：Authorization: Bearer sk-gw-test
    let resp = client
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("anthropic-version", "2023-06-01")
        .header("authorization", "Bearer sk-gw-test")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[]}"#)
        .send()
        .await
        .expect("POST /v1/messages bearer");
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "valid Bearer must pass auth (hits 501 placeholder pre-Task-7)"
    );
    // x-api-key 形式
    let resp = client
        .post(format!("http://{}/v1/chat/completions", gw.addr))
        .header("x-api-key", "sk-gw-test")
        .header("content-type", "application/json")
        .body(r#"{"model":"gpt-4","messages":[]}"#)
        .send()
        .await
        .expect("POST /v1/chat/completions x-api-key");
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "valid x-api-key must pass auth (hits 501 placeholder pre-Task-7)"
    );
}
