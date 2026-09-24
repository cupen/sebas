//! `sebas fake-provider`：本地 Anthropic `/v1/messages` 假上游（fake-provider-upstream）。
//!
//! 供进程级 e2e / 本地演示使用：零 token、确定性应答、可离线断言。设计与
//! `test_provider` 的区别：内置 debug `test` provider 在 router **内部**自答、
//! 不经过拨号路径；本模块是一个**真正可拨的 HTTP 上游**，router 以自定义
//! provider（`base_url_anthropic` 指向它）零改动接入，透传引擎（header 过滤 /
//! key 注入 / SSE 透传 / usage 结算）因此首次在进程级可验收。
//!
//! 三个能力面：
//!
//! - **内置确定性规则**（无剧本）：请求带非空 `tools` 且消息历史无 `tool_result`
//!   → 首个（优先只读/命令类）tool 的 `tool_use` 块，`stop_reason=tool_use`；
//!   已含 `tool_result` → 终文本（`end_turn`）；无 `tools` → 纯文本。
//! - **scenario 文件**（`--scenario`）：JSON 剧本按序消费，耗尽回落内置规则；
//!   条目可注入错误（状态码 + `retry-after`）。
//! - **journal**（`--journal`）：NDJSON 逐行记录收到的请求（method / path /
//!   headers / body），套件离线断言透传行为（上游 key 注入、下游 key 不泄漏）。
//!   **journal 属测试工件**：只应指向 dummy key 的 fake 上游，绝不可把生产
//!   凭据指向本服务。
//!
//! 只实现 Anthropic 协议面（design D5：OpenAI 面留后续 change）。auth header
//! 不校验具体值。

use std::collections::VecDeque;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::anthropic_wire::{AnthropicMessage, AnthropicUsage, text_block, tool_use_block};

/// 内置规则的确定性 usage（spec「确定性 usage 计量」：默认值固定且非零）。
pub const DEFAULT_INPUT_TOKENS: u64 = 12;
pub const DEFAULT_OUTPUT_TOKENS: u64 = 7;

/// 无 `tools` 请求的确定性文本应答。
pub const PLAIN_TEXT: &str = "fake-provider: no tools requested";
/// 消息历史已含 `tool_result` 后的终局文本（agent 工具环收敛点）。
pub const FINAL_TEXT: &str = "fake-provider: tool loop complete";
/// 内置规则 tool_use 的确定性块 id。
pub const TOOL_USE_ID: &str = "toolu_fake_1";

/// 请求体上限（假上游只服务测试，不需要 router 级配额；超限读失败按空 body 处理）。
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// `sebas fake-provider` 的启动参数（主 crate 薄壳动词的载荷）。
#[derive(Debug, Clone)]
pub struct FakeProviderConfig {
    /// 监听地址；`127.0.0.1:0` = 系统分配随机端口（spec「随机端口启动可见」）。
    pub listen: String,
    /// 可选 JSON 剧本（缺失/非法 → 启动失败）。
    pub scenario: Option<PathBuf>,
    /// 可选 NDJSON 请求留痕文件（父目录自动创建）。
    pub journal: Option<PathBuf>,
}

impl Default for FakeProviderConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:0".into(),
            scenario: None,
            journal: None,
        }
    }
}

/// 启动/服务期错误。`Display` 即 CLI `startup-failure:` 摘要的可读原因。
#[derive(Debug, Error)]
pub enum FakeProviderError {
    #[error("scenario file {path}: {reason}")]
    Scenario { path: String, reason: String },
    #[error("bind {listen}: {source}")]
    Bind {
        listen: String,
        source: std::io::Error,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serve: {0}")]
    Serve(String),
}

// ---------------------------------------------------------------------------
// scenario 文件（spec「scenario 文件编排」）
// ---------------------------------------------------------------------------

/// 剧本根：`{"responses":[...]}`（也接受顶层数组，便于手写短剧本）。
/// 未知字段宽松忽略（serde 默认行为）——加字段不破坏旧剧本。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Scenario {
    #[serde(default)]
    pub responses: Vec<ScenarioEntry>,
}

