//! 测试支撑：启动真实 gateway（OS 分配端口），自动 set 两个测试 env key，
//! 并把 config 中的 `__USAGE__` 占位替换为 tempdir 内 usage.jsonl，
//! 避免测试污染 `~/.local/state`。
//!
//! Task 9 扩展本模块追加 mock upstream（双协议面 axum fallback）+ fixture 集
//! + 断言辅助（header 查找、usage.jsonl 轮询）。
//!
//! 各 test 二进制独立编译本模块，未必用到每个 pub 项（如 auth_test 不用
//! mock upstream），故模块级 `allow(dead_code)` 抑制跨二进制未用警告。
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Once};
use std::time::Duration;

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::Response;
use tempfile::TempDir;
use tokio::sync::Mutex;

use gateway::config::GatewayConfig;
use gateway::proto::Protocol;
use gateway::server;

/// 启动一个 gateway 实例并返回其监听地址 + 持有 TempDir（drop 即清理）。
///
/// `config_toml` 中：
/// - `usage_file = "__USAGE__"` 会被替换为 tempdir 内 `usage.jsonl`；
/// - provider 的 `api_key_env` 应指向 `SEBAS_GATEWAY_TEST_UPSTREAM_KEY`
///   或 `SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI`，本函数自动 set 两者。
///
/// Task 8 的 usage sink 会写经 `__USAGE__` 替换出的 tempdir 路径，故测试
/// 不会触及 `~/.local/state`。
pub async fn start_gateway(config_toml: &str) -> TestGateway {
    start_gateway_impl(config_toml, false).await
}

/// 以 debug 模式启动：parse 完成后注入内置 test provider（`--debug` 语义）。
pub async fn start_gateway_debug(config_toml: &str) -> TestGateway {
    start_gateway_impl(config_toml, true).await
}

async fn start_gateway_impl(config_toml: &str, debug: bool) -> TestGateway {
    ensure_test_env_keys();

    let dir = tempfile::tempdir().expect("tempdir");
    let usage_path = dir.path().join("usage.jsonl");
    // Windows 临时路径含反斜杠（`C:\Users\...\Temp\...`），TOML 会把 `\U` 当
    // unicode 转义导致解析失败。统一换成 `/`（TOML 与 OS 都接受）。
    let usage = usage_path.to_string_lossy().replace('\\', "/");
    let raw = config_toml.replace("__USAGE__", &usage);
    let mut cfg = GatewayConfig::parse(&raw).expect("parse test config");
    if debug {
        cfg.enable_debug_test_provider();
    }
    let state = server::build_state(cfg).expect("build_state");
    let app = server::build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("server ran");
    });

    TestGateway {
        addr,
        dir,
        _server: server,
    }
}

/// 运行中的 gateway 测试实例。drop 时 abort 后台 task + 清理 TempDir。
pub struct TestGateway {
    pub addr: SocketAddr,
    /// 持有以保持 tempdir 存活至 drop；Task 9 读 `dir.path()` 轮询 usage.jsonl。
    pub dir: TempDir,
    _server: tokio::task::JoinHandle<()>,
}

impl Drop for TestGateway {
    fn drop(&mut self) {
        self._server.abort();
    }
}

static ENV_ONCE: Once = Once::new();

/// 设置两个测试上游 key（每个测试进程仅 set 一次，值恒定）。
///
/// 用 `Once` 而非每次 `start_gateway` 都 set：本进程内多个 `#[tokio::test]`
/// 并发调用 `start_gateway` 时，`call_once` 保证 set 恰好发生一次且先于任何
/// `build_state` 的 `std::env::var` 读取返回，无写读竞态。
fn ensure_test_env_keys() {
    ENV_ONCE.call_once(|| {
        // SAFETY: `Once::call_once` 保证本块在进程内只执行一次；set 后不 remove、
        // 值恒定。各测试文件独立进程，无跨文件竞态。后续 `build_state` 的
        // `std::env::var` 读取发生在 `call_once` 返回之后，无写读竞态。
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_TEST_UPSTREAM_KEY", "test-anthropic-key");
            std::env::set_var("SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI", "test-openai-key");
        }
    });
}

