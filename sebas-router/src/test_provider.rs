//! 内置 test provider（`--debug` / `[router] debug = true`）。
//!
//! debug 模式给 router 增加一个自定义模型 `test`：请求不转发到外部上游，
//! 而是由 router 自身应答。**模型名携带场景**（extend-test-model-scenarios
//! D1）：`test/<scenario>` 选一段确定性剧本，bare `test` 保持既有 echo 行为
//! **逐字不变**（AGENTS.md 食谱与既有 e2e 的已文档化契约）。
//!
//! Anthropic（/v1/messages）、OpenAI chat（/v1/chat/completions）与
//! OpenAI Responses（/v1/responses）三个协议面、流式与非流式都支持
//! （OpenAI 家族是**降级面**：thinking 静默降为纯文本，design D5）。
//!
//! 场景（九个，含 bare `test`）：
//!
//! | 模型 | 形状 | 驱动的工作台能力 |
//! |---|---|---|
//! | `test` | 固定文字 + 回显最后一条用户消息（`msg_test_debug`） | 既有 debug 应答契约 |
//! | `test/text` | 同上（场景 id） | 正文呈现、对话连续性（echo） |
//! | `test/long` | 确定性长文，流式按固定粒度分块滴流 | 流式背压/增量、流中取消 |
//! | `test/thinking` | thinking + 正文 | thinking 呈现 |
//! | `test/tool-use` | 工具环（与 fake 上游同规范） | 权限三值流、工具环到 Done |
//! | `test/tools-parallel` | 一回合每个声明 tool 一个 tool_use | 并行权限请求 |
//! | `test/full` | thinking + 正文 + tool_use 混排 | 块序/呈现区分 |
//! | `test/empty` | 零 content 块完成回合 | 零输出通知 |
//! | `test/error` | Anthropic 错误体（HTTP 5xx api_error） | 失败呈现/诚实降级 |

use std::time::Duration;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::Value;

use crate::agent_loop;
use crate::anthropic_wire::{
    AnthropicMessage, AnthropicUsage, text_block, thinking_block, tool_use_block,
};
use crate::proto::WireProtocol;
use crate::sse::UsageInfo;

/// 回显文案：`I'm test provider. I received your message "<echo>".`
pub fn test_message(echo: &str) -> String {
    format!("I'm test provider. I received your message \"{echo}\".")
}

/// debug provider 的固定 id / model（bare `test` 的既有契约，逐字不变）。
const DEBUG_MSG_ID: &str = "msg_test_debug";
const DEBUG_MODEL: &str = "test";

/// 场景应答的块 id 前缀（bare `test` 用 `DEBUG_MSG_ID`）。
const SCENARIO_MSG_PREFIX: &str = "msg_test_";

/// `test/long` 的确定性长文（固定内容；流式按 [`LONG_CHUNK`] 个字符分块）。
/// 39 行 × 26 字符 ≈ 1KB，分块数固定 → 「增量拼接结果」可精确断言。
pub const LONG_TEXT_HEADER: &str = "sebas test provider long response\n";

/// `test/long` 长文的确定性主体（每行 26 个字母，行号固定）。
fn long_text() -> String {
    let mut out = String::from(LONG_TEXT_HEADER);
    for line in 0..39u32 {
        for col in 0..26u32 {
            let c = char::from(b'a' + ((line * 26 + col) % 26) as u8);
            out.push(c);
        }
        out.push('\n');
    }
    out
}

/// `test/long` 的流式分块粒度（字符）——确定性「固定分块数」的来源。
pub const LONG_CHUNK: usize = 32;

/// `test/long` 流式每帧之间的节流间隔。存在理由：短回路里整个 SSE 体一次
/// 到达，「流式期间取消」「流中增量呈现」都无从发生——滴流让取消落在流中间
/// （design D4/D7；真实上游也有网络节奏，debug 面把它变成确定性的）。
pub const LONG_FRAME_INTERVAL: Duration = Duration::from_millis(60);

/// 场景模型的流式分块粒度（thinking / text / tool input）。
const SCENARIO_CHUNK: usize = 8;

/// `test/thinking` 的确定性 thinking 文本与签名。
const THINKING_TEXT: &str =
    "test provider thinking: the operator asked a question, so I reason briefly, then answer.";
const THINKING_SIGNATURE: &str = "sebas-test-thinking-signature";

/// `test/tool-use` / `test/full` 的确定性终文本。
const TOOL_FINAL_TEXT: &str = "test provider: tool loop complete.";
/// 无 tools 时的确定性纯文本（降级路径）。
const PLAIN_TEXT: &str = "test provider: no tools requested.";
/// tool_use 块的确定性 id（`test/tool-use` / `test/full` / `test/tools-parallel`）。
const TOOL_USE_ID_PREFIX: &str = "toolu_test_";

/// `test/error` 的确定性错误文案（Anthropic 错误体 + OpenAI 家族错误体共用）。
pub const ERROR_MESSAGE: &str = "test provider: injected upstream failure (api_error)";

/// 场景枚举。`test/<scenario>` 的 `<scenario>` 段；未知段回落 [`Scenario::Echo`]
/// （见 [`scenario_from_model`] 的注释）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// bare `test`：既有 echo（逐字不变）。
    Echo,
    Text,
    Long,
    Thinking,
    ToolUse,
    ToolsParallel,
    Full,
    Empty,
    Error,
}

/// 场景名 → 枚举（`test/<name>` 的 `<name>` 段）。未知名字**回落 Echo**：
/// `test/<anything>` 自 debug 模式引入起就路由到这里并回显（AGENTS.md 食谱
/// 明写），保持该兜底比新增 404 面更安全（design 4.3 的实测选择）。
pub fn scenario_from_name(name: &str) -> Scenario {
    match name {
        "text" => Scenario::Text,
        "long" => Scenario::Long,
        "thinking" => Scenario::Thinking,
        "tool-use" => Scenario::ToolUse,
        "tools-parallel" => Scenario::ToolsParallel,
        "full" => Scenario::Full,
        "empty" => Scenario::Empty,
        "error" => Scenario::Error,
        // 未知 `test/<x>`（含空段）：既有回显兜底。
        _ => Scenario::Echo,
    }
}

