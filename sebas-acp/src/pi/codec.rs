//! pi RPC 协议编解码（stdio 严格 JSONL，add-pi-driver D2/D8）。
//!
//! 三族记录（见 /tmp 侧 pi 文档 rpc.md）：stdin 命令、stdout `response`、
//! stdout 会话事件。协议纪律（D8）：
//!
//! - **严格按 LF（`0x0A`）分帧**、容忍前导 CR（CRLF 输入）；不得用会把
//!   U+2028/U+2029 当行边界的通用行读取器（Node readline 语义）——
//!   [`FrameDecoder`] 是字节级切分器，UTF-8 多字节序列里永不出现 `0x0A`，
//!   完整帧只在 LF 到齐后解码，天然免疫该陷阱。
//! - 命令关联用 `pi.id`（异步乱序），事件一般无 id。
//!
//! 事件 → `AcpEvent` 的映射（`translate_event`）与命令组装
//! （`translate_command`）都是纯函数，录制帧 fixture 单测锁形状（D9）。

use crate::session::{AcpCommand, AcpEvent, AvailableCommand, TurnUsage};
use serde_json::Value;

// ── 分帧 ──────────────────────────────────────────────────────────────

/// 字节级 LF 分帧器：持续 push 字节、按 `0x0A` 切帧；帧尾的单个 CR 被剥掉
/// （容忍 CRLF）。不完整帧留在缓冲里等下一批字节；绝不把 U+2028/U+2029
/// 当边界（它们是 3 字节 UTF-8 序列 `E2 80 A8/A9`，不含 `0x0A`）。
pub(crate) struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// 取出下一帧（LF 已剥、可选前导 CR 已剥）；缓冲里没有完整帧时 `None`。
    pub(crate) fn next_frame(&mut self) -> Option<String> {
        let pos = self.buf.iter().position(|&b| b == b'\n')?;
        let mut frame: Vec<u8> = self.buf.drain(..=pos).collect();
        frame.pop(); // LF
        if frame.last() == Some(&b'\r') {
            frame.pop();
        }
        Some(String::from_utf8_lossy(&frame).into_owned())
    }
}

// ── stdout 记录：response / 会话事件 ──────────────────────────────────

/// stdout 的一条记录：命令应答（带 `pi.id` 关联）或会话事件。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PiFrame {
    Response(PiResponse),
    Event(PiEvent),
}

/// `{"id":…,"type":"response","command":…,"success":…,"error":…,"data":…}`。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PiResponse {
    pub id: Option<String>,
    pub command: Option<String>,
    pub success: bool,
    pub error: Option<String>,
    pub data: Option<Value>,
}

/// pi 会话事件（json.md 事件清单）。只对 sebas 消费的形状建类型；其余
/// （compaction_* / auto_retry_* / queue_update / turn_* / message_start…
/// ）归入 [`PiEvent::Other`] 由驱动层记日志——`agent_settled` 是唯一回合
/// 边界，它们不发明新 `AcpEvent`（D2）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PiEvent {
    /// `message_update`：内容块增量 + 顶层累计 usage。
    MessageUpdate {
        usage: Option<PiUsage>,
        assistant_message_event: AssistantMessageEvent,
    },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        partial_result: Value,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: Value,
        is_error: Option<bool>,
    },
    AgentSettled { aborted: bool },
    AgentEnd { will_retry: Option<bool> },
    /// 未映射的事件类型：携带 type 名供日志（前向兼容——pi 升级新增事件
    /// 形状时不炸解码，只是如实记下）。
    Other(String),
}

/// `message_update.assistantMessageEvent`：只取文本/思考增量两个消费者
/// 关心的形状，其余（start/end/toolcall_*/done/error）归 `Other`。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AssistantMessageEvent {
    TextDelta { delta: String },
    ThinkingDelta { delta: String },
    Other(String),
}

/// 事件流携带的累计 usage（报多少是多少，缺项为 `None`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PiUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

impl PiUsage {
    fn is_empty(self) -> bool {
        self.input.is_none()
            && self.output.is_none()
            && self.cache_read.is_none()
            && self.cache_write.is_none()
    }

