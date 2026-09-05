//! SSE 增量 usage 解析器（Task 8，见 openspec/specs/router-metrics/spec.md）。
//!
//! 纯透传路由在 SSE 字节流上做"旁路 tee"：每 chunk 喂 parser 提取 usage 计数，
//! 字节本身原样透传。容错铁律：`[DONE]`、未知事件、截断帧、坏 JSON 一律跳过
//! ——解析失败只丢统计，绝不影响透传字节。
//!
//! 双协议 usage 来源：
//! - Anthropic：`message_start` 事件的 `message.usage`（input + cache_*）+
//!   `message_delta` 的 `usage.output_tokens`。
//! - OpenAI：chat completions 是 `usage.{prompt_tokens, completion_tokens}`，
//!   Responses API 是 `usage.{input_tokens, output_tokens}`（按 key 存在性探测）；
//!   流式 Responses 的 `response.completed` 把 usage 放在 `response.usage`。

use crate::proto::WireProtocol;

/// 提取出的 usage 计数。全字段 `Option`：未观测到的字段保持 `None`，由
/// `SseUsageParser::feed` 增量 merge（`Some` 覆盖 `None`）。cache_* 仅 Anthropic
/// 协议会填；OpenAI 协议恒为 `None`。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UsageInfo {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
}

impl UsageInfo {
    /// 合并：`other` 中 `Some` 的字段覆盖 `self`。用于增量解析时累加最新观测
    /// （如 Anthropic：message_start 给 input+cache_*，message_delta 后给 output）。
    pub fn merge(&mut self, other: UsageInfo) {
        if other.input_tokens.is_some() {
            self.input_tokens = other.input_tokens;
        }
        if other.output_tokens.is_some() {
            self.output_tokens = other.output_tokens;
        }
        if other.cache_read_tokens.is_some() {
            self.cache_read_tokens = other.cache_read_tokens;
        }
        if other.cache_creation_tokens.is_some() {
            self.cache_creation_tokens = other.cache_creation_tokens;
        }
    }
}

/// 增量 SSE usage 解析器。
///
/// `feed(chunk)` 把字节追加到内部缓冲，按 `\n\n` 事件边界切出完整事件解析，
/// 跨 chunk 的不完整事件保留到下次 `feed`。`finish()` 在流结束时 flush 残余
/// 缓冲（可能含未闭合的尾事件）。解析失败静默跳过，不报错。
pub struct SseUsageParser {
    proto: WireProtocol,
    buf: Vec<u8>,
}

impl SseUsageParser {
    pub fn new(proto: WireProtocol) -> Self {
        SseUsageParser {
            proto,
            buf: Vec::new(),
        }
    }

    /// 喂一个 chunk，返回本 chunk 中新解析出的 usage 增量。无 usage / 解析失败
    /// → 返回全 `None` 的 `UsageInfo`。
    ///
    /// 字节追加到内部缓冲，按 `\n\n` 事件边界切出完整事件解析；跨 chunk 的
    /// 不完整事件保留到下次 `feed`。容错：`[DONE]`、未知事件、坏 JSON 静默跳过。
    pub fn feed(&mut self, chunk: &[u8]) -> UsageInfo {
        self.buf.extend_from_slice(chunk);
        let mut acc = UsageInfo::default();
        // 逐个取出已闭合（含 `\n\n`）的事件解析。每次 drain 到边界 +2 字节。
        while let Some(idx) = find_event_boundary(&self.buf) {
            let event_bytes: Vec<u8> = self.buf.drain(..idx + 2).collect();
            // event_bytes 末尾含 `\n\n`，截掉后两字节得到事件正文。
            let end = event_bytes.len().saturating_sub(2);
            let event_str = String::from_utf8_lossy(&event_bytes[..end]);
            acc.merge(parse_event(self.proto, &event_str));
        }
        acc
    }

    /// 流结束：flush 残余缓冲。即便无 `\n\n` 也尝试解析一次（上游可能未
    /// 闭合最后一个事件就断流）。
    pub fn finish(&mut self) -> UsageInfo {
        let buf = std::mem::take(&mut self.buf);
        if buf.is_empty() {
            return UsageInfo::default();
        }
        let event_str = String::from_utf8_lossy(&buf);
        parse_event(self.proto, &event_str)
    }
}