/// 请求 body 的 `model` 字段 → 场景。
pub fn scenario_from_model(model: &str) -> Scenario {
    match model.strip_prefix("test/") {
        Some(rest) => scenario_from_name(rest),
        None => Scenario::Echo,
    }
}

/// 请求 body → 场景（`model` 缺失/非 JSON → [`Scenario::Echo`]）。
pub fn scenario_from_body(body: Option<&axum::body::Bytes>) -> Scenario {
    body.and_then(|b| serde_json::from_slice::<Value>(b.as_ref()).ok())
        .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
        .as_deref()
        .map(scenario_from_model)
        .unwrap_or(Scenario::Echo)
}

impl Scenario {
    /// 场景名（=`test/<name>` 的 `<name>`；Echo 无后缀）。
    pub fn name(self) -> &'static str {
        match self {
            Scenario::Echo => "test",
            Scenario::Text => "test/text",
            Scenario::Long => "test/long",
            Scenario::Thinking => "test/thinking",
            Scenario::ToolUse => "test/tool-use",
            Scenario::ToolsParallel => "test/tools-parallel",
            Scenario::Full => "test/full",
            Scenario::Empty => "test/empty",
            Scenario::Error => "test/error",
        }
    }

    /// 应答 message id（每场景固定 → 确定性与可区分性同时成立）。
    pub fn message_id(self) -> String {
        match self {
            Scenario::Echo => DEBUG_MSG_ID.to_string(),
            other => format!("{SCENARIO_MSG_PREFIX}{}", other.slug()),
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Scenario::Echo => "debug",
            Scenario::Text => "text",
            Scenario::Long => "long",
            Scenario::Thinking => "thinking",
            Scenario::ToolUse => "tool_use",
            Scenario::ToolsParallel => "tools_parallel",
            Scenario::Full => "full",
            Scenario::Empty => "empty",
            Scenario::Error => "error",
        }
    }

    /// HTTP 状态：`test/error` 5xx，其余 200。
    pub fn status(self) -> StatusCode {
        match self {
            Scenario::Error => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::OK,
        }
    }

    /// usage 记录里的 error 字段（仅 `test/error`）。
    pub fn error(self) -> Option<&'static str> {
        match self {
            Scenario::Error => Some(ERROR_MESSAGE),
            _ => None,
        }
    }

    /// 确定性非零 token 用量（design D6）：消息型场景各一组**互不相同**的值，
    /// 断言字段错位立即可见；bare `test` 与 `test/empty` 保持既有全零。
    pub fn usage(self) -> AnthropicUsage {
        match self {
            Scenario::Echo | Scenario::Empty | Scenario::Error => AnthropicUsage::default(),
            Scenario::Text => AnthropicUsage::new(11, 7, 3, 2),
            Scenario::Long => AnthropicUsage::new(13, 101, 5, 4),
            Scenario::Thinking => AnthropicUsage::new(17, 23, 7, 6),
            Scenario::ToolUse => AnthropicUsage::new(19, 29, 11, 8),
            Scenario::ToolsParallel => AnthropicUsage::new(23, 31, 13, 10),
            Scenario::Full => AnthropicUsage::new(29, 37, 17, 12),
        }
    }

    /// usage 记录面（sink）的计数：零值场景保持 `None`（bare `test` 的历史
    /// 形状，逐字不变），非零场景逐字段上报。
    pub fn usage_info(self) -> UsageInfo {
        let u = self.usage();
        if u.is_zero() {
            return UsageInfo::default();
        }
        UsageInfo {
            input_tokens: Some(u.input_tokens),
            output_tokens: Some(u.output_tokens),
            cache_read_tokens: Some(u.cache_read_input_tokens),
            cache_creation_tokens: Some(u.cache_creation_input_tokens),
        }
    }
}

/// 从 buffered 请求 body 提取「最后一条 user 消息的文本」用于回显。
/// 兼容 Anthropic / OpenAI 的 `messages` 数组：content 为字符串或 text 块数组。
/// 无 messages / 无 user 消息 → 空串。
pub fn echo_text(body: Option<&axum::body::Bytes>) -> String {
    let Some(bytes) = body else {
        return String::new();
    };
    let Ok(v) = serde_json::from_slice::<Value>(bytes.as_ref()) else {
        return String::new();
    };
    let Some(msgs) = v.get("messages").and_then(|m| m.as_array()) else {
        return String::new();
    };
    msgs.iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| content_text(m.get("content")))
        .unwrap_or_default()
}

/// 归一化 content 字段（字符串直接取；数组取各 text 块拼接）。
fn content_text(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(blocks)) => {
            let parts: Vec<&str> = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(""))
            }
        }
        _ => None,
    }
}

