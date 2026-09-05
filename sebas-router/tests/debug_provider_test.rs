//! 内置 test provider（debug 模式）集成测试。
//!
//! `[router] debug = true` 时，model=test 的请求由 router 自身应答：
//! 固定文字 + 回显最后一条 user 消息；Anthropic/OpenAI、流式/非流式都覆盖。
//! 同时验证：非 debug 模式不会拦截（model=test 正常走路由 → NoRoute 502）。

mod support;

use std::time::Duration;

use support::{start_router, start_router_debug};

/// debug 配置：parse 后由 `start_router_debug` 注入内置 test provider；
/// 配置本身仍需至少一个 provider（provider 从配置读取）。
const DEBUG_CFG: &str = r#"
[router]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

[[router.keys]]
key = "sk-gw-debug"
name = "debug-test"

[provider.anthropic]
base_url_anthropic = "http://127.0.0.1:9"
api_key = "test-key"
"#;

/// 非 debug 配置：两个 provider（避免唯一 provider 隐式默认）+ claude-* 路由；
/// model=test 无路由 → NoRoute。
const NON_DEBUG_CFG: &str = r#"
[router]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"

[[router.keys]]
key = "sk-gw-debug"

[provider.anthropic]
base_url_anthropic = "http://127.0.0.1:9"
api_key = "test-key"

[provider.openai]
base_url_openai = "http://127.0.0.1:9"
api_key = "test-key-oai"

[router.routes]
"claude-*" = ["anthropic"]
"#;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

const EXPECTED: &str = "I'm test provider. I received your message \"hello\".";

#[tokio::test]
async fn debug_anthropic_messages_json_echoes_input() {
    let gw = start_router_debug(DEBUG_CFG).await;
    let resp = client()
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("authorization", "Bearer sk-gw-debug")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"test","messages":[{"role":"user","content":"hello"}]}"#)
        .send()
        .await
        .expect("POST /v1/messages");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
    let v: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    assert_eq!(v["model"], "test");
    assert_eq!(v["content"][0]["text"], EXPECTED);
}

#[tokio::test]
async fn debug_anthropic_messages_sse_echoes_input() {
    let gw = start_router_debug(DEBUG_CFG).await;
    let resp = client()
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("authorization", "Bearer sk-gw-debug")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"test","stream":true,"messages":[{"role":"user","content":"hello"}]}"#)
        .send()
        .await
        .expect("POST /v1/messages stream");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let body = resp.text().await.expect("body");
    assert!(body.contains("event: message_start"));
    assert!(body.contains("I'm test provider. I received your message \\\"hello\\\"."));
}

#[tokio::test]
async fn debug_openai_chat_json_echoes_input() {
    let gw = start_router_debug(DEBUG_CFG).await;
    let resp = client()
        .post(format!("http://{}/v1/chat/completions", gw.addr))
        .header("authorization", "Bearer sk-gw-debug")
        .header("content-type", "application/json")
        .body(r#"{"model":"test","messages":[{"role":"user","content":"hello"}]}"#)
        .send()
        .await
        .expect("POST /v1/chat/completions");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    assert_eq!(v["choices"][0]["message"]["content"], EXPECTED);
    assert_eq!(v["model"], "test");
}

#[tokio::test]
async fn debug_openai_chat_sse_contains_text_and_done() {
    let gw = start_router_debug(DEBUG_CFG).await;
    let resp = client()
        .post(format!("http://{}/v1/chat/completions", gw.addr))
        .header("authorization", "Bearer sk-gw-debug")
        .header("content-type", "application/json")
        .body(r#"{"model":"test","stream":true,"messages":[{"role":"user","content":"hello"}]}"#)
        .send()
        .await
        .expect("POST /v1/chat/completions stream");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let body = resp.text().await.expect("body");
    assert!(body.contains("data: [DONE]"));
    assert!(body.contains("I'm test provider. I received your message \\\"hello\\\"."));
}

#[tokio::test]
async fn debug_requests_write_usage_record() {
    let gw = start_router_debug(DEBUG_CFG).await;
    let usage_path = gw.dir.path().join("usage.jsonl");
    let resp = client()
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("authorization", "Bearer sk-gw-debug")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"test","messages":[{"role":"user","content":"hello"}]}"#)
        .send()
        .await
        .expect("POST /v1/messages");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let records = support::poll_usage_jsonl(&usage_path, 1).await;
    assert_eq!(records[0]["status"], 200);
    assert_eq!(records[0]["provider"], "test");
    assert_eq!(records[0]["model"], "test");
}

#[tokio::test]
async fn debug_mode_skips_downstream_auth() {
    // debug 模式下不要求下游 key：无鉴权请求也能命中 test 模型。
    let gw = start_router_debug(DEBUG_CFG).await;
    let resp = client()
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"test","messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .expect("POST /v1/messages without key");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    assert_eq!(
        v["content"][0]["text"],
        "I'm test provider. I received your message \"hi\"."
    );
}

#[tokio::test]
async fn non_debug_router_does_not_intercept_test_model() {
    let gw = start_router(NON_DEBUG_CFG).await;
    let resp = client()
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("authorization", "Bearer sk-gw-debug")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"test","messages":[{"role":"user","content":"hello"}]}"#)
        .send()
        .await
        .expect("POST /v1/messages");

    // 非 debug：model=test 无路由 → 502 no_route（不落入内置 test provider）。
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    let v: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    assert_eq!(v["error"]["type"], "no_route");
}