/// 非流式响应的 usage 解析（见 openspec/specs/router-metrics/spec.md）。body 是完整 JSON。坏 JSON → 全 None。
/// Anthropic 取 top-level `usage.{input,output,cache_read,cache_creation}`；
/// OpenAI 按 key 存在性探测 chat（prompt/completion）vs responses（input/output）。
pub fn parse_json_usage(proto: WireProtocol, body: &[u8]) -> UsageInfo {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return UsageInfo::default();
    };
    match proto {
        WireProtocol::Anthropic => extract_anthropic_json(&v),
        WireProtocol::OpenAi => extract_openai_usage(&v),
    }
}

/// 返回首个 `\n\n` 边界中第一个 `\n` 的索引（无则 `None`）。
fn find_event_boundary(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    (0..buf.len() - 1).find(|&i| buf[i] == b'\n' && buf[i + 1] == b'\n')
}

/// 解析一个完整事件（已剥 `\n\n`）：提取 `data:` 行负载，跳过 `[DONE]`，
/// 解析 JSON 后按协议提取 usage。坏 JSON / 未知事件 → 全 None。
fn parse_event(proto: WireProtocol, event_str: &str) -> UsageInfo {
    // 拼接所有 `data:` 行的负载。SSE spec 多行 data 用换行拼接；LLM 上游
    // 单行居多，简单拼接即可。`data:` 后可能有一个前导空格（按 spec）。
    let mut data_parts: Vec<&str> = Vec::new();
    for line in event_str.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(rest) = line.strip_prefix("data:") {
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            data_parts.push(rest);
        }
    }
    if data_parts.is_empty() {
        return UsageInfo::default();
    }
    let data = data_parts.join("\n");
    // `[DONE]` 标记：跳过。
    if data.trim() == "[DONE]" {
        return UsageInfo::default();
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) else {
        return UsageInfo::default();
    };
    extract_usage_from_value(proto, &v)
}

/// 按协议提取 usage。SSE 流走 type 路由（Anthropic）或 key 探测（OpenAI）。
/// 非 SSE 的 Anthropic 走 `extract_anthropic_json`（top-level usage）。
fn extract_usage_from_value(proto: WireProtocol, v: &serde_json::Value) -> UsageInfo {
    match proto {
        WireProtocol::Anthropic => extract_anthropic_sse_event(v),
        WireProtocol::OpenAi => extract_openai_usage(v),
    }
}

/// Anthropic SSE 事件：`message_start` 给 input + cache_*；`message_delta`
/// 给 output_tokens。其它 type（ping、message_stop、error、未知）跳过。
fn extract_anthropic_sse_event(v: &serde_json::Value) -> UsageInfo {
    let mut info = UsageInfo::default();
    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match ty {
        "message_start" => {
            if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                info.input_tokens = u.get("input_tokens").and_then(as_u64);
                info.cache_read_tokens = u.get("cache_read_input_tokens").and_then(as_u64);
                info.cache_creation_tokens = u.get("cache_creation_input_tokens").and_then(as_u64);
                // message_start 的 output_tokens 是占位（通常 1），不取——
                // output 由 message_delta 的累计值覆盖（brief 契约）。
            }
        }
        "message_delta" => {
            if let Some(u) = v.get("usage")
                && let Some(o) = u.get("output_tokens").and_then(as_u64)
            {
                info.output_tokens = Some(o);
            }
        }
        _ => {}
    }
    info
}

/// Anthropic 非流式响应：top-level `usage.{input,output,cache_read,cache_creation}`。
/// （非流式响应本体即 message，无 `type` 字段路由。）
fn extract_anthropic_json(v: &serde_json::Value) -> UsageInfo {
    let mut info = UsageInfo::default();
    let Some(u) = v.get("usage") else {
        return info;
    };
    info.input_tokens = u.get("input_tokens").and_then(as_u64);
    info.output_tokens = u.get("output_tokens").and_then(as_u64);
    info.cache_read_tokens = u.get("cache_read_input_tokens").and_then(as_u64);
    info.cache_creation_tokens = u.get("cache_creation_input_tokens").and_then(as_u64);
    info
}

/// OpenAI usage 提取（流式与非流式共用）。按 key 存在性探测 shape：
/// - `prompt_tokens` 存在 → chat completions（input=prompt, output=completion）；
/// - `input_tokens` 存在 → Responses API（input/output）。
///
/// 探测位置：top-level `usage`，或 `response.usage`（Responses 流式 completed）。
fn extract_openai_usage(v: &serde_json::Value) -> UsageInfo {
    let mut info = UsageInfo::default();
    let usage = v
        .get("usage")
        .or_else(|| v.get("response").and_then(|r| r.get("usage")));
    let Some(u) = usage else {
        return info;
    };
    // chat shape
    if let Some(p) = u.get("prompt_tokens").and_then(as_u64) {
        info.input_tokens = Some(p);
        info.output_tokens = u.get("completion_tokens").and_then(as_u64);
        return info;
    }
    // responses shape
    if let Some(i) = u.get("input_tokens").and_then(as_u64) {
        info.input_tokens = Some(i);
        info.output_tokens = u.get("output_tokens").and_then(as_u64);
        return info;
    }
    info
}