/// 一条预置应答。`kind` 缺省时按字段推断：有 `status` ≥400 → error；
/// 有 `tool` → tool_use；否则 text。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScenarioEntry {
    /// `"text"` / `"tool_use"` / `"error"`。
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    /// tool_use 的 tool 名（缺省回落到内置选择规则）。
    #[serde(default)]
    pub tool: Option<String>,
    /// tool_use 的 input（缺省由 tool 名给确定性最小对象）。
    #[serde(default)]
    pub input: Option<Value>,
    /// 覆盖 stop_reason（text 缺省 `end_turn`，tool_use 恒 `tool_use`）。
    #[serde(default)]
    pub stop_reason: Option<String>,
    /// usage 覆盖（未给的字段取确定性默认）。
    #[serde(default)]
    pub usage: Option<ScenarioUsage>,
    /// 错误注入状态码（error 条目）。
    #[serde(default)]
    pub status: Option<u16>,
    /// `retry-after` 响应头值（数字或字符串原样透传）。
    #[serde(default)]
    pub retry_after: Option<Value>,
    /// 错误注入的完整响应体（缺省给 Anthropic 错误形状）。
    #[serde(default)]
    pub body: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct ScenarioUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
}

impl ScenarioEntry {
    fn resolved_kind(&self) -> &str {
        if let Some(k) = self.kind.as_deref()
            && !k.is_empty()
        {
            return match k {
                "tool_use" => "tool_use",
                "error" => "error",
                _ => "text",
            };
        }
        if self.status.is_some_and(|s| s >= 400) {
            "error"
        } else if self.tool.is_some() {
            "tool_use"
        } else {
            "text"
        }
    }
}