    fn dedupe_key(self) -> (u64, u64, u64, u64) {
        (
            self.input.unwrap_or(0),
            self.output.unwrap_or(0),
            self.cache_read.unwrap_or(0),
            self.cache_write.unwrap_or(0),
        )
    }
}

/// 解析一行 stdout 记录。空行/非对象行 → `None`（跳过，不炸会话）。
pub(crate) fn parse_frame(line: &str) -> Option<PiFrame> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let obj = v.as_object()?;
    if obj.get("type").and_then(Value::as_str) == Some("response") {
        Some(PiFrame::Response(PiResponse {
            id: str_field(&v, "id"),
            command: str_field(&v, "command"),
            success: v.get("success").and_then(Value::as_bool).unwrap_or(false),
            error: str_field(&v, "error"),
            data: v.get("data").cloned(),
        }))
    } else {
        Some(PiFrame::Event(parse_event(&v)))
    }
}

/// 会话事件的手工分派：按 `type` 取已知形状、其余如实归 `Other`。
pub(crate) fn parse_event(v: &Value) -> PiEvent {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "message_update" => PiEvent::MessageUpdate {
            usage: v.get("usage").map(parse_usage),
            assistant_message_event: v
                .get("assistantMessageEvent")
                .map(parse_assistant_event)
                .unwrap_or(AssistantMessageEvent::Other("missing".into())),
        },
        "tool_execution_start" => PiEvent::ToolExecutionStart {
            tool_call_id: str_field(v, "toolCallId").unwrap_or_default(),
            tool_name: str_field(v, "toolName").unwrap_or_default(),
            args: v.get("args").cloned().unwrap_or(Value::Null),
        },
        "tool_execution_update" => PiEvent::ToolExecutionUpdate {
            tool_call_id: str_field(v, "toolCallId").unwrap_or_default(),
            tool_name: str_field(v, "toolName").unwrap_or_default(),
            partial_result: v.get("partialResult").cloned().unwrap_or(Value::Null),
        },
        "tool_execution_end" => PiEvent::ToolExecutionEnd {
            tool_call_id: str_field(v, "toolCallId").unwrap_or_default(),
            tool_name: str_field(v, "toolName").unwrap_or_default(),
            result: v.get("result").cloned().unwrap_or(Value::Null),
            is_error: v.get("isError").and_then(Value::as_bool),
        },
        "agent_settled" => PiEvent::AgentSettled {
            aborted: v.get("aborted").and_then(Value::as_bool).unwrap_or(false),
        },
        "agent_end" => PiEvent::AgentEnd {
            will_retry: v.get("willRetry").and_then(Value::as_bool),
        },
        _ => PiEvent::Other(kind.to_string()),
    }
}

fn parse_assistant_event(v: &Value) -> AssistantMessageEvent {
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "text_delta" => AssistantMessageEvent::TextDelta {
            delta: str_field(v, "delta").unwrap_or_default(),
        },
        "thinking_delta" => AssistantMessageEvent::ThinkingDelta {
            delta: str_field(v, "delta").unwrap_or_default(),
        },
        other => AssistantMessageEvent::Other(other.to_string()),
    }
}

fn parse_usage(v: &Value) -> PiUsage {
    PiUsage {
        input: v.get("input").and_then(Value::as_u64),
        output: v.get("output").and_then(Value::as_u64),
        cache_read: v.get("cacheRead").and_then(Value::as_u64),
        cache_write: v.get("cacheWrite").and_then(Value::as_u64),
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

// ── stdin 命令 ────────────────────────────────────────────────────────

/// sebas 会发出的 pi RPC 命令（stdin 侧全量）。`id` 用于应答关联（异步
/// 乱序，响应按 id 配对而非到达序）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PiRequest {
    Prompt { id: String, message: String },
    Abort { id: String },
    GetState { id: String },
    GetAvailableModels { id: String },
    GetCommands { id: String },
    SetModel { id: String, provider: String, model_id: String },
}

/// 命令 → 单行 JSONL 帧（严格 LF 结尾）。
pub(crate) fn encode_request(req: &PiRequest) -> String {
    let id = req.id();
    let v = match req {
        PiRequest::Prompt { message, .. } => json_command("prompt", id, |o| {
            o.insert("message".into(), Value::String(message.clone()));
        }),
        PiRequest::Abort { .. } => json_command("abort", id, |_| {}),
        PiRequest::GetState { .. } => json_command("get_state", id, |_| {}),
        PiRequest::GetAvailableModels { .. } => json_command("get_available_models", id, |_| {}),
        PiRequest::GetCommands { .. } => json_command("get_commands", id, |_| {}),
        PiRequest::SetModel {
            provider,
            model_id,
            ..
        } => json_command("set_model", id, |o| {
            o.insert("provider".into(), Value::String(provider.clone()));
            o.insert("modelId".into(), Value::String(model_id.clone()));
        }),
    };
    let mut line = v.to_string();
    line.push('\n');
    line
}

impl PiRequest {
    pub(crate) fn id(&self) -> &str {
        match self {
            PiRequest::Prompt { id, .. }
            | PiRequest::Abort { id }
            | PiRequest::GetState { id }
            | PiRequest::GetAvailableModels { id }
            | PiRequest::GetCommands { id }
            | PiRequest::SetModel { id, .. } => id,
        }
    }
}

fn json_command(
    kind: &str,
    id: &str,
    fill: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), Value::String(id.to_string()));
    obj.insert("type".into(), Value::String(kind.to_string()));
    fill(&mut obj);
    Value::Object(obj)
}