/// 请求是否要求流式（body 的 `stream` 布尔）。
pub fn wants_stream(body: Option<&axum::body::Bytes>) -> bool {
    body.and_then(|b| serde_json::from_slice::<Value>(b.as_ref()).ok())
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

/// bare `test` 的既有入口（签名与行为逐字不变）：固定文字 + 回显。
pub fn test_response(proto: WireProtocol, echoed: &str, stream: bool) -> Response {
    scenario_response(proto, Scenario::Echo, None, echoed, stream)
}

/// 场景应答入口：按场景选 Anthropic 块序列 / OpenAI 降级形状。
///
/// `body` 供工具环规则读取（`tools` / `tool_result`）；`None` = 无请求上下文
/// （等价于无 tools、无 tool_result 的纯文本路径）。`stream` 为真时
/// `test/long` 按帧滴流（其余场景一次性 body）。
pub fn scenario_response(
    proto: WireProtocol,
    scenario: Scenario,
    body: Option<&axum::body::Bytes>,
    echoed: &str,
    stream: bool,
) -> Response {
    if scenario == Scenario::Error {
        return error_response(proto);
    }
    if proto.is_openai_family() {
        return openai_scenario_response(proto, scenario, body, echoed, stream);
    }
    let message = anthropic_message(scenario, body, echoed);
    if !stream {
        return message.into_json_response();
    }
    if scenario != Scenario::Long {
        return message.into_sse_response();
    }
    // 滴流：每帧之间等 LONG_FRAME_INTERVAL（取消/增量呈现必须落在流中间）。
    let frames = message.sse_frames();
    let stream_body = futures_util::stream::unfold(frames.into_iter(), |mut it| async move {
        let frame = it.next()?;
        tokio::time::sleep(LONG_FRAME_INTERVAL).await;
        Some((Ok::<_, std::convert::Infallible>(frame), it))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream_body))
        .expect("static response parts valid")
}

/// 场景 → Anthropic message（块序列 / stop_reason / usage / 分块粒度）。
pub fn anthropic_message(
    scenario: Scenario,
    body: Option<&axum::body::Bytes>,
    echoed: &str,
) -> AnthropicMessage {
    let parsed: Option<Value> = body.and_then(|b| serde_json::from_slice(b.as_ref()).ok());
    let reply = test_message(echoed);
    let (content, stop_reason) = match scenario {
        Scenario::Echo | Scenario::Text => (vec![text_block(&reply)], "end_turn"),
        Scenario::Long => (vec![text_block(&long_text())], "end_turn"),
        Scenario::Thinking => (
            vec![
                thinking_block(THINKING_TEXT, Some(THINKING_SIGNATURE)),
                text_block(&reply),
            ],
            "end_turn",
        ),
        Scenario::Empty => (vec![], "end_turn"),
        Scenario::Error => unreachable!("error scenario answers with an error body"),
        Scenario::ToolUse => {
            let tools = agent_loop::tools_of(parsed.as_ref());
            if !tools.is_empty() && !agent_loop::has_tool_result(parsed.as_ref()) {
                match agent_loop::first_tool(&tools) {
                    Some((name, input)) => (
                        vec![tool_use_block(&format!("{TOOL_USE_ID_PREFIX}1"), &name, input)],
                        "tool_use",
                    ),
                    // tools 条目全无名 → 纯文本降级（不报错）。
                    None => (vec![text_block(PLAIN_TEXT)], "end_turn"),
                }
            } else {
                (vec![text_block(tool_final_text(parsed.as_ref()))], "end_turn")
            }
        }
        Scenario::ToolsParallel => {
            let tools = agent_loop::tools_of(parsed.as_ref());
            if !tools.is_empty() && !agent_loop::has_tool_result(parsed.as_ref()) {
                let blocks: Vec<Value> = agent_loop::parallel_tool_inputs(&tools)
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, input))| {
                        tool_use_block(&format!("{TOOL_USE_ID_PREFIX}p{i}"), &name, input)
                    })
                    .collect();
                if blocks.is_empty() {
                    (vec![text_block(PLAIN_TEXT)], "end_turn")
                } else {
                    (blocks, "tool_use")
                }
            } else {
                (vec![text_block(tool_final_text(parsed.as_ref()))], "end_turn")
            }
        }
        Scenario::Full => {
            let tools = agent_loop::tools_of(parsed.as_ref());
            if !tools.is_empty() && !agent_loop::has_tool_result(parsed.as_ref()) {
                match agent_loop::first_tool(&tools) {
                    Some((name, input)) => (
                        vec![
                            thinking_block(THINKING_TEXT, Some(THINKING_SIGNATURE)),
                            text_block(&reply),
                            tool_use_block(&format!("{TOOL_USE_ID_PREFIX}f1"), &name, input),
                        ],
                        "tool_use",
                    ),
                    None => (
                        vec![
                            thinking_block(THINKING_TEXT, Some(THINKING_SIGNATURE)),
                            text_block(PLAIN_TEXT),
                        ],
                        "end_turn",
                    ),
                }
            } else {
                // 终文本轮不带 thinking（design 开放问题 2 的实测选择）。
                (vec![text_block(tool_final_text(parsed.as_ref()))], "end_turn")
            }
        }
    };

    let chunk = match scenario {
        Scenario::Long => Some(LONG_CHUNK),
        Scenario::Echo | Scenario::Empty => None,
        _ => Some(SCENARIO_CHUNK),
    };
    let requested_model = parsed
        .as_ref()
        .and_then(|v| v.get("model"))
        .and_then(Value::as_str)
        .unwrap_or(DEBUG_MODEL);
    let mut message = AnthropicMessage::blocks(
        scenario.message_id(),
        requested_model,
        content,
        stop_reason,
        scenario.usage(),
    );
    if let Some(chunk) = chunk {
        message = message.with_chunk_size(chunk);
    }
    message
}

/// 工具环的终文本：历史已含 `tool_result` → 终文本；无 tools → 纯文本降级。
fn tool_final_text(body: Option<&Value>) -> &'static str {
    if agent_loop::has_tool_result(body) {
        TOOL_FINAL_TEXT
    } else {
        PLAIN_TEXT
    }
}

/// `test/error`：Anthropic 错误体（HTTP 5xx + `api_error`）。
fn error_response(proto: WireProtocol) -> Response {
    let status = Scenario::Error.status();
    let body = if proto.is_openai_family() {
        // OpenAI 家族的对应错误形状（`error.type`，不回 Anthropic 的
        // `type: error` 外壳）。
        serde_json::json!({
            "error": {
                "type": "api_error",
                "message": ERROR_MESSAGE,
                "code": "api_error",
            }
        })
    } else {
        serde_json::json!({
            "type": "error",
            "error": {"type": "api_error", "message": ERROR_MESSAGE},
        })
    };
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("static response parts valid")
}

// ---------------------------------------------------------------------------
// OpenAI 家族（chat / Responses）：降级面（design D5）
// ---------------------------------------------------------------------------

/// OpenAI 家族：所有块归并为纯文本（thinking 静默降级），tool_use 映射为
/// chat 的 `tool_calls` 对应物；usage 按 prompt/completion 映射。
fn openai_scenario_response(
    proto: WireProtocol,
    scenario: Scenario,
    body: Option<&axum::body::Bytes>,
    echoed: &str,
    stream: bool,
) -> Response {
    let message = anthropic_message(scenario, body, echoed);
    let text = openai_text(&message);
    let tool_calls = openai_tool_calls(&message);
    let finish = if message.stop_reason == "tool_use" {
        "tool_calls"
    } else {
        "stop"
    };
    if stream {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(Body::from(openai_sse_scenario(
                &message, &text, &tool_calls, finish, proto,
            )))
            .expect("static response parts valid")
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(openai_json_scenario(
                &message, &text, &tool_calls, finish,
            )))
            .expect("static response parts valid")
    }
}