/// 读取并解析剧本。`None` → 空剧本（直接走内置规则）；文件缺失/非法 JSON
/// → `Err`（启动失败语义）。
pub fn load_scenario(path: Option<&Path>) -> Result<Scenario, FakeProviderError> {
    let Some(path) = path else {
        return Ok(Scenario::default());
    };
    let raw = std::fs::read_to_string(path).map_err(|e| FakeProviderError::Scenario {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    let value: Value = serde_json::from_str(&raw).map_err(|e| FakeProviderError::Scenario {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    if value.is_array() {
        let responses: Vec<ScenarioEntry> =
            serde_json::from_value(value).map_err(|e| FakeProviderError::Scenario {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;
        return Ok(Scenario { responses });
    }
    serde_json::from_value(value).map_err(|e| FakeProviderError::Scenario {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

// ---------------------------------------------------------------------------
// 内置确定性规则（spec「内置 agent-loop 确定性规则」）
// ---------------------------------------------------------------------------
//
// 规则判断本身已下沉到 `crate::agent_loop`（extend-test-model-scenarios 2.6：
// 与 test 模型共用同一事实源，避免两处语义漂移）；此处保留同名 re-export，
// 既有 `fake_provider::pick_tool` 等调用路径不变。文本常量与块 id 仍是本模块
// 私有的契约（fake 上游的应答文案与 test 模型不同，判断同规范）。

pub use crate::agent_loop::{
    deterministic_input, has_tool_result, pick_tool, tool_schema, tools_of,
};

/// 请求是否要求流式（body 的 `stream` 布尔）。
pub fn wants_stream(body: Option<&Value>) -> bool {
    body.and_then(|v| v.get("stream"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 内置规则 → `(content 块数组, stop_reason)`。相同请求恒相同输出。
pub fn builtin_response(body: Option<&Value>) -> (Vec<Value>, String) {
    match crate::agent_loop::loop_reply(body) {
        crate::agent_loop::LoopReply::ToolUse { name, input } => (
            vec![tool_use_block(TOOL_USE_ID, &name, input)],
            "tool_use".to_string(),
        ),
        crate::agent_loop::LoopReply::FinalText => {
            (vec![text_block(FINAL_TEXT)], "end_turn".to_string())
        }
        crate::agent_loop::LoopReply::PlainText => {
            (vec![text_block(PLAIN_TEXT)], "end_turn".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 引擎：scenario 消费 → 内置回落 + journal
// ---------------------------------------------------------------------------

/// 引擎产出的响应计划（纯数据，便于单测；axum handler 只做转译）。
#[derive(Debug, Clone, PartialEq)]
pub enum NextResponse {
    Json {
        status: StatusCode,
        retry_after: Option<String>,
        body: String,
    },
    Sse {
        body: String,
    },
}

impl NextResponse {
    pub fn into_http(self) -> Response {
        match self {
            NextResponse::Json {
                status,
                retry_after,
                body,
            } => {
                let mut builder = Response::builder()
                    .status(status)
                    .header("content-type", "application/json");
                if let Some(retry_after) = retry_after {
                    builder = builder.header("retry-after", retry_after);
                }
                builder
                    .body(Body::from(body))
                    .expect("static response parts valid")
            }
            NextResponse::Sse { body } => Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .body(Body::from(body))
                .expect("static response parts valid"),
        }
    }
}

/// 假上游引擎：剧本按序弹出（耗尽回落内置规则）+ journal 追加。
pub struct Engine {
    scenario: Mutex<VecDeque<ScenarioEntry>>,
    journal: Option<PathBuf>,
    journal_lock: Mutex<()>,
}

impl Engine {
    /// 建引擎。journal 父目录缺失则创建；失败 → Err（启动失败语义）。
    pub fn new(scenario: Scenario, journal: Option<PathBuf>) -> Result<Self, FakeProviderError> {
        if let Some(path) = &journal
            && let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            scenario: Mutex::new(scenario.responses.into()),
            journal,
            journal_lock: Mutex::new(()),
        })
    }

    /// 处理一次请求，返回响应计划。副作用：消费一条剧本 + 追加一条 journal。
    pub fn respond(
        &self,
        method: &str,
        path: &str,
        headers: &HeaderMap,
        raw_body: &[u8],
    ) -> NextResponse {
        self.journalize(method, path, headers, raw_body);
        let parsed: Option<Value> = serde_json::from_slice(raw_body).ok();
        let stream = wants_stream(parsed.as_ref());
        let entry = self
            .scenario
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front();
        match entry {
            Some(entry) => self.render_scenario(entry, parsed.as_ref(), stream),
            None => {
                let (content, stop_reason) = builtin_response(parsed.as_ref());
                let message = AnthropicMessage {
                    id: "msg_fake_builtin".into(),
                    model: request_model(parsed.as_ref()).unwrap_or_else(|| "fake-model".into()),
                    content,
                    stop_reason,
                    usage: default_usage(),
                    // fake 上游保持单帧 delta 的历史形状（不分块）。
                    chunk_size: None,
                };
                if stream {
                    NextResponse::Sse {
                        body: message.sse(),
                    }
                } else {
                    NextResponse::Json {
                        status: StatusCode::OK,
                        retry_after: None,
                        body: message.json(),
                    }
                }
            }
        }
    }

    fn render_scenario(
        &self,
        entry: ScenarioEntry,
        parsed: Option<&Value>,
        stream: bool,
    ) -> NextResponse {
        match entry.resolved_kind() {
            "error" => {
                let status = entry
                    .status
                    .and_then(|s| StatusCode::from_u16(s).ok())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let body = entry.body.unwrap_or_else(|| error_body(status));
                NextResponse::Json {
                    status,
                    retry_after: entry.retry_after.map(retry_after_string),
                    body: body.to_string(),
                }
            }
            "tool_use" => {
                let tool = entry
                    .tool
                    .clone()
                    .or_else(|| {
                        parsed
                            .and_then(|v| v.get("tools"))
                            .and_then(Value::as_array)
                            .and_then(|t| pick_tool(t))
                            .map(|(name, _)| name)
                    })
                    .unwrap_or_else(|| "Bash".into());
                let input = entry.input.unwrap_or_else(|| deterministic_input(None));
                let message = AnthropicMessage {
                    id: "msg_fake_scenario".into(),
                    model: request_model(parsed).unwrap_or_else(|| "fake-model".into()),
                    content: vec![tool_use_block(TOOL_USE_ID, &tool, input)],
                    stop_reason: "tool_use".into(),
                    usage: entry
                        .usage
                        .map(usage_from_override)
                        .unwrap_or_else(default_usage),
                    // fake 上游保持单帧 delta 的历史形状（不分块）。
                    chunk_size: None,
                };
                if stream {
                    NextResponse::Sse {
                        body: message.sse(),
                    }
                } else {
                    NextResponse::Json {
                        status: StatusCode::OK,
                        retry_after: None,
                        body: message.json(),
                    }
                }
            }
            _ => {
                let text = entry.text.unwrap_or_else(|| PLAIN_TEXT.to_string());
                let stop_reason = entry.stop_reason.unwrap_or_else(|| "end_turn".to_string());
                let message = AnthropicMessage::text(
                    "msg_fake_scenario",
                    request_model(parsed).unwrap_or_else(|| "fake-model".into()),
                    &text,
                    stop_reason,
                    entry
                        .usage
                        .map(usage_from_override)
                        .unwrap_or_else(default_usage),
                );
                if stream {
                    NextResponse::Sse {
                        body: message.sse(),
                    }
                } else {
                    NextResponse::Json {
                        status: StatusCode::OK,
                        retry_after: None,
                        body: message.json(),
                    }
                }
            }
        }
    }

    /// NDJSON 追加一条请求记录（method / path / headers / body）。写失败只
    /// warn——留痕是旁路，绝不阻断应答。
    fn journalize(&self, method: &str, path: &str, headers: &HeaderMap, raw_body: &[u8]) {
        let Some(path_out) = &self.journal else {
            return;
        };
        let mut hdrs = serde_json::Map::new();
        for (name, value) in headers.iter() {
            let value = value.to_str().unwrap_or("<binary>");
            let key = name.as_str().to_string();
            match hdrs.get_mut(&key) {
                Some(Value::String(existing)) => {
                    existing.push_str(", ");
                    existing.push_str(value);
                }
                _ => {
                    hdrs.insert(key, Value::String(value.to_string()));
                }
            }
        }
        let body = serde_json::from_slice::<Value>(raw_body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(raw_body).to_string()));
        let line = json!({
            "method": method,
            "path": path,
            "headers": Value::Object(hdrs),
            "body": body,
        })
        .to_string();
        let _guard = self.journal_lock.lock().unwrap_or_else(|e| e.into_inner());
        let write = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path_out)
            .and_then(|mut f| writeln!(f, "{line}"));
        if let Err(e) = write {
            tracing::warn!(path = %path_out.display(), error = %e, "fake-provider journal write failed");
        }
    }
}

fn request_model(body: Option<&Value>) -> Option<String> {
    body.and_then(|v| v.get("model"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn default_usage() -> AnthropicUsage {
    AnthropicUsage {
        input_tokens: DEFAULT_INPUT_TOKENS,
        output_tokens: DEFAULT_OUTPUT_TOKENS,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    }
}

fn usage_from_override(o: ScenarioUsage) -> AnthropicUsage {
    let d = default_usage();
    AnthropicUsage {
        input_tokens: o.input_tokens.unwrap_or(d.input_tokens),
        output_tokens: o.output_tokens.unwrap_or(d.output_tokens),
        ..d
    }
}

fn retry_after_string(v: Value) -> String {
    match v {
        Value::String(s) => s,
        other => other.to_string(),
    }
}

/// 错误注入的默认 Anthropic 错误体。
fn error_body(status: StatusCode) -> Value {
    let err_type = match status.as_u16() {
        429 => "rate_limit_error",
        400 | 404 | 422 => "invalid_request_error",
        _ => "api_error",
    };
    json!({
        "type": "error",
        "error": {
            "type": err_type,
            "message": format!("fake-provider injected error {}", status.as_u16()),
        },
    })
}

// ---------------------------------------------------------------------------
// HTTP 面
// ---------------------------------------------------------------------------

/// 组装 axum router（`POST /v1/messages`）。auth header 不校验。
pub fn build_router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/v1/messages", post(handle_messages))
        .with_state(engine)
}

async fn handle_messages(State(engine): State<Arc<Engine>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let bytes = to_bytes(body, MAX_BODY_BYTES).await.unwrap_or_default();
    engine
        .respond(
            parts.method.as_str(),
            parts.uri.path(),
            &parts.headers,
            &bytes,
        )
        .into_http()
}

/// 启动 fake 上游：加载剧本（失败 = 启动失败）→ bind → stdout 打可解析的
/// ready 行 → 持续应答直到 ctrl_c / SIGTERM。
pub async fn run(cfg: FakeProviderConfig) -> Result<(), FakeProviderError> {
    let scenario = load_scenario(cfg.scenario.as_deref())?;
    let engine = Arc::new(Engine::new(scenario, cfg.journal.clone())?);
    let listener = tokio::net::TcpListener::bind(&cfg.listen)
        .await
        .map_err(|source| FakeProviderError::Bind {
            listen: cfg.listen.clone(),
            source,
        })?;
    let addr = listener
        .local_addr()
        .map_err(|source| FakeProviderError::Bind {
            listen: cfg.listen.clone(),
            source,
        })?;
    announce(addr);
    serve(listener, engine, shutdown_signal()).await
}

/// bind 成功后 **stdout 单行** ready 信号（harness 解析该行拿端口；与 router
/// 的 `sebas router listening addr=…` 同款机制）。
fn announce(addr: SocketAddr) {
    println!("fake-provider listening addr={addr}");
    let _ = std::io::stdout().flush();
}

/// 在给定 listener 上服务，`shutdown` 触发后优雅退出。测试与嵌入共用。
pub async fn serve(
    listener: tokio::net::TcpListener,
    engine: Arc<Engine>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), FakeProviderError> {
    let app = build_router(engine);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| FakeProviderError::Serve(e.to_string()))
}

/// 等待 ctrl_c（全平台）或 SIGTERM（unix）。首个信号生效。
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl_c handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("fake-provider shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use serde_json::json;

    fn engine(scenario: Scenario, journal: Option<PathBuf>) -> Arc<Engine> {
        Arc::new(Engine::new(scenario, journal).expect("engine"))
    }

    fn headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", "sk-upstream-fake".parse().unwrap());
        h.insert("anthropic-version", "2023-06-01".parse().unwrap());
        h
    }

    fn json_body(v: Value) -> Bytes {
        Bytes::from(v.to_string())
    }

    fn parse(resp: &NextResponse) -> Value {
        match resp {
            NextResponse::Json { body, .. } => serde_json::from_str(body).expect("json body"),
            NextResponse::Sse { body } => {
                serde_json::from_str(body).unwrap_or_else(|_| json!({"sse": body}))
            }
        }
    }

    // ---------------- 1.3 内置规则三路径 + 同请求同应答 ----------------

    #[test]
    fn builtin_rule_tool_use_when_tools_and_no_tool_result() {
        let e = engine(Scenario::default(), None);
        let body = json!({
            "model": "fake-model",
            "tools": [{"name": "Bash", "input_schema": {"type": "object", "required": ["command"], "properties": {"command": {"type": "string"}}}}],
            "messages": [{"role": "user", "content": "hi"}]
        });
        let resp = e.respond("POST", "/v1/messages", &headers(), &json_body(body));
        let v = parse(&resp);
        assert_eq!(v["stop_reason"], "tool_use");
        assert_eq!(v["content"][0]["type"], "tool_use");
        assert_eq!(v["content"][0]["name"], "Bash");
        assert_eq!(v["content"][0]["input"]["command"], "echo ok");
        assert_eq!(v["usage"]["input_tokens"], DEFAULT_INPUT_TOKENS);
        assert_eq!(v["usage"]["output_tokens"], DEFAULT_OUTPUT_TOKENS);
    }

    #[test]
    fn builtin_rule_final_text_after_tool_result() {
        let e = engine(Scenario::default(), None);
        let body = json!({
            "model": "fake-model",
            "tools": [{"name": "Bash", "input_schema": {"type": "object", "required": ["command"], "properties": {"command": {"type": "string"}}}}],
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_fake_1", "name": "Bash", "input": {"command": "echo ok"}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_fake_1", "content": "ok"}]}
            ]
        });
        let resp = e.respond("POST", "/v1/messages", &headers(), &json_body(body));
        let v = parse(&resp);
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][0]["text"], FINAL_TEXT);
    }

    #[test]
    fn builtin_rule_plain_text_without_tools() {
        let e = engine(Scenario::default(), None);
        let body = json!({"model": "fake-model", "messages": [{"role": "user", "content": "hi"}]});
        let resp = e.respond("POST", "/v1/messages", &headers(), &json_body(body));
        let v = parse(&resp);
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["content"][0]["text"], PLAIN_TEXT);
    }

    #[test]
    fn builtin_rule_is_deterministic_for_same_request() {
        let e = engine(Scenario::default(), None);
        let body = json!({
            "model": "fake-model",
            "tools": [{"name": "Task", "input_schema": {"type": "object", "required": ["description", "subagent_type"], "properties": {"description": {"type": "string"}, "subagent_type": {"type": "string", "enum": ["general-purpose", "explore"]}}}}],
            "messages": [{"role": "user", "content": "hi"}]
        });
        let a = parse(&e.respond("POST", "/v1/messages", &headers(), &json_body(body.clone())));
        let b = parse(&e.respond("POST", "/v1/messages", &headers(), &json_body(body)));
        assert_eq!(a, b, "同请求必须同应答");
    }

    #[test]
    fn pick_tool_prefers_read_then_command_tools() {
        let tools = vec![
            json!({"name": "Task", "input_schema": {"type": "object"}}),
            json!({"name": "Bash", "input_schema": {"type": "object"}}),
            json!({"name": "Read", "input_schema": {"type": "object"}}),
        ];
        let (name, _) = pick_tool(&tools).expect("a tool");
        assert_eq!(name, "Read", "只读类优先于命令类");
        let (name, _) = pick_tool(&tools[..2]).expect("a tool");
        assert_eq!(name, "Bash");
        let (name, _) = pick_tool(&tools[..1]).expect("a tool");
        assert_eq!(name, "Task", "无偏好命中取首个");
        assert!(pick_tool(&[]).is_none());
    }

    #[test]
    fn deterministic_input_fills_required_by_type_and_enum() {
        let schema = json!({
            "type": "object",
            "required": ["path", "lines", "force", "tags", "meta"],
            "properties": {
                "path": {"type": "string"},
                "lines": {"type": "integer"},
                "force": {"type": "boolean"},
                "tags": {"type": "array"},
                "meta": {"type": "object"}
            }
        });
        let input = deterministic_input(Some(&schema));
        assert_eq!(input["path"], "ok");
        assert_eq!(input["lines"], 0);
        assert_eq!(input["force"], false);
        assert_eq!(input["tags"], json!([]));
        assert_eq!(input["meta"], json!({}));
        // 枚举取首值；无 required → 空对象；无 schema → 空对象。
        assert_eq!(
            deterministic_input(Some(
                &json!({"required": ["x"], "properties": {"x": {"enum": ["a", "b"]}}})
            )),
            json!({"x": "a"})
        );
        assert_eq!(
            deterministic_input(Some(&json!({"type": "object"}))),
            json!({})
        );
        assert_eq!(deterministic_input(None), json!({}));
    }

    // ---------------- 1.4 流式应答 ----------------

    #[test]
    fn stream_request_returns_sse_with_same_text_as_json() {
        let e = engine(Scenario::default(), None);
        let base = json!({"model": "fake-model", "messages": [{"role": "user", "content": "hi"}]});
        let non_stream =
            parse(&e.respond("POST", "/v1/messages", &headers(), &json_body(base.clone())));
        let mut streaming = base;
        streaming["stream"] = json!(true);
        let resp = e.respond("POST", "/v1/messages", &headers(), &json_body(streaming));
        let sse = match resp {
            NextResponse::Sse { body } => body,
            other => panic!("stream=true must answer SSE, got {other:?}"),
        };
        for event in [
            "event: message_start",
            "event: content_block_start",
            "event: content_block_delta",
            "event: content_block_stop",
            "event: message_delta",
            "event: message_stop",
        ] {
            assert!(sse.contains(event), "missing {event} in:\n{sse}");
        }
        assert!(
            sse.contains(PLAIN_TEXT),
            "SSE text must match non-stream text"
        );
        assert_eq!(
            non_stream["content"][0]["text"], PLAIN_TEXT,
            "non-stream parity"
        );
    }

    // ---------------- 1.5 scenario ----------------

    #[test]
    fn scenario_entries_are_consumed_in_order() {
        let scenario = Scenario {
            responses: vec![
                ScenarioEntry {
                    text: Some("first".into()),
                    ..Default::default()
                },
                ScenarioEntry {
                    tool: Some("Bash".into()),
                    input: Some(json!({"command": "echo ok"})),
                    ..Default::default()
                },
                ScenarioEntry {
                    text: Some("third".into()),
                    ..Default::default()
                },
            ],
        };
        let e = engine(scenario, None);
        let body = json!({"model": "m", "messages": []});
        let first = parse(&e.respond("POST", "/v1/messages", &headers(), &json_body(body.clone())));
        assert_eq!(first["content"][0]["text"], "first");
        assert_eq!(first["stop_reason"], "end_turn");

        let second =
            parse(&e.respond("POST", "/v1/messages", &headers(), &json_body(body.clone())));
        assert_eq!(second["content"][0]["type"], "tool_use");
        assert_eq!(second["content"][0]["name"], "Bash");
        assert_eq!(second["stop_reason"], "tool_use");

        let third = parse(&e.respond("POST", "/v1/messages", &headers(), &json_body(body)));
        assert_eq!(third["content"][0]["text"], "third");
    }

    #[test]
    fn scenario_error_injection_carries_status_and_retry_after() {
        let scenario = Scenario {
            responses: vec![
                ScenarioEntry {
                    status: Some(429),
                    retry_after: Some(json!(3)),
                    ..Default::default()
                },
                ScenarioEntry {
                    text: Some("after".into()),
                    ..Default::default()
                },
            ],
        };
        let e = engine(scenario, None);
        let body = json!({"model": "m", "messages": []});
        let resp = e.respond("POST", "/v1/messages", &headers(), &json_body(body.clone()));
        match &resp {
            NextResponse::Json {
                status,
                retry_after,
                body,
            } => {
                assert_eq!(*status, StatusCode::TOO_MANY_REQUESTS);
                assert_eq!(retry_after.as_deref(), Some("3"));
                let v: Value = serde_json::from_str(body).unwrap();
                assert_eq!(v["error"]["type"], "rate_limit_error");
            }
            other => panic!("error injection must be JSON, got {other:?}"),
        }
        // 后续请求不受影响（剧本继续按序）。
        let next = parse(&e.respond("POST", "/v1/messages", &headers(), &json_body(body)));
        assert_eq!(next["content"][0]["text"], "after");
    }

    #[test]
    fn scenario_exhaustion_falls_back_to_builtin_rules() {
        let scenario = Scenario {
            responses: vec![ScenarioEntry {
                text: Some("only".into()),
                ..Default::default()
            }],
        };
        let e = engine(scenario, None);
        let with_tools = json!({
            "model": "m",
            "tools": [{"name": "Read", "input_schema": {"type": "object", "required": ["file_path"], "properties": {"file_path": {"type": "string"}}}}],
            "messages": []
        });
        let first = parse(&e.respond(
            "POST",
            "/v1/messages",
            &headers(),
            &json_body(with_tools.clone()),
        ));
        assert_eq!(first["content"][0]["text"], "only");
        let second = parse(&e.respond("POST", "/v1/messages", &headers(), &json_body(with_tools)));
        assert_eq!(
            second["content"][0]["type"], "tool_use",
            "耗尽后回落内置规则"
        );
        assert_eq!(second["content"][0]["name"], "Read");
    }

    #[test]
    fn load_scenario_accepts_object_and_array_and_rejects_bad_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let object = dir.path().join("object.json");
        std::fs::write(&object, r#"{"responses":[{"text":"hi"}]}"#).unwrap();
        let s = load_scenario(Some(&object)).expect("object form");
        assert_eq!(s.responses.len(), 1);

        let array = dir.path().join("array.json");
        std::fs::write(&array, r#"[{"text":"a"},{"kind":"error","status":500}]"#).unwrap();
        let s = load_scenario(Some(&array)).expect("array form");
        assert_eq!(s.responses.len(), 2);
        assert_eq!(s.responses[1].resolved_kind(), "error");

        // 未知字段宽松忽略（前向兼容）。
        let future = dir.path().join("future.json");
        std::fs::write(&future, r#"{"responses":[{"text":"x","future_field":42}]}"#).unwrap();
        assert!(load_scenario(Some(&future)).is_ok());

        // 文件不存在 / 非法 JSON → 启动失败。
        assert!(matches!(
            load_scenario(Some(&dir.path().join("missing.json"))),
            Err(FakeProviderError::Scenario { .. })
        ));
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{not json").unwrap();
        assert!(matches!(
            load_scenario(Some(&bad)),
            Err(FakeProviderError::Scenario { .. })
        ));
        // 未配置 → 空剧本。
        assert!(load_scenario(None).unwrap().responses.is_empty());
    }

    // ---------------- 1.6 journal ----------------

    #[test]
    fn journal_appends_one_parseable_line_per_request() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fake-journal.jsonl");
        let e = engine(Scenario::default(), Some(path.clone()));
        let body = json!({"model": "fake/x", "messages": []});
        e.respond("POST", "/v1/messages", &headers(), &json_body(body.clone()));
        e.respond("POST", "/v1/messages", &headers(), &json_body(body));

        let content = std::fs::read_to_string(&path).expect("journal exists");
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2, "两项请求恰两行: {content}");
        for line in lines {
            let v: Value = serde_json::from_str(line).expect("journal line is JSON");
            assert_eq!(v["method"], "POST");
            assert_eq!(v["path"], "/v1/messages");
            assert_eq!(v["headers"]["x-api-key"], "sk-upstream-fake");
            assert_eq!(v["headers"]["anthropic-version"], "2023-06-01");
            assert_eq!(v["body"]["model"], "fake/x");
        }
    }

    #[test]
    fn journal_records_non_json_body_as_string() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("j.jsonl");
        let e = engine(Scenario::default(), Some(path.clone()));
        e.respond("POST", "/v1/messages", &headers(), b"not json");
        let content = std::fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(v["body"], "not json");
    }

    // ---------------- 1.2 HTTP 面（in-process：bind + 拨号） ----------------

    #[tokio::test]
    async fn http_messages_serves_non_stream_json_and_releases_port() {
        let e = engine(Scenario::default(), None);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(serve(listener, e, std::future::pending::<()>()));

        let cli = reqwest::Client::new();
        let resp = cli
            .post(format!("http://{addr}/v1/messages"))
            .header("content-type", "application/json")
            .header("x-api-key", "sk-whatever")
            .body(r#"{"model":"fake-model","messages":[{"role":"user","content":"hi"}]}"#)
            .send()
            .await
            .expect("dial fake upstream");
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let text = resp.text().await.expect("body text");
        let v: Value = serde_json::from_str(&text).expect("json body");
        assert_eq!(v["content"][0]["text"], PLAIN_TEXT);

        // SIGTERM/SIGINT 之外的拆卸路径：abort 后端口立即可再绑定。
        handle.abort();
        let _ = handle.await;
        tokio::net::TcpListener::bind(addr)
            .await
            .expect("port must be released after teardown");
    }

    #[tokio::test]
    async fn http_error_injection_carries_status_and_retry_after_header() {
        let scenario = Scenario {
            responses: vec![ScenarioEntry {
                status: Some(429),
                retry_after: Some(json!("7")),
                ..Default::default()
            }],
        };
        let e = engine(scenario, None);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(serve(listener, e, std::future::pending::<()>()));

        let resp = reqwest::Client::new()
            .post(format!("http://{addr}/v1/messages"))
            .header("content-type", "application/json")
            .body(r#"{"model":"m","messages":[]}"#)
            .send()
            .await
            .expect("dial");
        assert_eq!(resp.status().as_u16(), 429);
        assert_eq!(
            resp.headers()
                .get("retry-after")
                .map(|v| v.to_str().unwrap()),
            Some("7"),
            "scenario retry-after must be transmitted"
        );
        let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
        assert_eq!(body["error"]["type"], "rate_limit_error");
        handle.abort();
        let _ = handle.await;
    }

    // ---------------- 1.2 ready 行解析契约 ----------------

    #[test]
    fn ready_line_shape_is_parseable() {
        // harness 解析契约：单行 `fake-provider listening addr=127.0.0.1:<port>`。
        let addr: SocketAddr = "127.0.0.1:34567".parse().unwrap();
        let line = format!("fake-provider listening addr={addr}");
        let idx = line.find("addr=").expect("addr= present");
        let parsed: SocketAddr = line[idx + 5..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(parsed, addr);
    }
}