/// 下一个 `pi.id`（会话内单调，`sebas-<n>` 前缀命名空间化）。
pub(crate) fn next_request_id(seq: &mut u64) -> String {
    *seq += 1;
    format!("sebas-{seq}")
}

// ── AcpCommand → PiRequest ────────────────────────────────────────────

/// 统一命令词表 → pi RPC 命令（D2 映射表）：
/// send → `prompt`、取消 → `abort`、SetModel → `set_model`。`SetMode`/
/// `PermissionReply` 无 pi 等价物 → `None`（驱动层如实报不支持 / 忽略）。
pub(crate) fn translate_command(cmd: &AcpCommand, seq: &mut u64) -> Option<PiRequest> {
    match cmd {
        AcpCommand::CreateSession { prompt, .. } | AcpCommand::ContinueSession { prompt, .. } => {
            Some(PiRequest::Prompt {
                id: next_request_id(seq),
                message: prompt.clone(),
            })
        }
        AcpCommand::Cancel { .. } => Some(PiRequest::Abort {
            id: next_request_id(seq),
        }),
        AcpCommand::SetModel { model_id, .. } => {
            let (provider, model_id) = split_model_id(model_id)?;
            Some(PiRequest::SetModel {
                id: next_request_id(seq),
                provider,
                model_id,
            })
        }
        AcpCommand::SetMode { .. } | AcpCommand::PermissionReply { .. } => None,
    }
}

/// 模型选择面的 wire 形态是 `provider/id`（pi `--model` 与 `set_model` 的
/// 词汇）；拆出 provider 与裸模型 id。无 `/` → `None`（调用方如实报错，
/// 不猜 provider）。
pub(crate) fn split_model_id(model_id: &str) -> Option<(String, String)> {
    let (provider, id) = model_id.split_once('/')?;
    if provider.is_empty() || id.is_empty() {
        return None;
    }
    Some((provider.to_string(), id.to_string()))
}

/// `set_model` 失败响应 → 非终态 `Error`（会话仍可用、模型未变）。与通用
/// ACP / claude 驱动同一口径：消息以 [`crate::MODEL_UNCHANGED_MARKER`] 收尾，
/// 引擎据此按「模型切换失败」的终态边收尾回合。
pub(crate) fn translate_set_model_failure(session_id: &str, model_id: &str, error: &str) -> AcpEvent {
    AcpEvent::Error {
        session_id: session_id.to_string(),
        message: format!(
            "set model {model_id:?} 被拒绝（{error}），{}",
            crate::MODEL_UNCHANGED_MARKER
        ),
        terminal: false,
    }
}

// ── 事件 → AcpEvent ───────────────────────────────────────────────────

/// 翻译器状态：usage 去重（`message_update` 每条都带累计 usage，逐条上抛
/// 会按 delta 频率刷屏；只在计数变化时上报一次，终值必与上一帧不同、必达
/// 计费面）。每回合开轮时经 [`Translator::begin_turn`] 复位。
#[derive(Debug, Default)]
pub(crate) struct Translator {
    last_usage: Option<(u64, u64, u64, u64)>,
}

