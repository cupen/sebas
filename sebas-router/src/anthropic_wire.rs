//! Anthropic `/v1/messages` 线协议**响应形状**构造（fake-provider-upstream 1.1；
//! 块类型扩展见 extend-test-model-scenarios 1.1–1.3）。
//!
//! 从 `test_provider` 下沉出来的共用事实源：内置 debug `test` provider
//! （`test_provider.rs`）与 `fake_provider.rs`（本地假上游）都用这里的
//! `AnthropicMessage` 生成非流式 JSON 与流式 SSE，避免两处各写一份 Anthropic
//! 线协议知识。
//!
//! 块类型：`text` / `thinking` / `tool_use`（未知类型保守透传 start/stop）。
//! 非流式 JSON 与流式 SSE 出自同一 `content` 块数组——**同构**由
//! [`AnthropicMessage::sse_frames`] 保证（每块一类事件，delta 类型随块）。
//!
//! 只构造**响应**方向（router 从不构造 Anthropic 请求）；请求方向的解析
//! （echo / wants_stream / tool 扫描）仍留在各自模块。

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Value, json};

/// Anthropic usage 四元组（线协议字段名逐一对应）。`Default` = 全零
/// （debug `test` provider 的既有形状）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

impl AnthropicUsage {
    /// 显式四元组（场景确定性 usage 表；`const` 便于写成常量表）。
    pub const fn new(
        input_tokens: u64,
        output_tokens: u64,
        cache_read_input_tokens: u64,
        cache_creation_input_tokens: u64,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_input_tokens,
            cache_creation_input_tokens,
        }
    }

    /// 全零（bare `test` 与 `test/empty` 的既有形状）。
    pub fn is_zero(self) -> bool {
        self == Self::default()
    }

    /// usage 对象（json! 保序：input → output → cache_read → cache_creation）。
    pub fn to_value(self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_read_input_tokens": self.cache_read_input_tokens,
            "cache_creation_input_tokens": self.cache_creation_input_tokens,
        })
    }
}

/// 一条完整的 assistant message（非流式 JSON 与流式 SSE 同一事实源）。
#[derive(Debug, Clone)]
pub struct AnthropicMessage {
    pub id: String,
    pub model: String,
    /// content 块数组（`text` / `thinking` / `tool_use`）。空数组合法
    /// （`test/empty` 与 SSE 起始帧）。
    pub content: Vec<Value>,
    /// `end_turn` / `tool_use` / …（SSE 的 message_delta.delta.stop_reason）。
    pub stop_reason: String,
    pub usage: AnthropicUsage,
    /// 流式**分块粒度**（字符数）：`Some(n)` 时 text / thinking / tool input
    /// 按 n 个字符切成多个 delta；`None` = 整块一个 delta。
    ///
    /// `None` 是 bare `test` 与 fake 上游的历史形状（逐字节不变）；
    /// 场景模型用 `Some(·)` 换取确定性可断言的增量序列（design D4）。
    pub chunk_size: Option<usize>,
}