// ===== Mock upstream（Task 9）=====

/// mock 上游记录的入站请求快照（method / path+query / headers / body）。
/// `path` 含 query string（若有）。headers 的 key 已被 `HeaderName::as_str()`
/// 规范化为小写；用 `recorded_header_get` 做 case-insensitive 查找。
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// 运行中的 mock 上游。`url` 是 gateway config 中 provider.base_url 应指向的
/// 地址；`requests` 是入站请求记录（drop 时 abort 后台 task）。
pub struct MockUpstream {
    pub url: String,
    pub requests: Arc<Mutex<Vec<RecordedRequest>>>,
    _server: tokio::task::JoinHandle<()>,
}

impl Drop for MockUpstream {
    fn drop(&mut self) {
        self._server.abort();
    }
}

/// 启动一台 mock 上游（axum fallback），按 `flavor`（Anthropic / OpenAI）
/// 在已记录的路径上回固定 fixture。两台各起一次，用「请求落在哪台 mock」
/// 断言 gateway 路由。
pub async fn start_mock_upstream(flavor: Protocol) -> MockUpstream {
    let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let state = MockState {
        flavor,
        requests: requests.clone(),
    };
    let app = Router::new().fallback(mock_handler).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock upstream");
    let addr = listener.local_addr().expect("mock local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock upstream ran");
    });
    MockUpstream {
        url: format!("http://{addr}"),
        requests,
        _server: server,
    }
}

#[derive(Clone)]
struct MockState {
    flavor: Protocol,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

/// mock fallback handler：先记录入站请求（method/path+query/headers/body），
/// 再按 (flavor, path, method, stream) 选 fixture 回固定响应。
async fn mock_handler(State(st): State<MockState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let bytes = to_bytes(body, 64 * 1024).await.unwrap_or_default();
    let method = parts.method.clone();
    let path = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let mut hdrs = HashMap::new();
    for (n, v) in parts.headers.iter() {
        if let Ok(s) = v.to_str() {
            hdrs.insert(n.as_str().to_string(), s.to_string());
        }
    }
    let body_str = String::from_utf8_lossy(&bytes).to_string();
    st.requests.lock().await.push(RecordedRequest {
        method: method.as_str().to_string(),
        path,
        headers: hdrs,
        body: body_str.clone(),
    });

    // stream 标志：解析 body JSON 取 `stream` 布尔。GET / 坏 JSON → false。
    let wants_stream = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false);

    let path_only = parts.uri.path();
    let (status, content_type, body_out, trace) =
        match_fixture(st.flavor, path_only, &method, wants_stream);

    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("x-mock-trace", trace)
        .body(axum::body::Body::from(body_out))
        .unwrap()
}