impl Translator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 新回合：usage 从头累计（pi 的 usage 是「当前 assistant 响应」的累计
    /// 值，跨回合不复位会让第二回合的零头被去重吞掉）。
    pub(crate) fn begin_turn(&mut self) {
        self.last_usage = None;
    }
}

/// pi 会话事件 → `AcpEvent`（D2 映射表）。回合边界（`agent_settled` →
/// `Finished`）与进程退出终态由驱动层处理，本函数不产出。
pub(crate) fn translate_event(
    session_id: &str,
    event: &PiEvent,
    translator: &mut Translator,
) -> Vec<AcpEvent> {
    let sid = || session_id.to_string();
    match event {
        PiEvent::MessageUpdate {
            usage,
            assistant_message_event,
        } => {
            let mut out = Vec::new();
            if let Some(u) = usage
                && !u.is_empty()
                && translator.last_usage.as_ref() != Some(&u.dedupe_key())
            {
                translator.last_usage = Some(u.dedupe_key());
                out.push(AcpEvent::UsageUpdate {
                    session_id: sid(),
                    usage: TurnUsage {
                        model: None,
                        input_tokens: u.input,
                        output_tokens: u.output,
                        cache_read_input_tokens: u.cache_read,
                        cache_creation_input_tokens: u.cache_write,
                    },
                });
            }
            match assistant_message_event {
                AssistantMessageEvent::TextDelta { delta } if !delta.is_empty() => {
                    out.push(AcpEvent::TextDelta {
                        session_id: sid(),
                        delta: delta.clone(),
                    })
                }
                AssistantMessageEvent::ThinkingDelta { delta } if !delta.is_empty() => {
                    out.push(AcpEvent::ThinkingDelta {
                        session_id: sid(),
                        delta: delta.clone(),
                    })
                }
                _ => {}
            }
            out
        }
        PiEvent::ToolExecutionStart {
            tool_name, args, ..
        } => vec![AcpEvent::ToolStart {
            session_id: sid(),
            tool_name: tool_name.clone(),
            args: args.clone(),
        }],
        PiEvent::ToolExecutionUpdate {
            tool_name,
            partial_result,
            ..
        } => vec![AcpEvent::ToolProgress {
            session_id: sid(),
            tool_name: tool_name.clone(),
            progress: render_tool_payload(partial_result),
        }],
        PiEvent::ToolExecutionEnd {
            tool_name,
            result,
            ..
        } => vec![AcpEvent::ToolEnd {
            session_id: sid(),
            tool_name: tool_name.clone(),
            result: render_tool_payload(result),
        }],
        PiEvent::AgentSettled { .. } | PiEvent::AgentEnd { .. } | PiEvent::Other(_) => Vec::new(),
    }
}

/// 工具结果载荷（`{"content":[{"type":"text","text":…}],…}`）→ 展示字符串：
/// 拼接 content 里的 text 块；没有 text 块时序列化整个对象（保真兜底）。
fn render_tool_payload(v: &Value) -> String {
    if let Some(blocks) = v.get("content").and_then(Value::as_array) {
        let texts: Vec<&str> = blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect();
        if !texts.is_empty() {
            return texts.join("\n");
        }
    }
    if v.is_null() {
        return String::new();
    }
    v.to_string()
}

// ── 握手应答解析 ──────────────────────────────────────────────────────

/// `get_state` 应答的会话身份：pi 会话 id（握手上报，对齐通用 ACP 驱动的
/// `acp_session_id` 语义）与当前模型（可缺省——pi 未选模型时省略）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PiSessionState {
    pub session_id: String,
    pub current_model: Option<(String, String)>, // (provider, id)
}

pub(crate) fn parse_state_data(data: &Value) -> Option<PiSessionState> {
    let session_id = str_field(data, "sessionId")?;
    let current_model = data.get("model").and_then(|m| {
        let provider = str_field(m, "provider")?;
        let id = str_field(m, "id")?;
        Some((provider, id))
    });
    Some(PiSessionState {
        session_id,
        current_model,
    })
}

