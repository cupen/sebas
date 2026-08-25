//! 限流中间件集成测试（sebas-lva P0）。
//!
//! 真正拉起 gateway（`start_gateway`），配置 `[gateway.rate_limit] capacity=2`，
//! 验证：
//! - 未超限（≤ capacity）请求放行（探测器回 fixture，非 429）；
//! - 超限请求返回 429 Too Many Requests，协议面为 Anthropic
//!   `rate_limit_error`；
//! - `/healthz` 豁免（无论限流与否恒 200）；
//! - 不同 token 独立桶：sk-a 超限不影响 sk-b。

mod support;

use std::time::Duration;

use support::start_gateway;

/// capacity=2 + 极慢 refill，保证测试窗口内不自动补充（避免时序抖动）。
/// `__USAGE__` 由 `start_gateway` 替换。provider 用 test 上游 key
/// （`SEBAS_GATEWAY_TEST_UPSTREAM_KEY`，start_gateway 自动 set）。
const CFG: &str = r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

auth_token = "sk-gw-test"

[gateway.rate_limit]
capacity = 2
refill_per_sec = 0.0001

[provider.anthropic]
base_url_anthropic = "https://api.anthropic.com"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY"

[gateway.routes]
"claude-*" = ["anthropic"]
"#;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

const AUTH: (&str, &str) = ("authorization", "Bearer sk-gw-test");

#[tokio::test]
async fn over_capacity_returns_429() {
    let gw = start_gateway(CFG).await;
    let client = client();
    let url = format!("http://{}/v1/messages", gw.addr);

    // capacity=2：前 2 个请求应放行（路由直达上游 → 非 429）。
    for i in 0..2 {
        let resp = client
            .post(&url)
            .header(AUTH.0, AUTH.1)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(r#"{"model":"claude-sonnet-4","messages":[]}"#)
            .send()
            .await
            .expect("POST within capacity");
        assert_ne!(
            resp.status(),
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "request {i} within capacity must not 429"
        );
    }
    // 第 3 个：超限 → 429。
    let resp = client
        .post(&url)
        .header(AUTH.0, AUTH.1)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[]}"#)
        .send()
        .await
        .expect("POST over capacity");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "over capacity must 429"
    );
    // Anthropic 协议面：{"type":"error","error":{"type":"rate_limit_error","message":...}}
    let body: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "rate_limit_error");
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn healthz_exempt_from_rate_limit() {
    let gw = start_gateway(CFG).await;
    let client = client();
    // healthz 不计费、不占令牌：连打多次恒 200。
    for _ in 0..10 {
        let resp = client
            .get(format!("http://{}/healthz", gw.addr))
            .send()
            .await
            .expect("GET /healthz");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }
}

#[tokio::test]
async fn distinct_tokens_have_independent_buckets() {
    // 两个已批准 token，各配 capacity=1：sk-a 打爆不影响 sk-b。
    const CFG_TWO: &str = r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

auth_token = ["sk-a", "sk-b"]

[gateway.rate_limit]
capacity = 1
refill_per_sec = 0.0001

[provider.anthropic]
base_url_anthropic = "https://api.anthropic.com"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY"

[gateway.routes]
"claude-*" = ["anthropic"]
"#;
    let gw = start_gateway(CFG_TWO).await;
    let client = client();
    let url = format!("http://{}/v1/messages", gw.addr);

    let send = |token: &str| {
        client
            .post(&url)
            .header("authorization", format!("Bearer {token}"))
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(r#"{"model":"claude-sonnet-4","messages":[]}"#)
            .send()
    };

    // sk-a 打爆自己的 1 令牌桶。
    let a1 = send("sk-a").await.expect("sk-a first");
    assert_ne!(
        a1.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "sk-a within capacity must pass"
    );
    let a2 = send("sk-a").await.expect("sk-a over");
    assert_eq!(
        a2.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "sk-a over capacity must 429"
    );

    // sk-b 独立桶：仍有 1 令牌，应放行。
    let b1 = send("sk-b").await.expect("sk-b first");
    assert_ne!(
        b1.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "sk-b has its own bucket and must pass"
    );
}