/// 归并所有 text / thinking 块为纯文本（thinking 降级 = 文本，不报错）。
fn openai_text(message: &AnthropicMessage) -> String {
    let mut out = String::new();
    for block in &message.content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => out.push_str(block.get("text").and_then(Value::as_str).unwrap_or("")),
            Some("thinking") => {
                out.push_str(block.get("thinking").and_then(Value::as_str).unwrap_or(""))
            }
            _ => {}
        }
    }
    out
}

/// tool_use 块 → chat 协议的 `tool_calls` 数组（无 tool_use → `None`）。
fn openai_tool_calls(message: &AnthropicMessage) -> Option<Value> {
    let calls: Vec<Value> = message
        .content
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(|b| {
            serde_json::json!({
                "id": b.get("id").cloned().unwrap_or(Value::Null),
                "type": "function",
                "function": {
                    "name": b.get("name").cloned().unwrap_or(Value::Null),
                    "arguments": b.get("input").cloned().unwrap_or_else(|| serde_json::json!({})).to_string(),
                },
            })
        })
        .collect();
    (!calls.is_empty()).then(|| Value::Array(calls))
}

fn openai_json_scenario(
    message: &AnthropicMessage,
    text: &str,
    tool_calls: &Option<Value>,
    finish: &str,
) -> String {
    let mut body = serde_json::json!({
        "id": "chatcmpl-test-debug",
        "object": "chat.completion",
        "created": 0,
        "model": message.model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": text},
            "finish_reason": finish,
        }],
        "usage": openai_usage(message),
    });
    if let Some(calls) = tool_calls {
        body["choices"][0]["message"]["tool_calls"] = calls.clone();
    }
    body.to_string()
}

fn openai_sse_scenario(
    message: &AnthropicMessage,
    text: &str,
    tool_calls: &Option<Value>,
    finish: &str,
    _proto: WireProtocol,
) -> String {
    let mut out = String::new();
    let mut frame = |delta: Value, finish_reason: &str| {
        out.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({
                "id": "chatcmpl-test-debug",
                "object": "chat.completion.chunk",
                "created": 0,
                "model": message.model,
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
            })
        ));
    };
    frame(serde_json::json!({"role": "assistant", "content": ""}), "");
    if !text.is_empty() {
        frame(serde_json::json!({"content": text}), "");
    }
    if let Some(calls) = tool_calls {
        frame(serde_json::json!({"tool_calls": calls}), "");
    }
    frame(serde_json::json!({}), finish);
    out.push_str("data: [DONE]\n\n");
    out
}