/// 按 (flavor, path, method, stream) 选 fixture。返回
/// (status, content-type, body, trace-sentinel)。无匹配 → 404。
fn match_fixture(
    flavor: Protocol,
    path: &str,
    method: &Method,
    wants_stream: bool,
) -> (StatusCode, &'static str, &'static str, &'static str) {
    match (flavor, path, method.clone()) {
        (Protocol::Anthropic, "/v1/messages", Method::POST) if wants_stream => (
            StatusCode::OK,
            "text/event-stream",
            ANTHROPIC_MESSAGES_SSE,
            "anthropic-messages-sse",
        ),
        (Protocol::Anthropic, "/v1/messages", Method::POST) => (
            StatusCode::OK,
            "application/json",
            ANTHROPIC_MESSAGES_JSON,
            "anthropic-messages-json",
        ),
        (Protocol::Anthropic, "/v1/messages/count_tokens", Method::POST) => (
            StatusCode::OK,
            "application/json",
            ANTHROPIC_COUNT_TOKENS,
            "anthropic-count-tokens",
        ),
        (Protocol::Anthropic, "/v1/models", Method::GET) => (
            StatusCode::OK,
            "application/json",
            ANTHROPIC_MODELS_LIST,
            "anthropic-models-list",
        ),
        (Protocol::Anthropic, p, Method::GET) if is_model_get(p) => (
            StatusCode::OK,
            "application/json",
            ANTHROPIC_MODEL_GET,
            "anthropic-model-get",
        ),
        (Protocol::OpenAi, "/v1/chat/completions", Method::POST) if wants_stream => (
            StatusCode::OK,
            "text/event-stream",
            OPENAI_CHAT_SSE,
            "openai-chat-sse",
        ),
        (Protocol::OpenAi, "/v1/chat/completions", Method::POST) => (
            StatusCode::OK,
            "application/json",
            OPENAI_CHAT_JSON,
            "openai-chat-json",
        ),
        (Protocol::OpenAi, "/v1/responses", Method::POST) => (
            StatusCode::OK,
            "application/json",
            OPENAI_RESPONSES_JSON,
            "openai-responses-json",
        ),
        (Protocol::OpenAi, "/v1/embeddings", Method::POST) => (
            StatusCode::OK,
            "application/json",
            OPENAI_EMBEDDINGS,
            "openai-embeddings",
        ),
        (Protocol::OpenAi, "/v1/models", Method::GET) => (
            StatusCode::OK,
            "application/json",
            OPENAI_MODELS_LIST,
            "openai-models-list",
        ),
        (Protocol::OpenAi, p, Method::GET) if is_model_get(p) => (
            StatusCode::OK,
            "application/json",
            OPENAI_MODEL_GET,
            "openai-model-get",
        ),
        _ => (
            StatusCode::NOT_FOUND,
            "application/json",
            r#"{"error":"no fixture for this path/method/flavor"}"#,
            "no-fixture",
        ),
    }
}

/// 判定 path 是否为 `/v1/models/{id}`（单层 id，无嵌套）。
fn is_model_get(path: &str) -> bool {
    let rest = match path.strip_prefix("/v1/models/") {
        Some(r) => r,
        None => return false,
    };
    !rest.is_empty() && !rest.contains('/')
}

// ===== Fixtures =====
// usage 数字须与 contract_test.rs 的 usage.jsonl 断言一致。
// Anthropic messages：input=10 output=25 cache_read=5 cache_creation=2。
// OpenAI chat：prompt=12 completion=34。OpenAI responses：input=8 output=20。

/// Anthropic messages 非流式 JSON（含 usage + cache 字段）。
pub const ANTHROPIC_MESSAGES_JSON: &str = r#"{
  "id": "msg_mock_001",
  "type": "message",
  "role": "assistant",
  "model": "claude-sonnet-4",
  "content": [{"type": "text", "text": "Hello from mock anthropic"}],
  "stop_reason": "end_turn",
  "stop_sequence": null,
  "usage": {
    "input_tokens": 10,
    "output_tokens": 25,
    "cache_read_input_tokens": 5,
    "cache_creation_input_tokens": 2
  }
}"#;

/// Anthropic messages SSE（message_start/content_block_delta/message_delta/message_stop）。
/// message_start 给 input+cache_*（output_tokens=1 占位，parser 不取）；
/// message_delta 给 output_tokens=25。
pub const ANTHROPIC_MESSAGES_SSE: &str = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_mock_002\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":5,\"cache_creation_input_tokens\":2,\"output_tokens\":1}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" from mock\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":25}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

/// Anthropic count_tokens 响应（无 usage 块，本身即计数）。
pub const ANTHROPIC_COUNT_TOKENS: &str = r#"{"input_tokens": 42}"#;

