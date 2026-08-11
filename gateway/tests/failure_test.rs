//! 失败场景集成测试（上游 429 / 5xx / 读超时 / SSE 截断）。
//!
//! 全部用 ad hoc axum handler（proxy_smoke 风格），不动 `support` 的 fixture 表：
//! ① 上游 429 + `retry-after` 原样透传（status/header/body 逐字节断言）；
//! ② 上游 503 JSON 原样透传；
//! ③ 上游读超时（config `read_timeout_secs = 1`，mock sleep 3s）→ 502
//!    `upstream_error` 且不泄漏 key；
//! ④ SSE 中途截断（message_start 帧截断）→ 客户端收到的字节与上游完全一致、
//!    不 502，usage 尽力落盘。

mod support;

use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use support::start_gateway;

/// 失败场景枚举：`failure_handler` 按场景回不同响应。
#[derive(Clone, Copy)]
enum Scenario {
    TooManyRequests,
    ServerError,
    Slow,
    TruncatedSse,
}

/// 429 fixture：status/header/body 都要原样透传。
const TOO_MANY_REQUESTS_BODY: &str =
    r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#;
/// 503 fixture：status/body 原样透传。
const SERVER_ERROR_BODY: &str = r#"{"error":{"type":"server_error","message":"boom"}}"#;
/// 截断的 SSE：`message_start` 完整，`content_block_delta` 的 JSON 中途断掉
/// （无闭合括号、无结尾换行）。网关必须原样转发，不因坏帧返回 502。
const TRUNCATED_SSE: &str = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_trunc\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}";

/// ad hoc 失败 handler：按 scenario 回固定响应。
async fn failure_handler(State(s): State<Scenario>, _req: Request) -> Response {
    match s {
        Scenario::TooManyRequests => Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("content-type", "application/json")
            .header("retry-after", "30")
            .body(axum::body::Body::from(TOO_MANY_REQUESTS_BODY))
            .unwrap(),
        Scenario::ServerError => Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(SERVER_ERROR_BODY))
            .unwrap(),
        Scenario::Slow => {
            tokio::time::sleep(Duration::from_secs(3)).await;
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(r#"{"ok":true}"#))
                .unwrap()
        }
        Scenario::TruncatedSse => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(axum::body::Body::from(TRUNCATED_SSE))
            .unwrap(),
    }
}

/// 启动一台只服务 `/v1/messages` 的 ad hoc 上游，返回 base URL。
async fn start_failure_upstream(scenario: Scenario) -> String {
    let app = Router::new()
        .route("/v1/messages", post(failure_handler))
        .with_state(scenario);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failure upstream");
    let addr = listener.local_addr().expect("failure upstream local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("failure upstream ran");
    });
    format!("http://{addr}")
}

/// gateway config：单 provider `up` 指向 ad hoc 上游，明文 api_key（仅测试用）。
/// `extra_gateway_fields` 插到 `[gateway]` 段（如 `read_timeout_secs`）。
fn cfg(upstream: &str, extra_gateway_fields: &str) -> String {
    format!(
        r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"
{extra_gateway_fields}
default_provider = "up"

auth_token = "sk-gw-fail"
name = "failure-test"

[provider.up]
base_url_anthropic = "{upstream}"
api_key = "test-upstream-key"
"#
    )
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client")
}

/// 通用请求：POST /v1/messages（Anthropic 面）+ 下游 key + 业务 header。
fn post_messages(client: &reqwest::Client, base: &str) -> reqwest::RequestBuilder {
    client
        .post(format!("{base}/v1/messages"))
        .header("authorization", "Bearer sk-gw-fail")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#)
}

#[tokio::test]
async fn upstream_429_passthrough_with_retry_after() {
    let url = start_failure_upstream(Scenario::TooManyRequests).await;
    let gw = start_gateway(&cfg(&url, "")).await;
    let resp = post_messages(&client(), &format!("http://{}", gw.addr))
        .send()
        .await
        .expect("POST /v1/messages");

    assert_eq!(resp.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(
        resp.text().await.expect("body"),
        TOO_MANY_REQUESTS_BODY,
        "upstream 429 body must pass through byte-for-byte"
    );
}

#[tokio::test]
async fn upstream_503_passthrough() {
    let url = start_failure_upstream(Scenario::ServerError).await;
    let gw = start_gateway(&cfg(&url, "")).await;
    let resp = post_messages(&client(), &format!("http://{}", gw.addr))
        .send()
        .await
        .expect("POST /v1/messages");

    assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(
        resp.text().await.expect("body"),
        SERVER_ERROR_BODY,
        "upstream 503 body must pass through byte-for-byte"
    );
}

#[tokio::test]
async fn upstream_read_timeout_returns_502_without_key_leak() {
    let url = start_failure_upstream(Scenario::Slow).await;
    let gw = start_gateway(&cfg(&url, "read_timeout_secs = 1\n")).await;
    let resp = post_messages(&client(), &format!("http://{}", gw.addr))
        .send()
        .await
        .expect("POST /v1/messages");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    let body: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    // Anthropic 错误格式：{"type":"error","error":{"type":"upstream_error","message":...}}
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "upstream_error");
    let msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        !msg.contains("test-upstream-key") && !msg.contains("sk-gw-fail"),
        "502 message must not leak keys: {msg}"
    );
}

#[tokio::test]
async fn truncated_sse_passthrough_and_usage_best_effort() {
    let url = start_failure_upstream(Scenario::TruncatedSse).await;
    let gw = start_gateway(&cfg(&url, "")).await;
    let usage_path = gw.dir.path().join("usage.jsonl");
    let resp = post_messages(&client(), &format!("http://{}", gw.addr))
        .send()
        .await
        .expect("POST /v1/messages");

    // 不 502：字节原样透传
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let bytes = resp.bytes().await.expect("read SSE body");
    assert_eq!(
        &bytes[..],
        TRUNCATED_SSE.as_bytes(),
        "truncated SSE must pass through byte-for-byte"
    );

    // usage 尽力解析：流结束即结算一条 200 record（token 字段可全 None）。
    let records = support::poll_usage_jsonl(&usage_path, 1).await;
    assert_eq!(records[0]["status"], 200);
    assert_eq!(records[0]["protocol"], "anthropic");
}