/// serde_json Number → u64。只取非负整数；浮点/超大 → None（上游 token 计数
/// 恒为非负小整数，as_u64 足够；保留 as_i64 兜底以容忍负数序列化事故）。
fn as_u64(v: &serde_json::Value) -> Option<u64> {
    if let Some(u) = v.as_u64() {
        return Some(u);
    }
    // 兜底：个别上游可能把 token 计数序列化成负数或浮点字符串。
    v.as_i64()
        .and_then(|i| u64::try_from(i).ok().filter(|_| i >= 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- 1. Anthropic 完整流（input + cache + output） ----------------

    #[test]
    fn anthropic_full_stream_input_cache_output() {
        let mut p = SseUsageParser::new(WireProtocol::Anthropic);
        // message_start 带 input + cache_read + cache_creation（output_tokens=1
        // 是占位，按 brief 不取——output 来自 message_delta）。
        let chunk1 = b"event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\
\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":5,\
\"cache_creation_input_tokens\":2,\"output_tokens\":1}}}\n\n";
        // message_delta 给最终 output_tokens。
        let chunk2 = b"event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\
\"usage\":{\"output_tokens\":50}}\n\n";
        let chunk3 = b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

        let mut info = UsageInfo::default();
        info.merge(p.feed(chunk1));
        info.merge(p.feed(chunk2));
        info.merge(p.feed(chunk3));
        info.merge(p.finish());

        assert_eq!(info.input_tokens, Some(10));
        assert_eq!(info.output_tokens, Some(50));
        assert_eq!(info.cache_read_tokens, Some(5));
        assert_eq!(info.cache_creation_tokens, Some(2));
    }

    // ---------------- 2. Anthropic 跨 chunk 截断帧重组 ----------------

    #[test]
    fn anthropic_split_across_chunks_reassembled() {
        let mut p = SseUsageParser::new(WireProtocol::Anthropic);
        // 把 message_start 事件切成三段：header / usage 前半 / usage 后半 + 边界
        let part1 = b"event: message_start\ndata: {\"type\":\"message_start\",\
\"message\":{\"id\":\"msg_x\",\"usage\":{\"input_tokens\":7";
        let part2 = b",\"cache_read_input_tokens\":3,\
\"cache_creation_input_tokens\":1,\"output_tokens\":1}}}\n\n";
        let part3 = b"event: message_delta\ndata: {\"type\":\"message_delta\",\
\"usage\":{\"output_tokens\":42}}\n\n";

        let mut info = UsageInfo::default();
        info.merge(p.feed(part1)); // 不完整事件 → 缓冲，无解析
        assert_eq!(
            info,
            UsageInfo::default(),
            "partial event must yield nothing"
        );
        info.merge(p.feed(part2)); // 现在事件闭合 → 解析出 input + cache
        assert_eq!(info.input_tokens, Some(7));
        assert_eq!(info.cache_read_tokens, Some(3));
        info.merge(p.feed(part3)); // message_delta → output
        info.merge(p.finish());
        assert_eq!(info.output_tokens, Some(42));
        assert_eq!(info.cache_creation_tokens, Some(1));
    }

    // ---------------- 3. OpenAI chat completions shape ----------------

    #[test]
    fn openai_chat_shape_prompt_completion_tokens() {
        let mut p = SseUsageParser::new(WireProtocol::OpenAi);
        // 末尾 chunk 带 usage（chat completions shape）。
        let chunk = b"data: {\"id\":\"chatcmpl-1\",\"choices\":[],\
\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":34,\"total_tokens\":46}}\n\n\
data: [DONE]\n\n";
        let mut info = p.feed(chunk);
        info.merge(p.finish());
        assert_eq!(info.input_tokens, Some(12));
        assert_eq!(info.output_tokens, Some(34));
        assert_eq!(info.cache_read_tokens, None);
        assert_eq!(info.cache_creation_tokens, None);
    }

    // ---------------- 4. OpenAI Responses API shape ----------------

    #[test]
    fn openai_responses_shape_input_output_tokens() {
        let mut p = SseUsageParser::new(WireProtocol::OpenAi);
        // response.completed 事件把 usage 放在 response.usage 下。
        let chunk = b"event: response.completed\ndata: {\"type\":\"response.completed\",\
\"response\":{\"id\":\"resp_1\",\"usage\":{\"input_tokens\":8,\"output_tokens\":20,\
\"total_tokens\":28}}}\n\n";
        let mut info = p.feed(chunk);
        info.merge(p.finish());
        assert_eq!(info.input_tokens, Some(8));
        assert_eq!(info.output_tokens, Some(20));
        assert_eq!(info.cache_read_tokens, None);
        assert_eq!(info.cache_creation_tokens, None);
    }

    // ---------------- 5. `[DONE]` 与未知事件容忍 ----------------

    #[test]
    fn done_marker_and_unknown_events_tolerated() {
        let mut p = SseUsageParser::new(WireProtocol::OpenAi);
        let chunk = b": ping comment\n\nevent: unknown_thing\ndata: {\"foo\":\"bar\"}\n\n\
data: [DONE]\n\n";
        // 全部应被容忍——无 panic、无 usage。
        let mut info = p.feed(chunk);
        info.merge(p.finish());
        assert_eq!(info, UsageInfo::default());

        // Anthropic 侧未知事件同样容忍，且不破坏后续 message_delta。
        let mut pa = SseUsageParser::new(WireProtocol::Anthropic);
        let chunk2 = b"event: ping\ndata: {\"type\":\"ping\"}\n\n\
event: something_new\ndata: {\"type\":\"something_new\",\"x\":1}\n\n\
event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":99}}\n\n";
        let mut info2 = pa.feed(chunk2);
        info2.merge(pa.finish());
        assert_eq!(info2.output_tokens, Some(99));
    }

    // ---------------- 6. 坏 JSON 容忍 ----------------

    #[test]
    fn malformed_json_tolerated() {
        let mut p = SseUsageParser::new(WireProtocol::Anthropic);
        let chunk = b"event: message_start\ndata: {not valid json\n\n\
event: message_delta\ndata: {\"type\":\"message_delta\",\
\"usage\":{\"output_tokens\":7}}\n\n";
        let mut info = p.feed(chunk);
        info.merge(p.finish());
        // 坏 JSON 事件被跳过；后续合法事件仍解析。
        assert_eq!(info.output_tokens, Some(7));
        assert_eq!(info.input_tokens, None);
    }

    // ---------------- 7. parse_json_usage 双协议 ----------------

    #[test]
    fn parse_json_usage_both_protocols() {
        // Anthropic 非流式：top-level usage.{input,output,cache_read,cache_creation}
        let anth = b"{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\
\"content\":[],\"model\":\"claude-sonnet\",\"usage\":{\"input_tokens\":11,\
\"output_tokens\":22,\"cache_read_input_tokens\":4,\"cache_creation_input_tokens\":1}}";
        let info = parse_json_usage(WireProtocol::Anthropic, anth);
        assert_eq!(info.input_tokens, Some(11));
        assert_eq!(info.output_tokens, Some(22));
        assert_eq!(info.cache_read_tokens, Some(4));
        assert_eq!(info.cache_creation_tokens, Some(1));

        // OpenAI chat 非流式：usage.{prompt_tokens, completion_tokens}
        let chat = b"{\"id\":\"chatcmpl-1\",\"choices\":[],\
\"usage\":{\"prompt_tokens\":13,\"completion_tokens\":27,\"total_tokens\":40}}";
        let info = parse_json_usage(WireProtocol::OpenAi, chat);
        assert_eq!(info.input_tokens, Some(13));
        assert_eq!(info.output_tokens, Some(27));
        assert_eq!(info.cache_read_tokens, None);

        // OpenAI Responses 非流式：usage.{input_tokens, output_tokens}
        let resp = b"{\"id\":\"resp_1\",\"usage\":{\"input_tokens\":9,\"output_tokens\":14,\
\"total_tokens\":23}}";
        let info = parse_json_usage(WireProtocol::OpenAi, resp);
        assert_eq!(info.input_tokens, Some(9));
        assert_eq!(info.output_tokens, Some(14));

        // 坏 JSON → 全 None
        let info = parse_json_usage(WireProtocol::Anthropic, b"not json");
        assert_eq!(info, UsageInfo::default());

        // 合法 JSON 但无 usage → 全 None
        let info = parse_json_usage(WireProtocol::OpenAi, b"{\"id\":\"x\"}");
        assert_eq!(info, UsageInfo::default());
    }
}