/// `get_available_models` 应答 → 模型选择面。wire 形态 `provider/id`（与
/// `set_model`/`--model` 同词汇）；不内置硬编码模型表（D2）。
pub(crate) fn parse_models_data(data: &Value) -> Vec<(String, String)> {
    let Some(models) = data.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    models
        .iter()
        .filter_map(|m| {
            let provider = str_field(m, "provider")?;
            let id = str_field(m, "id")?;
            Some((provider, id))
        })
        .collect()
}

/// `get_commands` 应答 → 会话命令面板（缺 name 跳条、缺 description 置空
/// ——诚实退化，空表 = 无面板）。
pub(crate) fn parse_commands_data(data: &Value) -> Vec<AvailableCommand> {
    let Some(commands) = data.get("commands").and_then(Value::as_array) else {
        return Vec::new();
    };
    commands
        .iter()
        .filter_map(|c| {
            let name = str_field(c, "name")?;
            Some(AvailableCommand {
                name,
                description: str_field(c, "description").unwrap_or_default(),
                hint: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 1.1 分帧纪律（D8）──

    /// U+2028/U+2029 出现在 JSON 字符串里**不是**帧边界：完整帧必须原样
    /// 取出、帧内字符保真。这是「不得用通用行读取器」的红线测试——按
    /// `char::is_line_terminator` 切行的实现必红。
    #[test]
    fn u2028_u2029_inside_json_string_are_not_frame_boundaries() {
        let json = format!("{{\"delta\":\"a\u{2028}b\u{2029}c\"}}");
        let mut raw = json.clone().into_bytes();
        raw.push(b'\n');
        raw.extend_from_slice(b"{\"next\":1}\n");

        let mut dec = FrameDecoder::new();
        dec.push(&raw);
        assert_eq!(dec.next_frame().as_deref(), Some(json.as_str()));
        assert_eq!(dec.next_frame().as_deref(), Some("{\"next\":1}"));
        assert_eq!(dec.next_frame(), None);
    }

    /// 字节流任意切分（多字节 UTF-8 跨 chunk）：帧边界只认 `0x0A`。
    #[test]
    fn frames_split_across_arbitrary_chunk_boundaries() {
        let raw = "{\"t\":\"中文\u{2028}x\"}\r\n{\"t\":2}\n".as_bytes().to_vec();
        // 单字节逐个 push——最极端的切分。
        let mut dec = FrameDecoder::new();
        let mut frames = Vec::new();
        for b in &raw {
            dec.push(std::slice::from_ref(b));
            while let Some(f) = dec.next_frame() {
                frames.push(f);
            }
        }
        assert_eq!(frames, vec!["{\"t\":\"中文\u{2028}x\"}".to_string(), "{\"t\":2}".to_string()]);
    }

    /// 前导 CR 容忍（CRLF 输入剥一个 CR；只有 CR 没有 LF 不成帧）。
    #[test]
    fn leading_cr_is_stripped_and_bare_cr_is_not_a_boundary() {
        let mut dec = FrameDecoder::new();
        dec.push(b"{\"a\":1}\r");
        assert_eq!(dec.next_frame(), None, "只有 CR 没有 LF 不成帧");
        dec.push(b"\n");
        assert_eq!(dec.next_frame().as_deref(), Some("{\"a\":1}"));
    }

    // ── 1.2 事件翻译（录制帧 fixture）──

    fn ev(line: &str) -> PiEvent {
        match parse_frame(line).expect("frame parses") {
            PiFrame::Event(e) => e,
            other => panic!("expected event, got {other:?}"),
        }
    }

    fn resp(line: &str) -> PiResponse {
        match parse_frame(line).expect("frame parses") {
            PiFrame::Response(r) => r,
            other => panic!("expected response, got {other:?}"),
        }
    }

    /// `AcpEvent` 未实现 `PartialEq`（词表类型不为本测试让步）——经 serde
    /// 序列化比对，形状锁反而更严（wire 形状逐键一致）。
    fn assert_events(actual: Vec<AcpEvent>, expected: Vec<AcpEvent>) {
        let ser = |e: &AcpEvent| serde_json::to_value(e).unwrap();
        let a: Vec<Value> = actual.iter().map(ser).collect();
        let e: Vec<Value> = expected.iter().map(ser).collect();
        assert_eq!(a, e);
    }

    /// 文档示例的完整回合事件序列 → 统一词表：text/thinking 增量、工具
    /// 三段生命周期、usage 逐条到达（去重后只有变化值上抛）。
    #[test]
    fn recorded_turn_maps_to_the_shared_vocabulary() {
        let sid = "s-1";
        let mut tr = Translator::new();

        // text_delta（带 usage）→ TextDelta + UsageUpdate。
        let events = translate_event(
            sid,
            &ev(r#"{"type":"message_update","usage":{"input":100,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":101,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hello "}}"#),
            &mut tr,
        );
        assert_events(
            events,
            vec![
                AcpEvent::UsageUpdate {
                    session_id: sid.into(),
                    usage: TurnUsage {
                        model: None,
                        input_tokens: Some(100),
                        output_tokens: Some(1),
                        cache_read_input_tokens: Some(0),
                        cache_creation_input_tokens: Some(0),
                    },
                },
                AcpEvent::TextDelta {
                    session_id: sid.into(),
                    delta: "Hello ".into(),
                },
            ],
        );

        // thinking_delta → ThinkingDelta。
        let events = translate_event(
            sid,
            &ev(r#"{"type":"message_update","usage":{"input":100,"output":1,"cacheRead":0,"cacheWrite":0},"assistantMessageEvent":{"type":"thinking_delta","contentIndex":1,"delta":"hmm"}}"#),
            &mut tr,
        );
        assert_events(
            events,
            vec![AcpEvent::ThinkingDelta {
                session_id: sid.into(),
                delta: "hmm".into(),
            }],
        );

        // 工具三段：start（args 原样）/ update（partialResult 的 text 块）/
        // end（result 的 text 块）。
        let events = translate_event(
            sid,
            &ev(r#"{"type":"tool_execution_start","toolCallId":"call_abc","toolName":"bash","args":{"command":"ls -la"}}"#),
            &mut tr,
        );
        assert_events(
            events,
            vec![AcpEvent::ToolStart {
                session_id: sid.into(),
                tool_name: "bash".into(),
                args: serde_json::json!({"command": "ls -la"}),
            }],
        );
        let events = translate_event(
            sid,
            &ev(r#"{"type":"tool_execution_update","toolCallId":"call_abc","toolName":"bash","args":{"command":"ls -la"},"partialResult":{"content":[{"type":"text","text":"partial output"}],"details":{}}}"#),
            &mut tr,
        );
        assert_events(
            events,
            vec![AcpEvent::ToolProgress {
                session_id: sid.into(),
                tool_name: "bash".into(),
                progress: "partial output".into(),
            }],
        );
        let events = translate_event(
            sid,
            &ev(r#"{"type":"tool_execution_end","toolCallId":"call_abc","toolName":"bash","result":{"content":[{"type":"text","text":"complete output"}],"details":{}},"isError":false,"durationMs":12}"#),
            &mut tr,
        );
        assert_events(
            events,
            vec![AcpEvent::ToolEnd {
                session_id: sid.into(),
                tool_name: "bash".into(),
                result: "complete output".into(),
            }],
        );

        // agent_settled / agent_end：翻译层不产出（回合边界归驱动层）。
        assert!(translate_event(sid, &ev(r#"{"type":"agent_settled","aborted":false}"#), &mut tr).is_empty());
        assert!(translate_event(sid, &ev(r#"{"type":"agent_end","messages":[],"willRetry":false}"#), &mut tr).is_empty());
    }

    /// usage 去重：同值不重复上抛、变化必达；回合复位后零值也重新上报。
    #[test]
    fn usage_is_deduped_until_it_changes_or_the_turn_resets() {
        let sid = "s-1";
        let mut tr = Translator::new();
        let frame = |usage: &str| {
            ev(&format!(
                r#"{{"type":"message_update","usage":{usage},"assistantMessageEvent":{{"type":"text_delta","contentIndex":0,"delta":"x"}}}}"#
            ))
        };
        let zero = r#"{"input":10,"output":0,"cacheRead":0,"cacheWrite":0}"#;
        // 首条：usage 上抛（10/0/0/0）。
        let n = translate_event(sid, &frame(zero), &mut tr)
            .iter()
            .filter(|e| matches!(e, AcpEvent::UsageUpdate { .. }))
            .count();
        assert_eq!(n, 1);
        // 同值：去重（只剩 TextDelta）。
        let events = translate_event(sid, &frame(zero), &mut tr);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AcpEvent::TextDelta { .. }));
        // 变化（output 1→2）：必达。
        let grown = r#"{"input":10,"output":2,"cacheRead":0,"cacheWrite":0}"#;
        let events = translate_event(sid, &frame(grown), &mut tr);
        assert!(events.iter().any(|e| matches!(
            e,
            AcpEvent::UsageUpdate { usage, .. } if usage.output_tokens == Some(2)
        )));
        // 新回合复位：回到 10/0/0/0 也重新上报（累计口径按回合）。
        tr.begin_turn();
        let events = translate_event(sid, &frame(zero), &mut tr);
        assert!(events.iter().any(|e| matches!(e, AcpEvent::UsageUpdate { .. })));
    }

    /// 未映射事件（compaction / auto_retry / queue_update / 未知新形状）：
    /// 不炸解码、不产出事件、type 名如实保留供日志。
    #[test]
    fn unmapped_events_carry_their_type_name() {
        assert_eq!(
            ev(r#"{"type":"compaction_start","reason":"threshold"}"#),
            PiEvent::Other("compaction_start".into())
        );
        assert_eq!(
            ev(r#"{"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":2000,"errorMessage":"529"}"#),
            PiEvent::Other("auto_retry_start".into())
        );
        assert_eq!(
            ev(r#"{"type":"queue_update","steering":[],"followUp":[]}"#),
            PiEvent::Other("queue_update".into())
        );
        assert_eq!(ev(r#"{"type":"brand_new_thing"}"#), PiEvent::Other("brand_new_thing".into()));
    }

    /// response 帧解析：id/command/success/error/data 五槽位（malformed parse
    /// response 无 id 也能读）。
    #[test]
    fn response_frames_parse_with_and_without_id() {
        let r = resp(
            r#"{"id":"req-1","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}"#,
        );
        assert_eq!(r.id.as_deref(), Some("req-1"));
        assert_eq!(r.command.as_deref(), Some("prompt"));
        assert!(r.success);
        assert_eq!(
            r.data.as_ref().and_then(|d| d.get("disposition")).and_then(Value::as_str),
            Some("started")
        );

        let parse_err = resp(
            r#"{"type":"response","command":"parse","success":false,"error":"Failed to parse command: ..."}"#,
        );
        assert_eq!(parse_err.id, None);
        assert!(!parse_err.success);
        assert!(parse_err.error.unwrap().contains("Failed to parse"));
    }

    // ── 1.3 命令组装 ──

    fn cmd_of(line: &str) -> AcpCommand {
        serde_json::from_str(line).unwrap()
    }

    /// send → prompt、取消 → abort、SetModel → set_model（provider/id 拆分
    /// + `pi.id` 关联单调递增）；SetMode/PermissionReply 无等价物 → None。
    #[test]
    fn commands_translate_with_monotonic_ids() {
        let mut seq = 0u64;
        let req = translate_command(
            &cmd_of(r#"{"type":"create_session","session_id":"s","prompt":"hi"}"#),
            &mut seq,
        )
        .unwrap();
        match req {
            PiRequest::Prompt { id, message } => {
                assert_eq!(id, "sebas-1");
                assert_eq!(message, "hi");
            }
            other => panic!("prompt expected: {other:?}"),
        }

        let req = translate_command(&cmd_of(r#"{"type":"cancel","session_id":"s"}"#), &mut seq).unwrap();
        assert_eq!(req, PiRequest::Abort { id: "sebas-2".into() });

        let req = translate_command(
            &cmd_of(r#"{"type":"set_model","session_id":"s","model_id":"anthropic/claude-sonnet-4"}"#),
            &mut seq,
        )
        .unwrap();
        assert_eq!(
            req,
            PiRequest::SetModel {
                id: "sebas-3".into(),
                provider: "anthropic".into(),
                model_id: "claude-sonnet-4".into(),
            }
        );

        assert!(translate_command(&cmd_of(r#"{"type":"set_mode","session_id":"s","mode":"ask"}"#), &mut seq).is_none());
        assert!(translate_command(
            &cmd_of(r#"{"type":"permission_reply","session_id":"s","request_id":"r","decision":{"decision":"allow_once"}}"#),
            &mut seq,
        )
        .is_none());
    }

    /// 命令帧编码：单行 JSON + LF，键序（id/type 在前）与载荷形状可读回。
    #[test]
    fn encoded_requests_are_single_line_jsonl() {
        let line = encode_request(&PiRequest::Prompt {
            id: "sebas-1".into(),
            message: "Hello\nworld".into(),
        });
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1, "载荷里的换行必须被 JSON 转义");
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["id"], "sebas-1");
        assert_eq!(v["type"], "prompt");
        assert_eq!(v["message"], "Hello\nworld");

        let line = encode_request(&PiRequest::SetModel {
            id: "sebas-2".into(),
            provider: "anthropic".into(),
            model_id: "claude-sonnet-4".into(),
        });
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["type"], "set_model");
        assert_eq!(v["provider"], "anthropic");
        assert_eq!(v["modelId"], "claude-sonnet-4");
    }

    /// 模型 id 拆分：`provider/id` 有效；裸 id / 空 provider / 尾斜杠 → None
    /// （调用方如实报错，不猜 provider）。
    #[test]
    fn model_id_split_requires_provider_and_id() {
        assert_eq!(
            split_model_id("anthropic/claude-sonnet-4"),
            Some(("anthropic".into(), "claude-sonnet-4".into()))
        );
        assert_eq!(split_model_id("claude-sonnet-4"), None);
        assert_eq!(split_model_id("/claude"), None);
        assert_eq!(split_model_id("anthropic/"), None);
        assert_eq!(split_model_id("a/b/c"), Some(("a".into(), "b/c".into())));
    }

    /// set_model 失败响应翻译：非终态 Error + 稳定标记（引擎按模型切换
    /// 失败收尾，会话保持可用）。
    #[test]
    fn set_model_failure_translates_to_non_terminal_error_with_marker() {
        let evt = translate_set_model_failure("s-1", "anthropic/nope", "Model not found: nope");
        match evt {
            AcpEvent::Error {
                session_id,
                message,
                terminal,
            } => {
                assert_eq!(session_id, "s-1");
                assert!(!terminal, "会话保持可用");
                assert!(message.contains("anthropic/nope"));
                assert!(message.contains(crate::MODEL_UNCHANGED_MARKER));
            }
            other => panic!("error expected: {other:?}"),
        }
    }

    // ── 握手应答解析 ──

    /// get_state / get_available_models / get_commands 应答形状（文档示例）：
    /// 会话 id 与当前模型、模型面（provider/id 词汇）、命令面板。
    #[test]
    fn handshake_response_parsers_match_the_documented_shapes() {
        let state = parse_state_data(
            &serde_json::json!({
                "model": {"id": "claude-sonnet-4-20250514", "provider": "anthropic"},
                "thinkingLevel": "medium",
                "isStreaming": false,
                "sessionFile": "/path/to/session.jsonl",
                "sessionId": "abc123",
            }),
        )
        .unwrap();
        assert_eq!(state.session_id, "abc123");
        assert_eq!(
            state.current_model,
            Some(("anthropic".into(), "claude-sonnet-4-20250514".into()))
        );

        // 未选模型：model 键省略，session id 仍在。
        let state = parse_state_data(&serde_json::json!({"sessionId": "abc"})).unwrap();
        assert_eq!(state.current_model, None);
        // 缺 sessionId → None（握手诚实失败，由驱动层处理）。
        assert!(parse_state_data(&serde_json::json!({"isStreaming": false})).is_none());

        let models = parse_models_data(&serde_json::json!({
            "models": [
                {"id": "claude-sonnet-4", "provider": "anthropic"},
                {"id": "gpt-5.4", "provider": "openai"},
                {"name": "broken entry"},
            ]
        }));
        assert_eq!(
            models,
            vec![
                ("anthropic".into(), "claude-sonnet-4".into()),
                ("openai".into(), "gpt-5.4".into()),
            ]
        );

        let commands = parse_commands_data(&serde_json::json!({
            "commands": [
                {"name": "fix-tests", "description": "Fix failing tests", "source": "prompt"},
                {"description": "no name, skipped"},
            ]
        }));
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "fix-tests");
        assert_eq!(commands[0].description, "Fix failing tests");
    }
}