/// Anthropic models 列表。
pub const ANTHROPIC_MODELS_LIST: &str = r#"{
  "data": [
    {"type":"model","id":"claude-sonnet-4-20250514","display_name":"Claude Sonnet 4","created_at":"2025-05-14T00:00:00Z"},
    {"type":"model","id":"claude-opus-4-20250514","display_name":"Claude Opus 4","created_at":"2025-05-14T00:00:00Z"}
  ],
  "has_more": false,
  "first_id": "claude-sonnet-4-20250514",
  "last_id": "claude-opus-4-20250514"
}"#;

/// Anthropic 单 model 获取。
pub const ANTHROPIC_MODEL_GET: &str = r#"{"type":"model","id":"claude-sonnet-4-20250514","display_name":"Claude Sonnet 4","created_at":"2025-05-14T00:00:00Z"}"#;

/// OpenAI chat 非流式 JSON（usage.prompt_tokens=12 completion_tokens=34）。
pub const OPENAI_CHAT_JSON: &str = r#"{
  "id": "chatcmpl-mock-001",
  "object": "chat.completion",
  "created": 1700000000,
  "model": "gpt-4",
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello from mock openai"}, "finish_reason": "stop"}],
  "usage": {"prompt_tokens": 12, "completion_tokens": 34, "total_tokens": 46}
}"#;

/// OpenAI chat SSE（末尾 chunk 带 usage + [DONE]）。usage.prompt_tokens=12 completion_tokens=34。
pub const OPENAI_CHAT_SSE: &str = "\
data: {\"id\":\"chatcmpl-mock-002\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-mock-002\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-mock-002\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" from mock\"},\"finish_reason\":null}]}\n\
\n\
data: {\"id\":\"chatcmpl-mock-002\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":34,\"total_tokens\":46}}\n\
\n\
data: [DONE]\n\
\n";

/// OpenAI Responses 非流式 JSON（usage.input_tokens=8 output_tokens=20）。
pub const OPENAI_RESPONSES_JSON: &str = r#"{
  "id": "resp_mock_001",
  "object": "response",
  "created_at": 1700000000,
  "status": "completed",
  "model": "gpt-4",
  "output": [{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello from mock responses"}]}],
  "usage": {"input_tokens": 8, "output_tokens": 20, "total_tokens": 28}
}"#;

/// OpenAI embeddings 响应。
pub const OPENAI_EMBEDDINGS: &str = r#"{
  "object": "list",
  "data": [{"object":"embedding","index":0,"embedding":[0.1,0.2,0.3]}],
  "model": "text-embedding-3-small",
  "usage": {"prompt_tokens": 4, "total_tokens": 4}
}"#;

/// OpenAI models 列表。
pub const OPENAI_MODELS_LIST: &str = r#"{
  "object": "list",
  "data": [
    {"id":"gpt-4","object":"model","created":1700000000,"owned_by":"openai"},
    {"id":"gpt-3.5-turbo","object":"model","created":1700000000,"owned_by":"openai"}
  ]
}"#;

/// OpenAI 单 model 获取。
pub const OPENAI_MODEL_GET: &str =
    r#"{"id":"gpt-4","object":"model","created":1700000000,"owned_by":"openai"}"#;

// ===== 断言辅助 =====

/// 轮询 usage.jsonl 直到至少 `min_lines` 行可解析为 JSON，或超时（3s）。
/// writer 异步（mpsc + tokio task），故需带超时重试。返回按出现序的 Value 列表。
pub async fn poll_usage_jsonl(path: &Path, min_lines: usize) -> Vec<serde_json::Value> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
            let vals: Vec<serde_json::Value> = content
                .lines()
                .filter(|l| !l.is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            if vals.len() >= min_lines {
                return vals;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("usage.jsonl records not written within 3s timeout")
}

/// 在 mock 记录的请求头 map 中做 case-insensitive 查找。返回匹配值。
pub fn recorded_header_get<'a>(
    headers: &'a HashMap<String, String>,
    name: &str,
) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}