/// OpenAI usage 形状（prompt/completion/total）由 Anthropic 四元组映射。
fn openai_usage(message: &AnthropicMessage) -> Value {
    let u = message.usage;
    serde_json::json!({
        "prompt_tokens": u.input_tokens,
        "completion_tokens": u.output_tokens,
        "total_tokens": u.input_tokens + u.output_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;

    fn body(s: &str) -> Bytes {
        Bytes::from(s.to_string())
    }

    fn tools_body(tools: &str, tool_result: bool) -> Bytes {
        let messages = if tool_result {
            r#"[{"role":"user","content":"hi"},{"role":"assistant","content":[{"type":"tool_use","id":"toolu_x","name":"Bash","input":{}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_x","content":"ok"}]}]"#
        } else {
            r#"[{"role":"user","content":"hi"}]"#
        };
        body(&format!(
            "{{\"model\":\"test/tool-use\",\"tools\":{tools},\"messages\":{messages}}}"
        ))
    }

    const BASH_TOOL: &str = r#"[{"name":"Bash","input_schema":{"type":"object","required":["command"],"properties":{"command":{"type":"string"}}}}]"#;
    const TWO_TOOLS: &str = r#"[{"name":"Bash","input_schema":{"type":"object","required":["command"],"properties":{"command":{"type":"string"}}}},{"name":"Read","input_schema":{"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}}]"#;

    async fn json_of(resp: Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    async fn text_of(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read body");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    // ── 既有单元测试（bare `test` 契约；不改而通过）────────────────────────

    #[test]
    fn echo_text_extracts_last_user_message_string_content() {
        let b = body(
            r#"{"model":"test","messages":[{"role":"user","content":"first"},{"role":"assistant","content":"hi"},{"role":"user","content":"hello"}]}"#,
        );
        assert_eq!(echo_text(Some(&b)), "hello");
    }

    #[test]
    fn echo_text_handles_content_blocks() {
        let b = body(
            r#"{"model":"test","messages":[{"role":"user","content":[{"type":"text","text":"hello"},{"type":"text","text":" world"}]}]}"#,
        );
        assert_eq!(echo_text(Some(&b)), "hello world");
    }

    #[test]
    fn echo_text_falls_back_to_empty() {
        assert_eq!(echo_text(None), "");
        assert_eq!(echo_text(Some(&body("not json"))), "");
        assert_eq!(echo_text(Some(&body(r#"{"model":"test"}"#))), "");
        assert_eq!(
            echo_text(Some(&body(
                r#"{"model":"test","messages":[{"role":"assistant","content":"x"}]}"#
            ))),
            ""
        );
    }

    #[test]
    fn wants_stream_detects_flag() {
        assert!(wants_stream(Some(&body(r#"{"stream":true}"#))));
        assert!(!wants_stream(Some(&body(r#"{"stream":false}"#))));
        assert!(!wants_stream(Some(&body(r#"{}"#))));
        assert!(!wants_stream(None));
    }

    #[test]
    fn test_message_matches_requested_wording() {
        assert_eq!(
            test_message("hello"),
            "I'm test provider. I received your message \"hello\"."
        );
    }

    #[tokio::test]
    async fn anthropic_json_response_contains_text_and_model() {
        let resp = test_response(WireProtocol::Anthropic, "hello", false);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let v = json_of(resp).await;
        assert_eq!(v["model"], "test");
        assert_eq!(
            v["content"][0]["text"],
            "I'm test provider. I received your message \"hello\"."
        );
        assert_eq!(v["id"], "msg_test_debug");
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["usage"]["input_tokens"], 0);
    }

    #[tokio::test]
    async fn anthropic_sse_response_contains_text() {
        let resp = test_response(WireProtocol::Anthropic, "hi", true);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        let s = text_of(resp).await;
        assert!(s.contains("event: message_start"));
        assert!(s.contains("I'm test provider. I received your message \\\"hi\\\"."));
    }

    #[tokio::test]
    async fn openai_json_response_contains_text() {
        let resp = test_response(WireProtocol::OpenAiChat, "hello", false);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let v = json_of(resp).await;
        assert_eq!(
            v["choices"][0]["message"]["content"],
            "I'm test provider. I received your message \"hello\"."
        );
        assert_eq!(v["usage"]["prompt_tokens"], 0);
    }

    #[tokio::test]
    async fn openai_sse_response_contains_text_and_done() {
        let resp = test_response(WireProtocol::OpenAiChat, "hello", true);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        let s = text_of(resp).await;
        assert!(s.contains("data: [DONE]"));
        assert!(s.contains("I'm test provider. I received your message \\\"hello\\\"."));
    }

    // ── 2.1 场景解析 ──────────────────────────────────────────────────────

    #[test]
    fn scenario_parses_every_namespaced_name() {
        for (model, want) in [
            ("test", Scenario::Echo),
            ("test/text", Scenario::Text),
            ("test/long", Scenario::Long),
            ("test/thinking", Scenario::Thinking),
            ("test/tool-use", Scenario::ToolUse),
            ("test/tools-parallel", Scenario::ToolsParallel),
            ("test/full", Scenario::Full),
            ("test/empty", Scenario::Empty),
            ("test/error", Scenario::Error),
        ] {
            assert_eq!(scenario_from_model(model), want, "model {model}");
        }
    }

    #[test]
    fn scenario_names_round_trip() {
        for s in [
            Scenario::Echo,
            Scenario::Text,
            Scenario::Long,
            Scenario::Thinking,
            Scenario::ToolUse,
            Scenario::ToolsParallel,
            Scenario::Full,
            Scenario::Empty,
            Scenario::Error,
        ] {
            assert_eq!(scenario_from_model(s.name()), s, "{s:?}");
        }
    }

    #[test]
    fn unknown_namespaced_model_falls_back_to_echo() {
        for model in [
            "test/",
            "test/unknown",
            "test/TEXT",
            "test/text/extra",
            "not-a-test-model",
            "",
        ] {
            assert_eq!(scenario_from_model(model), Scenario::Echo, "model {model}");
        }
    }

    #[test]
    fn scenario_from_body_reads_model_field() {
        assert_eq!(scenario_from_body(None), Scenario::Echo);
        assert_eq!(scenario_from_body(Some(&body("not json"))), Scenario::Echo);
        assert_eq!(scenario_from_body(Some(&body(r#"{}"#))), Scenario::Echo);
        assert_eq!(
            scenario_from_body(Some(&body(r#"{"model":"test/thinking"}"#))),
            Scenario::Thinking
        );
    }

    #[tokio::test]
    async fn bare_test_stays_byte_identical_across_scenario_dispatch() {
        // scenario_response(Echo) 与既有 test_response 逐字一致（两种协议 × 两种流式）。
        for proto in [
            WireProtocol::Anthropic,
            WireProtocol::OpenAiChat,
            WireProtocol::OpenAiResponses,
        ] {
            for stream in [false, true] {
                let old = text_of(test_response(proto, "hi", stream)).await;
                let new = text_of(scenario_response(proto, Scenario::Echo, None, "hi", stream)).await;
                assert_eq!(old, new, "{proto:?} stream={stream}");
            }
        }
    }

    #[tokio::test]
    async fn every_scenario_produces_its_fixed_message_id_and_model() {
        for (scenario, id) in [
            (Scenario::Text, "msg_test_text"),
            (Scenario::Long, "msg_test_long"),
            (Scenario::Thinking, "msg_test_thinking"),
            (Scenario::ToolUse, "msg_test_tool_use"),
            (Scenario::ToolsParallel, "msg_test_tools_parallel"),
            (Scenario::Full, "msg_test_full"),
            (Scenario::Empty, "msg_test_empty"),
        ] {
            let b = body(r#"{"model":"test/x"}"#);
            let resp = scenario_response(WireProtocol::Anthropic, scenario, Some(&b), "hi", false);
            let v = json_of(resp).await;
            assert_eq!(v["id"], id, "{scenario:?}");
            // 应答 model 回带请求的模型名（用量/呈现面可区分场景）。
            assert_eq!(v["model"], "test/x", "{scenario:?}");
        }
    }

    // ── 2.2 agent-loop 规则（tool-use / full）─────────────────────────────

    #[tokio::test]
    async fn tool_use_scenario_drives_the_loop() {
        // tools + 无 tool_result → 首个 tool 的 tool_use。
        let resp = scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolUse,
            Some(&tools_body(TWO_TOOLS, false)),
            "hi",
            false,
        );
        let v = json_of(resp).await;
        assert_eq!(v["stop_reason"], "tool_use");
        assert_eq!(v["content"][0]["type"], "tool_use");
        assert_eq!(v["content"][0]["name"], "Bash", "首个声明工具");
        assert_eq!(v["content"][0]["input"]["command"], agent_loop::GATED_COMMAND_PREFIX);
        assert_eq!(v["content"][0]["id"], "toolu_test_1");

        // 有 tool_result → 终文本。
        let resp = scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolUse,
            Some(&tools_body(TWO_TOOLS, true)),
            "hi",
            false,
        );
        let v = json_of(resp).await;
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][0]["text"], TOOL_FINAL_TEXT);

        // 无 tools → 纯文本降级（不报错）。
        let b = body(r#"{"model":"test/tool-use","messages":[{"role":"user","content":"hi"}]}"#);
        let resp = scenario_response(WireProtocol::Anthropic, Scenario::ToolUse, Some(&b), "hi", false);
        let v = json_of(resp).await;
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["content"][0]["text"], PLAIN_TEXT);
    }

    #[tokio::test]
    async fn tool_use_scenario_with_nameless_tools_degrades_to_text() {
        let tools = r#"[{"input_schema":{"type":"object"}}]"#;
        let resp = scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolUse,
            Some(&tools_body(tools, false)),
            "hi",
            false,
        );
        let v = json_of(resp).await;
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["content"][0]["text"], PLAIN_TEXT);
    }

    #[tokio::test]
    async fn text_family_scenarios_never_emit_tool_use() {
        for scenario in [
            Scenario::Text,
            Scenario::Long,
            Scenario::Thinking,
            Scenario::Empty,
        ] {
            let resp = scenario_response(
                WireProtocol::Anthropic,
                scenario,
                Some(&tools_body(TWO_TOOLS, false)),
                "hi",
                false,
            );
            let v = json_of(resp).await;
            assert!(
                v["content"]
                    .as_array()
                    .expect("content array")
                    .iter()
                    .all(|b| b["type"] != "tool_use"),
                "{scenario:?} must not emit tool_use: {v}"
            );
            assert_ne!(v["stop_reason"], "tool_use", "{scenario:?}");
        }
    }

    #[tokio::test]
    async fn full_scenario_orders_thinking_text_then_tool_use() {
        let resp = scenario_response(
            WireProtocol::Anthropic,
            Scenario::Full,
            Some(&tools_body(TWO_TOOLS, false)),
            "hi",
            false,
        );
        let v = json_of(resp).await;
        let kinds: Vec<&str> = v["content"]
            .as_array()
            .expect("content")
            .iter()
            .map(|b| b["type"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(kinds, vec!["thinking", "text", "tool_use"]);
        assert_eq!(v["stop_reason"], "tool_use");

        // 终文本轮不带 thinking（design 开放问题 2）。
        let resp = scenario_response(
            WireProtocol::Anthropic,
            Scenario::Full,
            Some(&tools_body(TWO_TOOLS, true)),
            "hi",
            false,
        );
        let v = json_of(resp).await;
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["stop_reason"], "end_turn");
    }

    #[tokio::test]
    async fn full_scenario_without_tools_degrades_to_plain_text() {
        // design D2：无 tools → 纯文本降级（不报错）——连 thinking 都不发，
        // 场景职责单一（混排只在工具环成立时出现）。
        let b = body(r#"{"model":"test/full","messages":[{"role":"user","content":"hi"}]}"#);
        let resp = scenario_response(WireProtocol::Anthropic, Scenario::Full, Some(&b), "hi", false);
        let v = json_of(resp).await;
        let kinds: Vec<&str> = v["content"]
            .as_array()
            .expect("content")
            .iter()
            .map(|b| b["type"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(kinds, vec!["text"]);
        assert_eq!(v["content"][0]["text"], PLAIN_TEXT);
        assert_eq!(v["stop_reason"], "end_turn");
    }

    // ── 2.3 tools-parallel ────────────────────────────────────────────────

    #[tokio::test]
    async fn tools_parallel_emits_one_tool_use_per_declared_tool() {
        let resp = scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolsParallel,
            Some(&tools_body(TWO_TOOLS, false)),
            "hi",
            false,
        );
        let v = json_of(resp).await;
        let blocks = v["content"].as_array().expect("content");
        assert_eq!(blocks.len(), 2, "{v}");
        assert_eq!(v["stop_reason"], "tool_use");
        assert_eq!(blocks[0]["name"], "Bash");
        assert_eq!(blocks[1]["name"], "Read");
        assert_ne!(blocks[0]["input"], blocks[1]["input"], "inputs must differ");
        assert_eq!(blocks[0]["id"], "toolu_test_p0");
        assert_eq!(blocks[1]["id"], "toolu_test_p1");
    }

    #[tokio::test]
    async fn tools_parallel_single_tool_and_followup_turn() {
        let resp = scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolsParallel,
            Some(&tools_body(BASH_TOOL, false)),
            "hi",
            false,
        );
        let v = json_of(resp).await;
        assert_eq!(v["content"].as_array().expect("content").len(), 1);
        assert_eq!(v["stop_reason"], "tool_use");

        // 有 tool_result 的次回合 → 终文本（与 2.2 的收敛规则一致）。
        let resp = scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolsParallel,
            Some(&tools_body(TWO_TOOLS, true)),
            "hi",
            false,
        );
        let v = json_of(resp).await;
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["content"][0]["text"], TOOL_FINAL_TEXT);

        // 无 tools → 纯文本降级。
        let b = body(r#"{"model":"test/tools-parallel","messages":[]}"#);
        let resp = scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolsParallel,
            Some(&b),
            "hi",
            false,
        );
        let v = json_of(resp).await;
        assert_eq!(v["content"][0]["text"], PLAIN_TEXT);
    }

    // ── 2.4 long / empty / error ──────────────────────────────────────────

    #[tokio::test]
    async fn long_scenario_is_deterministic_and_assembles_from_stream() {
        let b = body(r#"{"model":"test/long"}"#);
        let j1 = json_of(scenario_response(WireProtocol::Anthropic, Scenario::Long, Some(&b), "hi", false)).await;
        let j2 = json_of(scenario_response(WireProtocol::Anthropic, Scenario::Long, Some(&b), "hi", false)).await;
        assert_eq!(j1, j2, "same request → same response");
        let text = j1["content"][0]["text"].as_str().expect("text");
        assert!(text.starts_with(LONG_TEXT_HEADER));
        assert!(text.len() > 1000, "long body: {} bytes", text.len());

        // 流式拼接 == 非流式正文（逐字）。
        let sse = text_of(scenario_response(WireProtocol::Anthropic, Scenario::Long, Some(&b), "hi", true)).await;
        let mut assembled = String::new();
        for line in sse.lines() {
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            let v: Value = serde_json::from_str(data).expect("frame json");
            if v["delta"]["type"] == "text_delta" {
                assembled.push_str(v["delta"]["text"].as_str().unwrap_or(""));
            }
        }
        assert_eq!(assembled, text);
        // 固定分块数：ceil(len / LONG_CHUNK)。
        let expected = text.chars().count().div_ceil(LONG_CHUNK);
        assert_eq!(sse.matches("\"type\":\"text_delta\"").count(), expected);
    }

    #[tokio::test]
    async fn empty_scenario_completes_without_content() {
        let b = body(r#"{"model":"test/empty"}"#);
        let v = json_of(scenario_response(WireProtocol::Anthropic, Scenario::Empty, Some(&b), "hi", false)).await;
        assert_eq!(v["content"], serde_json::json!([]));
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["usage"]["input_tokens"], 0);
        let sse = text_of(scenario_response(WireProtocol::Anthropic, Scenario::Empty, Some(&b), "hi", true)).await;
        assert!(!sse.contains("content_block_start"));
        assert!(sse.contains("message_stop"));
    }

    #[tokio::test]
    async fn error_scenario_answers_anthropic_error_body() {
        let b = body(r#"{"model":"test/error"}"#);
        let resp = scenario_response(WireProtocol::Anthropic, Scenario::Error, Some(&b), "hi", false);
        assert!(resp.status().is_server_error());
        let v = json_of(resp).await;
        assert_eq!(v["type"], "error");
        assert_eq!(v["error"]["type"], "api_error");
        assert_eq!(v["error"]["message"], ERROR_MESSAGE);
        assert!(v.get("content").is_none(), "no message is produced");
    }

    #[tokio::test]
    async fn error_scenario_uses_openai_error_shape_on_openai_family() {
        let b = body(r#"{"model":"test/error"}"#);
        for proto in [WireProtocol::OpenAiChat, WireProtocol::OpenAiResponses] {
            let resp = scenario_response(proto, Scenario::Error, Some(&b), "hi", false);
            assert!(resp.status().is_server_error());
            let v = json_of(resp).await;
            assert_eq!(v["error"]["type"], "api_error");
            assert!(v.get("type").is_none(), "no Anthropic wrapper: {v}");
        }
    }

    // ── 2.5 确定性 usage ──────────────────────────────────────────────────

    #[test]
    fn scenario_usage_is_non_zero_distinct_and_zero_for_echo_and_empty() {
        let message_scenarios = [
            Scenario::Text,
            Scenario::Long,
            Scenario::Thinking,
            Scenario::ToolUse,
            Scenario::ToolsParallel,
            Scenario::Full,
        ];
        let mut seen = std::collections::HashSet::new();
        for s in message_scenarios {
            let u = s.usage();
            assert!(!u.is_zero(), "{s:?} must report non-zero usage");
            assert!(
                seen.insert((u.input_tokens, u.output_tokens, u.cache_read_input_tokens, u.cache_creation_input_tokens)),
                "{s:?} usage collides with another scenario"
            );
            // 字段互不相同：断言字段错位立即可见（design D6）。
            let quad = [
                u.input_tokens,
                u.output_tokens,
                u.cache_read_input_tokens,
                u.cache_creation_input_tokens,
            ];
            let unique: std::collections::HashSet<u64> = quad.iter().copied().collect();
            assert_eq!(unique.len(), 4, "{s:?} quad not field-distinct: {quad:?}");
            assert_eq!(s.usage_info().output_tokens, Some(u.output_tokens));
        }
        for s in [Scenario::Echo, Scenario::Empty, Scenario::Error] {
            assert!(s.usage().is_zero(), "{s:?} keeps all-zero usage");
            assert_eq!(s.usage_info(), UsageInfo::default(), "{s:?}");
        }
    }

    #[tokio::test]
    async fn scenario_json_and_sse_report_the_same_usage() {
        let b = body(r#"{"model":"test/thinking"}"#);
        let v = json_of(scenario_response(WireProtocol::Anthropic, Scenario::Thinking, Some(&b), "hi", false)).await;
        assert_eq!(v["usage"]["input_tokens"], Scenario::Thinking.usage().input_tokens);
        assert_eq!(v["usage"]["output_tokens"], Scenario::Thinking.usage().output_tokens);
        assert_eq!(
            v["usage"]["cache_read_input_tokens"],
            Scenario::Thinking.usage().cache_read_input_tokens
        );
        assert_eq!(
            v["usage"]["cache_creation_input_tokens"],
            Scenario::Thinking.usage().cache_creation_input_tokens
        );
        let sse = text_of(scenario_response(WireProtocol::Anthropic, Scenario::Thinking, Some(&b), "hi", true)).await;
        assert!(sse.contains(&format!(
            "\"output_tokens\":{}",
            Scenario::Thinking.usage().output_tokens
        )));
        assert!(sse.contains(&format!(
            "\"input_tokens\":{}",
            Scenario::Thinking.usage().input_tokens
        )));
    }

    // ── 2.6 与 fake-provider 规则一致性 ───────────────────────────────────

    /// 同一请求形状分别过 test 模型规则与 fake 的内置规则：**块类型序列与
    /// stop_reason** 必须一致（D2：两处规则共享 `agent_loop` 事实源；差异只
    /// 允许在「选哪个 tool」——fake 按只读偏好、test 取首个，二者都是确定性
    /// 单值）。规则已提为共用函数，故本测试同时是共用性的存在性检查。
    #[tokio::test]
    async fn loop_shape_matches_fake_provider_rules() {
        // 偏好命中与首个一致的工具表 → 连 tool 名与 input 都相同。
        let same_shape =
            r#"{"model":"m","tools":[{"name":"Read","input_schema":{"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}}],"messages":[{"role":"user","content":"hi"}]}"#;
        let body_bytes = body(&same_shape.clone().replace("\"model\":\"m\"", "\"model\":\"test/tool-use\""));
        let parsed: Value = serde_json::from_str(&same_shape).expect("json");
        let (fake_content, fake_stop) = crate::fake_provider::builtin_response(Some(&parsed));
        let v = json_of(scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolUse,
            Some(&body_bytes),
            "hi",
            false,
        ))
        .await;
        let test_types: Vec<&str> = v["content"]
            .as_array()
            .expect("content")
            .iter()
            .map(|b| b["type"].as_str().unwrap_or(""))
            .collect();
        let fake_types: Vec<&str> = fake_content
            .iter()
            .map(|b| b["type"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(test_types, fake_types);
        assert_eq!(v["stop_reason"], fake_stop);
        // 仅只读工具 + fake 的只读命令：两侧 input 相同（test 侧命令字段不在
        // 该 schema 里，故与 fake 的确定性 input 一致）。
        assert_eq!(v["content"][0]["name"], fake_content[0]["name"]);
        assert_eq!(v["content"][0]["input"], fake_content[0]["input"]);

        // 有 tool_result → 两侧都是终文本（end_turn）。
        let with_result = r#"{"model":"test/tool-use","tools":[{"name":"Read","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":[{"type":"tool_result","tool_content":"x","content":"x"}]}]}"#;
        let (fake_content, fake_stop) = crate::fake_provider::builtin_response(Some(
            &serde_json::from_str(with_result).expect("json"),
        ));
        let v = json_of(scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolUse,
            Some(&body(with_result)),
            "hi",
            false,
        ))
        .await;
        assert_eq!(v["content"][0]["type"], fake_content[0]["type"]);
        assert_eq!(v["stop_reason"], fake_stop);

        // 无 tools → 两侧都是纯文本（end_turn）。
        let plain = r#"{"model":"test/tool-use","messages":[{"role":"user","content":"hi"}]}"#;
        let (fake_content, fake_stop) =
            crate::fake_provider::builtin_response(Some(&serde_json::from_str(plain).expect("json")));
        let v = json_of(scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolUse,
            Some(&body(plain)),
            "hi",
            false,
        ))
        .await;
        assert_eq!(v["content"][0]["type"], fake_content[0]["type"]);
        assert_eq!(v["stop_reason"], fake_stop);
    }

    /// 有意分岔的**显式钉住**（design D2）：工具表里首个工具不是偏好命中项
    /// 时，fake 仍挑偏好工具、test 模型取首个——分岔只影响选哪个 tool，
    /// 块类型序列与 stop_reason 仍一致。
    #[tokio::test]
    async fn tool_selection_diverges_by_design_but_shape_matches() {
        let req = r#"{"model":"test/tool-use","tools":[{"name":"Bash","input_schema":{"type":"object","required":["command"],"properties":{"command":{"type":"string"}}}},{"name":"Read","input_schema":{"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}}],"messages":[{"role":"user","content":"hi"}]}"#;
        let parsed: Value = serde_json::from_str(req).expect("json");
        let (fake_content, fake_stop) = crate::fake_provider::builtin_response(Some(&parsed));
        let v = json_of(scenario_response(
            WireProtocol::Anthropic,
            Scenario::ToolUse,
            Some(&body(req)),
            "hi",
            false,
        ))
        .await;
        assert_eq!(fake_content[0]["name"], "Read", "fake 按只读偏好");
        assert_eq!(v["content"][0]["name"], "Bash", "test 模型取首个声明工具");
        assert_eq!(v["content"][0]["type"], fake_content[0]["type"]);
        assert_eq!(v["stop_reason"], fake_stop);
        // 两侧 input 都是确定性对象，且 test 侧的命令可触发审批。
        assert_eq!(v["content"][0]["input"]["command"], agent_loop::GATED_COMMAND_PREFIX);
        assert_eq!(fake_content[0]["input"]["path"], "ok");
    }

    // ── OpenAI 家族降级（design D5）───────────────────────────────────────

    #[tokio::test]
    async fn thinking_degrades_to_plain_text_on_openai_family() {
        let b = body(r#"{"model":"test/thinking"}"#);
        for proto in [WireProtocol::OpenAiChat, WireProtocol::OpenAiResponses] {
            let resp = scenario_response(proto, Scenario::Thinking, Some(&b), "hi", false);
            assert_eq!(resp.status(), StatusCode::OK);
            let v = json_of(resp).await;
            let content = v["choices"][0]["message"]["content"]
                .as_str()
                .expect("openai content is text");
            assert!(content.contains("test provider thinking"), "{v}");
            assert!(content.contains("I'm test provider."), "{v}");
            assert!(
                v["choices"][0]["message"].get("tool_calls").is_none(),
                "{v}"
            );
        }
    }

    #[tokio::test]
    async fn openai_family_maps_tool_use_to_tool_calls() {
        let b = tools_body(TWO_TOOLS, false);
        let resp = scenario_response(WireProtocol::OpenAiChat, Scenario::ToolUse, Some(&b), "hi", false);
        let v = json_of(resp).await;
        assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
        let calls = v["choices"][0]["message"]["tool_calls"]
            .as_array()
            .expect("tool_calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "Bash");
        assert!(
            calls[0]["function"]["arguments"]
                .as_str()
                .expect("arguments string")
                .contains("command")
        );
        assert_eq!(v["usage"]["prompt_tokens"], Scenario::ToolUse.usage().input_tokens);
    }

    #[tokio::test]
    async fn openai_sse_carries_scenario_text_and_done() {
        let b = body(r#"{"model":"test/text","stream":true}"#);
        let resp = scenario_response(WireProtocol::OpenAiChat, Scenario::Text, Some(&b), "hello", true);
        let s = text_of(resp).await;
        assert!(s.contains("data: [DONE]"));
        assert!(s.contains("I'm test provider. I received your message \\\"hello\\\"."));
    }

    // ── long 场景滴流形状（流中取消/增量呈现的确定性前提）─────────────────

    #[tokio::test]
    async fn long_stream_response_drips_frames_over_time() {
        use futures_util::StreamExt;
        let b = body(r#"{"model":"test/long"}"#);
        let resp = scenario_response(WireProtocol::Anthropic, Scenario::Long, Some(&b), "hi", true);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        let mut body = resp.into_body().into_data_stream();
        let first = body.next().await.expect("first frame").expect("ok");
        assert!(
            String::from_utf8_lossy(&first).contains("message_start"),
            "first frame is message_start"
        );
        // 首帧之后必须还能等到后续帧（滴流：帧按 LONG_FRAME_INTERVAL 节流）。
        let second = body.next().await.expect("second frame").expect("ok");
        assert!(String::from_utf8_lossy(&second).starts_with("event: content_block_start"));
        // 非 long 场景仍是一次性 body（不滴流）。
        let resp = scenario_response(WireProtocol::Anthropic, Scenario::Text, Some(&b), "hi", true);
        let all = text_of(resp).await;
        assert!(all.contains("message_stop"));
    }

    #[tokio::test]
    async fn long_non_stream_response_is_single_json_body() {
        let b = body(r#"{"model":"test/long"}"#);
        let resp = scenario_response(WireProtocol::Anthropic, Scenario::Long, Some(&b), "hi", false);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn long_text_is_stable_across_calls() {
        assert_eq!(long_text(), long_text());
        assert_eq!(long_text().lines().count(), 40, "header + 39 lines");
    }
}