impl AnthropicMessage {
    /// 单 text 块的便捷构造（debug test provider 与 fake 的文本应答共用）。
    /// 签名与行为保持 fake-provider-upstream 1.1 的既有契约（chunk_size=None）。
    pub fn text(
        id: impl Into<String>,
        model: impl Into<String>,
        text: &str,
        stop_reason: impl Into<String>,
        usage: AnthropicUsage,
    ) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            content: vec![text_block(text)],
            stop_reason: stop_reason.into(),
            usage,
            chunk_size: None,
        }
    }

    /// 多块构造（thinking / text / tool_use 混排的场景模型）。
    pub fn blocks(
        id: impl Into<String>,
        model: impl Into<String>,
        content: Vec<Value>,
        stop_reason: impl Into<String>,
        usage: AnthropicUsage,
    ) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            content,
            stop_reason: stop_reason.into(),
            usage,
            chunk_size: None,
        }
    }

    /// builder：设定流式分块粒度（见 [`Self::chunk_size`]）。
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = Some(chunk_size);
        self
    }

    /// 非流式 200 JSON message 体（`application/json`）。
    pub fn json(&self) -> String {
        self.to_message_value().to_string()
    }

    /// 逐个 SSE 帧（每帧以空行收尾）；[`Self::sse`] 即其顺序拼接。
    /// 拆成帧是为了让调用方可以按帧节流下发（`test/long` 的确定性滴流）。
    ///
    /// 事件序列：
    /// `message_start → (content_block_start → content_block_delta(s) →
    /// content_block_stop)* → message_delta → message_stop`。
    ///
    /// delta 类型随块：text → `text_delta`；thinking → `thinking_delta`
    /// （块带 `signature` 时先补一帧 `signature_delta`）；tool_use →
    /// `input_json_delta`（`partial_json` 为 input 序列化的分片，Anthropic 官方
    /// SDK 按增量拼接）。
    pub fn sse_frames(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut push = |event: &str, data: Value| {
            let mut frame = String::new();
            frame.push_str("event: ");
            frame.push_str(event);
            frame.push_str("\ndata: ");
            frame.push_str(&data.to_string());
            frame.push_str("\n\n");
            out.push(frame);
        };

        // message_start：content 为空数组，usage 只带 input/cache（output 由
        // message_delta 的累计值承载——与 router 的 SseUsageParser 契约一致）。
        push(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": self.id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {
                        "input_tokens": self.usage.input_tokens,
                        "output_tokens": 0,
                        "cache_read_input_tokens": self.usage.cache_read_input_tokens,
                        "cache_creation_input_tokens": self.usage.cache_creation_input_tokens,
                    },
                },
            }),
        );

        for (index, block) in self.content.iter().enumerate() {
            let index = index as u64;
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                    push(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {"type": "text", "text": ""},
                        }),
                    );
                    for piece in chunk_text(text, self.chunk_size) {
                        push(
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": {"type": "text_delta", "text": piece},
                            }),
                        );
                    }
                }
                Some("thinking") => {
                    let thinking = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                    push(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {"type": "thinking", "thinking": ""},
                        }),
                    );
                    for piece in chunk_text(thinking, self.chunk_size) {
                        push(
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": {"type": "thinking_delta", "thinking": piece},
                            }),
                        );
                    }
                    // signature 只在块带该字段（且非空）时补帧——真实客户端的
                    // 实测结论见 extend-test-model-scenarios design D3/4.3。
                    if let Some(signature) = block
                        .get("signature")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        push(
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": {"type": "signature_delta", "signature": signature},
                            }),
                        );
                    }
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(Value::as_str).unwrap_or("");
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                    let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                    push(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}},
                        }),
                    );
                    for piece in chunk_text(&input.to_string(), self.chunk_size) {
                        push(
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": {
                                    "type": "input_json_delta",
                                    "partial_json": piece,
                                },
                            }),
                        );
                    }
                }
                // 未知块类型：只发 start/stop（保守透传形状，不发伪造 delta）。
                _ => {
                    push(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": block.clone(),
                        }),
                    );
                }
            }
            push(
                "content_block_stop",
                json!({"type": "content_block_stop", "index": index}),
            );
        }

        push(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": {"stop_reason": self.stop_reason, "stop_sequence": null},
                "usage": {"output_tokens": self.usage.output_tokens},
            }),
        );
        push("message_stop", json!({"type": "message_stop"}));
        out
    }

    /// 完整 SSE 体（帧的顺序拼接；`text/event-stream`）。
    pub fn sse(&self) -> String {
        self.sse_frames().concat()
    }

    fn to_message_value(&self) -> Value {
        json!({
            "id": self.id,
            "type": "message",
            "role": "assistant",
            "model": self.model,
            "content": self.content,
            "stop_reason": self.stop_reason,
            "stop_sequence": null,
            "usage": self.usage.to_value(),
        })
    }

    /// 200 非流式响应（`application/json`）。
    pub fn into_json_response(self) -> Response {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(self.json()))
            .expect("static response parts valid")
    }

    /// 200 流式响应（`text/event-stream`）。
    pub fn into_sse_response(self) -> Response {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(Body::from(self.sse()))
            .expect("static response parts valid")
    }
}

