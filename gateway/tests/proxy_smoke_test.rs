//! 透传引擎 smoke test（Task 7，spec §4.3）。
//!
//! 2 例（brief 契约）：
//! 1. 内联 mini 上游（axum 单路由回固定 JSON + 记录入站 header）验证
//!    JSON POST 端到端透传 + key 注入 + 业务 header 透传。
//! 2. 不可达上游（127.0.0.1:1）→ 502 `upstream_error`。
//!
//! 不复用 `support::start_gateway`：mini 上游需要把 provider.base_url 指向
//! 自己的 axum 实例，且需要读上游记录的入站 header，自旋更直接。

mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::response::Response;
use axum::routing::post;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

/// mini 上游记录的入站请求快照（header + body）。
#[derive(Debug, Clone, Default)]
struct UpstreamCapture {
    headers: std::collections::HashMap<String, String>,
    body: String,
}

#[derive(Clone)]
struct UpstreamState {
    capture: Arc<Mutex<UpstreamCapture>>,
}

/// mini 上游 handler：回固定 JSON，并把入站 header + body 落到 capture。
async fn echo_handler(State(st): State<UpstreamState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let bytes = to_bytes(body, 64 * 1024).await.unwrap_or_default();
    let mut hdrs = std::collections::HashMap::new();
    for (n, v) in parts.headers.iter() {
        if let Ok(s) = v.to_str() {
            hdrs.insert(n.as_str().to_string(), s.to_string());
        }
    }
    let body_str = String::from_utf8_lossy(&bytes).to_string();
    {
        let mut c = st.capture.lock().await;
        *c = UpstreamCapture {
            headers: hdrs,
            body: body_str,
        };
    }
    // 固定 JSON 响应（非 SSE，走 buffered 路径）。
    Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("content-type", "application/json")
        .header("x-upstream-trace", "sebas-mini")
        .body(axum::body::Body::from(
            r#"{"ok":true,"echoed":"from-mini-upstream"}"#,
        ))
        .unwrap()
}

/// 启动 mini 上游（OS 分配端口），返回地址 + capture 句柄。
async fn start_mini_upstream() -> (std::net::SocketAddr, Arc<Mutex<UpstreamCapture>>) {
    let capture = Arc::new(Mutex::new(UpstreamCapture::default()));
    let state = UpstreamState {
        capture: capture.clone(),
    };
    let app = Router::new()
        .route("/v1/messages", post(echo_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream");
    let addr = listener.local_addr().expect("upstream local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("upstream ran");
    });
    (addr, capture)
}

/// 构造一个指向 `upstream_addr` 的 gateway config：
/// - provider `anthropic` base_url = `http://{upstream_addr}`，明文 api_key
///   `test-upstream-anthropic-key`（仅测试用，resolve_api_keys 会 warn）。
/// - 下游 token `sk-downstream-gw`。
/// - model_map：claude-sonnet → upstream-claude-sonnet-4，用于验证 rename 透传。
fn gateway_cfg(upstream_addr: std::net::SocketAddr) -> String {
    format!(
        r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"
default_provider = "anthropic"

auth_token = "sk-downstream-gw"
name = "proxy-smoke"

[provider.anthropic]
protocol = "anthropic"
base_url = "http://{upstream_addr}"
api_key = "test-upstream-anthropic-key"

[provider.anthropic.model_map]
claude-sonnet = "upstream-claude-sonnet-4"
"#
    )
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

// ---------------- 1. JSON POST 端到端透传 + key 注入 + 业务 header 透传 ----------------

#[tokio::test]
async fn json_post_passthrough_injects_upstream_key_and_preserves_business_headers() {
    let (upstream_addr, capture) = start_mini_upstream().await;
    let gw = support::start_gateway(&gateway_cfg(upstream_addr)).await;
    let client = client();

    let resp = client
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("authorization", "Bearer sk-downstream-gw")
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "prompt-caching-2024-07-31")
        .header("content-type", "application/json")
        .header("x-downstream-trace", "client-abc")
        .body(r#"{"model":"claude-sonnet","messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .expect("POST through gateway");

    // 上游响应原样回传（status + body + 业务 header）
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("x-upstream-trace").unwrap(),
        "sebas-mini"
    );
    let body: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    assert_eq!(body["ok"], true);
    assert_eq!(body["echoed"], "from-mini-upstream");

    // 上游收到的入站请求：key 注入 + 业务 header 透传 + body 已 rename。
    // 轮询 capture 一会儿（mini 上游 handler 可能晚一拍写入）。
    let captured = timeout(Duration::from_secs(2), async {
        loop {
            let c = capture.lock().await.clone();
            if !c.body.is_empty() {
                return c;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("upstream capture not empty within 2s");

    // 上游注入的 x-api-key（非下游 key）
    assert_eq!(
        captured.headers.get("x-api-key").unwrap(),
        "test-upstream-anthropic-key",
        "upstream must receive injected x-api-key"
    );
    // 下游 key 绝不泄漏到上游
    assert!(
        !captured
            .headers
            .values()
            .any(|v| v.contains("sk-downstream-gw")),
        "downstream key must not leak to upstream: {:?}",
        captured.headers
    );
    // 业务 header 透传
    assert_eq!(
        captured.headers.get("anthropic-version").unwrap(),
        "2023-06-01"
    );
    assert_eq!(
        captured.headers.get("anthropic-beta").unwrap(),
        "prompt-caching-2024-07-31"
    );
    assert_eq!(
        captured.headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(
        captured.headers.get("x-downstream-trace").unwrap(),
        "client-abc"
    );
    // 下游的 authorization Bearer 必须被剥离（Anthropic 用 x-api-key）
    assert!(
        !captured.headers.contains_key("authorization"),
        "downstream Authorization must be stripped for Anthropic path"
    );
    // body 已 rename：model 从 claude-sonnet 改写为 upstream-claude-sonnet-4
    let upstream_body: serde_json::Value =
        serde_json::from_str(&captured.body).expect("upstream body is JSON");
    assert_eq!(
        upstream_body["model"], "upstream-claude-sonnet-4",
        "model must be renamed via model_map; got body: {}",
        captured.body
    );
    // 其它字段保留
    assert_eq!(upstream_body["messages"][0]["content"], "hi");
}

// ---------------- 2. 不可达上游 → 502 upstream_error ----------------

#[tokio::test]
async fn unreachable_upstream_returns_502_upstream_error() {
    // 127.0.0.1:1 几乎必然拒绝/超时（port 1 是特权端口，无监听）。
    // gateway 的 connect_timeout 默认 10s；这里把 reqwest 客户端 timeout 设 5s
    // 兜底，避免测试卡到 connect_timeout。
    let cfg = r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"
default_provider = "unreachable"

auth_token = "sk-downstream-gw"
name = "proxy-smoke"

[provider.unreachable]
protocol = "anthropic"
base_url = "http://127.0.0.1:1"
api_key = "test-unreachable-key"
"#;
    let gw = support::start_gateway(cfg).await;
    let client = client();

    let resp = client
        .post(format!("http://{}/v1/messages", gw.addr))
        .header("authorization", "Bearer sk-downstream-gw")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet","messages":[]}"#)
        .send()
        .await
        .expect("POST through gateway");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    let body: serde_json::Value =
        serde_json::from_str(&resp.text().await.expect("body")).expect("valid JSON");
    // Anthropic 错误格式：{"type":"error","error":{"type":"upstream_error","message":...}}
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "upstream_error");
    // message 通用，不含 key
    let msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        !msg.contains("test-unreachable-key") && !msg.contains("sk-downstream-gw"),
        "5xx message must not leak keys: {msg}"
    );
}