/// Anthropic text content 块。
pub fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

/// Anthropic thinking content 块。`signature` 为 `None` 时省略该字段
/// （无签名块仍是合法块；真实应答带签名，由调用方决定）。
pub fn thinking_block(thinking: &str, signature: Option<&str>) -> Value {
    match signature {
        Some(signature) => json!({"type": "thinking", "thinking": thinking, "signature": signature}),
        None => json!({"type": "thinking", "thinking": thinking}),
    }
}

/// Anthropic tool_use content 块。
pub fn tool_use_block(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

/// 把长文按 `size` 个字符切片；`None` → 整串一个分片（空串也算一帧——
/// fake / bare `test` 的历史形状）。
///
/// 按 **char** 切，不按字节：多字节 UTF-8 字符不会被劈开。
fn chunk_text(text: &str, size: Option<usize>) -> Vec<String> {
    match size {
        None => vec![text.to_string()],
        Some(size) if size == 0 => vec![text.to_string()],
        Some(size) => {
            let chars: Vec<char> = text.chars().collect();
            if chars.is_empty() {
                return Vec::new();
            }
            chars
                .chunks(size)
                .map(|c| c.iter().collect::<String>())
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage() -> AnthropicUsage {
        AnthropicUsage {
            input_tokens: 3,
            output_tokens: 5,
            cache_read_input_tokens: 1,
            cache_creation_input_tokens: 2,
        }
    }

    /// SSE 帧序列 → content 块数组（按 index 归位、按 delta 拼接）。
    /// 与 `AnthropicMessage::json()` 的 content 逐块比对 = 同构性断言的手段。
    fn assemble_blocks(sse: &str) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        let mut cur: Option<(usize, Value)> = None;
        let mut text = String::new();
        let mut thinking = String::new();
        let mut partial = String::new();
        let mut signature = String::new();
        for line in sse.lines() {
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            let v: Value = serde_json::from_str(data).expect("sse data line is JSON");
            match v.get("type").and_then(Value::as_str).unwrap_or("") {
                "content_block_start" => {
                    text.clear();
                    thinking.clear();
                    partial.clear();
                    signature.clear();
                    cur = Some((
                        v["index"].as_u64().expect("index") as usize,
                        v["content_block"].clone(),
                    ));
                }
                "content_block_delta" => {
                    let delta = &v["delta"];
                    match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                        "text_delta" => {
                            text.push_str(delta["text"].as_str().expect("text_delta.text"))
                        }
                        "thinking_delta" => thinking
                            .push_str(delta["thinking"].as_str().expect("thinking_delta.thinking")),
                        "input_json_delta" => partial.push_str(
                            delta["partial_json"]
                                .as_str()
                                .expect("input_json_delta.partial_json"),
                        ),
                        "signature_delta" => signature.push_str(
                            delta["signature"]
                                .as_str()
                                .expect("signature_delta.signature"),
                        ),
                        other => panic!("unexpected delta type {other}"),
                    }
                }
                "content_block_stop" => {
                    let (index, mut block) = cur.take().expect("stop after start");
                    match block.get("type").and_then(Value::as_str).unwrap_or("") {
                        "text" => block["text"] = Value::String(std::mem::take(&mut text)),
                        "thinking" => {
                            block["thinking"] = Value::String(std::mem::take(&mut thinking));
                            if !signature.is_empty() {
                                block["signature"] =
                                    Value::String(std::mem::take(&mut signature));
                            } else {
                                block.as_object_mut().expect("object").remove("signature");
                            }
                        }
                        "tool_use" => {
                            if !partial.is_empty() {
                                block["input"] =
                                    serde_json::from_str(&partial).expect("partial_json reassembles");
                            }
                        }
                        _ => {}
                    }
                    while out.len() <= index {
                        out.push(Value::Null);
                    }
                    out[index] = block;
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn text_json_carries_content_stop_reason_and_usage() {
        let msg = AnthropicMessage::text("msg_x", "m", "hello", "end_turn", usage());
        let v: Value = serde_json::from_str(&msg.json()).expect("valid JSON");
        assert_eq!(v["id"], "msg_x");
        assert_eq!(v["type"], "message");
        assert_eq!(v["role"], "assistant");
        assert_eq!(v["model"], "m");
        assert_eq!(v["content"][0]["type"], "text");
        assert_eq!(v["content"][0]["text"], "hello");
        assert_eq!(v["stop_reason"], "end_turn");
        assert_eq!(v["stop_sequence"], Value::Null);
        assert_eq!(v["usage"]["input_tokens"], 3);
        assert_eq!(v["usage"]["output_tokens"], 5);
        assert_eq!(v["usage"]["cache_read_input_tokens"], 1);
        assert_eq!(v["usage"]["cache_creation_input_tokens"], 2);
    }

    #[test]
    fn zero_usage_matches_debug_provider_shape() {
        // test_provider 历史形状：全零 usage + 单 text 块。
        let msg = AnthropicMessage::text(
            "msg_test_debug",
            "test",
            "hi",
            "end_turn",
            AnthropicUsage::default(),
        );
        let v: Value = serde_json::from_str(&msg.json()).expect("valid JSON");
        assert_eq!(v["usage"]["input_tokens"], 0);
        assert_eq!(v["usage"]["output_tokens"], 0);
        assert_eq!(v["model"], "test");
    }

    #[test]
    fn text_sse_event_sequence_is_complete_and_text_matches_json() {
        let msg = AnthropicMessage::text("msg_s", "m", "he\"llo", "end_turn", usage());
        let sse = msg.sse();
        let events: Vec<&str> = sse
            .lines()
            .filter_map(|l| l.strip_prefix("event: "))
            .collect();
        assert_eq!(
            events,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );

        // text_delta 的文本与 JSON 一致（转义形式）。
        let json_text = serde_json::to_string("he\"llo").unwrap();
        assert!(
            sse.contains(&json_text),
            "escaped text delta missing:\n{sse}"
        );
        assert!(sse.contains("\"type\":\"text_delta\""));
        // message_delta 的 stop_reason + usage 与 JSON 一致。
        assert!(sse.contains("\"stop_reason\":\"end_turn\""));
        assert!(sse.contains("\"output_tokens\":5"));
        // 每个事件都以空行收尾（SSE 帧边界；router 的 SseUsageParser 依赖）。
        assert_eq!(sse.matches("\n\n").count(), events.len());
    }

    #[test]
    fn tool_use_sse_emits_input_json_delta_and_tool_stop_reason() {
        let msg = AnthropicMessage::blocks(
            "msg_t",
            "m",
            vec![tool_use_block("toolu_1", "Read", json!({"file_path": "x"}))],
            "tool_use",
            usage(),
        );
        let sse = msg.sse();
        assert!(sse.contains("\"type\":\"tool_use\""));
        assert!(sse.contains("\"name\":\"Read\""));
        assert!(sse.contains("\"type\":\"input_json_delta\""));
        assert!(sse.contains("\"stop_reason\":\"tool_use\""));
        // partial_json 是完整 input 的序列化（SDK 单帧拼接即可）——作为 JSON
        // 字符串嵌在 data 行里，按 JSON 转义后的形态比对。
        let partial = serde_json::to_string(&json!({"file_path": "x"})).unwrap();
        let embedded = serde_json::to_string(&partial).unwrap();
        assert!(sse.contains(&embedded), "partial_json missing:\n{sse}");
    }

    #[test]
    fn multi_block_indices_increment() {
        let msg = AnthropicMessage::blocks(
            "id",
            "m",
            vec![text_block("a"), text_block("b")],
            "end_turn",
            usage(),
        );
        let sse = msg.sse();
        assert!(sse.contains("\"index\":0"));
        assert!(sse.contains("\"index\":1"));
    }

    // ── extend-test-model-scenarios 1.1：块构造入口 ──────────────────────────

    #[test]
    fn thinking_block_carries_thinking_and_optional_signature() {
        let with_sig = thinking_block("reasoning", Some("sig-1"));
        assert_eq!(with_sig["type"], "thinking");
        assert_eq!(with_sig["thinking"], "reasoning");
        assert_eq!(with_sig["signature"], "sig-1");

        let bare = thinking_block("reasoning", None);
        assert_eq!(bare["type"], "thinking");
        assert!(
            bare.get("signature").is_none(),
            "unsigned thinking block omits the field: {bare}"
        );
    }

    #[test]
    fn text_constructor_keeps_single_chunk_shape() {
        // 既有 text() 构造（fake / bare test 共用）：不设分块 = 单 delta。
        let msg = AnthropicMessage::text("id", "m", "abcdefghij", "end_turn", usage());
        assert_eq!(msg.chunk_size, None);
        let deltas = msg
            .sse()
            .lines()
            .filter(|l| l.starts_with("data: ") && l.contains("text_delta"))
            .count();
        assert_eq!(deltas, 1, "one text_delta per text block without chunking");
    }

    // ── extend-test-model-scenarios 1.2：每块类型一类事件 ───────────────────

    #[test]
    fn thinking_sse_emits_thinking_delta_then_text_delta_in_order() {
        let msg = AnthropicMessage::blocks(
            "msg_th",
            "m",
            vec![
                thinking_block("why", Some("sig-1")),
                text_block("answer"),
            ],
            "end_turn",
            usage(),
        );
        let sse = msg.sse();
        let (think_at, sig_at, text_at) = (
            sse.find("\"type\":\"thinking_delta\"").expect("thinking_delta"),
            sse.find("\"type\":\"signature_delta\"").expect("signature_delta"),
            sse.find("\"type\":\"text_delta\"").expect("text_delta"),
        );
        assert!(think_at < sig_at, "signature_delta follows thinking_delta");
        assert!(sig_at < text_at, "thinking block precedes the text block");
        // thinking 块的 start 形状含 thinking 字段且首帧为空（与 text 不同）。
        //
        // 断言**不得**依赖 JSON 键序：`json!` 建的 map 在 serde_json 未开
        // `preserve_order` 时是 BTreeMap（键按字典序），开了才是插入序。而
        // `preserve_order` 由 **第三方 crate 的 feature 统一**决定
        // （`agent-client-protocol` 声明了它）——只要它在依赖图里就是插入序，
        // 一旦图变化（如 768e3d9 断开 `domain → acp` 把它移出 router 图）就
        // 翻成字典序。故此处按字段独立断言，与键序解耦。
        let start = sse
            .lines()
            .find(|l| l.contains("\"type\":\"content_block_start\"") && l.contains("thinking"))
            .expect("a thinking content_block_start frame");
        assert!(
            start.contains("\"type\":\"thinking\"") && start.contains("\"thinking\":\"\""),
            "thinking content_block_start carries an empty thinking field:\n{start}"
        );
        assert!(sse.contains("\"index\":0"));
        assert!(sse.contains("\"index\":1"));
        // thinking 块也有独立的 stop。
        assert_eq!(sse.matches("\"type\":\"content_block_stop\"").count(), 2);
    }

    #[test]
    fn unsigned_thinking_block_emits_no_signature_delta() {
        let msg = AnthropicMessage::blocks(
            "id",
            "m",
            vec![thinking_block("why", None)],
            "end_turn",
            usage(),
        );
        let sse = msg.sse();
        assert!(sse.contains("\"type\":\"thinking_delta\""));
        assert!(!sse.contains("signature_delta"));
    }

    #[test]
    fn chunk_size_splits_text_and_thinking_into_deterministic_deltas() {
        let msg = AnthropicMessage::blocks(
            "id",
            "m",
            vec![text_block("abcdefghij"), thinking_block("0123456789", None)],
            "end_turn",
            usage(),
        )
        .with_chunk_size(4);
        let sse = msg.sse();
        // 10 字符 / 4 = 3 帧（4+4+2），两个块共 6 帧 text/thinking delta。
        let text_deltas: Vec<&str> = sse
            .lines()
            .filter(|l| l.contains("\"type\":\"text_delta\""))
            .collect();
        assert_eq!(text_deltas.len(), 3, "{text_deltas:?}");
        assert!(text_deltas[0].contains("abcd"));
        assert!(text_deltas[2].contains("ij"));
        let think_deltas = sse.matches("\"type\":\"thinking_delta\"").count();
        assert_eq!(think_deltas, 3);
    }

    #[test]
    fn chunked_tool_input_is_split_and_reassembles() {
        let input = json!({"command": "echo ok", "n": 12});
        let msg = AnthropicMessage::blocks(
            "id",
            "m",
            vec![tool_use_block("toolu_1", "bash", input.clone())],
            "tool_use",
            usage(),
        )
        .with_chunk_size(5);
        let sse = msg.sse();
        let deltas = sse.matches("\"type\":\"input_json_delta\"").count();
        assert!(deltas > 1, "chunked tool input must shard: {sse}");
        assert_eq!(assemble_blocks(&sse), vec![tool_use_block("toolu_1", "bash", input)]);
    }

    #[test]
    fn sse_frames_concatenate_to_sse() {
        let msg = AnthropicMessage::blocks(
            "id",
            "m",
            vec![thinking_block("t", None), text_block("hello")],
            "end_turn",
            usage(),
        )
        .with_chunk_size(3);
        let frames = msg.sse_frames();
        assert!(frames.len() > 4, "one frame per event");
        assert!(
            frames.iter().all(|f| f.ends_with("\n\n")),
            "every frame closes with a blank line: {frames:?}"
        );
        assert_eq!(frames.concat(), msg.sse());
    }

    // ── extend-test-model-scenarios 1.3：非流式与流式同构 ───────────────────

    #[test]
    fn mixed_blocks_json_and_sse_are_isomorphic() {
        let content = vec![
            thinking_block("first reason", Some("sig-9")),
            text_block("then answer"),
            tool_use_block("toolu_9", "bash", json!({"command": "echo ok"})),
        ];
        let msg = AnthropicMessage::blocks(
            "msg_mix",
            "test/full",
            content.clone(),
            "tool_use",
            usage(),
        )
        .with_chunk_size(4);
        let v: Value = serde_json::from_str(&msg.json()).expect("valid JSON");
        assert_eq!(v["content"], Value::Array(content.clone()));
        assert_eq!(assemble_blocks(&msg.sse()), content);
    }

    #[test]
    fn long_text_json_and_sse_are_isomorphic() {
        let long: String = (0..500)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let msg = AnthropicMessage::blocks(
            "msg_long",
            "test/long",
            vec![text_block(&long)],
            "end_turn",
            usage(),
        )
        .with_chunk_size(8);
        let v: Value = serde_json::from_str(&msg.json()).expect("valid JSON");
        assert_eq!(v["content"][0]["text"], long);
        // 500 / 8 = 63 帧（62×8 + 4）。
        assert_eq!(msg.sse().matches("\"type\":\"text_delta\"").count(), 63);
        assert_eq!(assemble_blocks(&msg.sse()), vec![text_block(&long)]);
    }

    #[test]
    fn chunking_is_char_based_not_byte_based() {
        // 多字节字符不会被劈开（拼回原文）。
        let msg = AnthropicMessage::blocks(
            "id",
            "m",
            vec![text_block("中文测试字符串")],
            "end_turn",
            usage(),
        )
        .with_chunk_size(3);
        assert_eq!(assemble_blocks(&msg.sse()), vec![text_block("中文测试字符串")]);
    }

    #[test]
    fn empty_content_produces_no_block_events() {
        let msg = AnthropicMessage::blocks("id", "m", vec![], "end_turn", usage());
        let sse = msg.sse();
        assert!(!sse.contains("content_block_start"));
        assert!(sse.contains("message_start"));
        assert!(sse.contains("message_stop"));
        let v: Value = serde_json::from_str(&msg.json()).expect("valid JSON");
        assert_eq!(v["content"], json!([]));
    }
}
