//! Session event stream + snapshot shapes for out-of-process observers
//! (openspec/changes/add-core-session-channel — tasks 1.1/1.3).
//!
//! `SessionInfo` is the externally visible view of one session: the mapping
//! state joined with the card-derived fields the WebUI renders. `SessionEvent`
//! is published on a bounded broadcast in `DispatchHandle` for every mapping
//! mutation (create / status-or-phase change / removal). `TurnEntry` is one
//! block of a session's rendered transcript, addressed by a monotonic
//! position so channel clients can fetch only what they have not seen.
//!
//! These types are serde-native by design: they cross the core session
//! channel as newline-delimited JSON. (`CardElement` deliberately is not
//! `Serialize` — the transcript carries rendered view shapes instead.)
//!
//! The session is addressed by its flattened [`ChannelKey`]: `channel` is the
//! originating channel name (`"feishu"`, `"web"`, ...) and `key` is the
//! channel-neutral reference, opaque to the core (feishu's reference encodes
//! `chat_id\0thread_id`; only the feishu adapter interprets it).

use serde::{Deserialize, Serialize};

use sebas_channels::ChannelKey;

/// 远端会话的呈现信息（add-remote-execution-node 8.x）。
///
/// 只有**执行节点上的**会话带这个结构；主控本机的会话是 `None`——本机没有
/// 「节点在线吗」这个维度，硬填一个 `online` 只会让展示层分不清两种情形。
///
/// 这些字段回答操作者要看的四个问题：它跑在哪台机器上（`node_id`）、那台机器
/// 现在通不通（`node_status` / `node_cause`）、它到底按哪个 mode 在跑
/// （`desired_mode` vs `effective_mode`）、以及它是不是**在等人批**而不是在干活
/// （`parked_approvals`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSessionView {
    /// 会话所在执行节点的稳定标识（`[node] id`）。
    pub node_id: String,
    /// 节点对该会话的可判定状态：`online` / `offline` / `terminated` /
    /// `gone`（节点侧已不存在）。
    pub node_status: String,
    /// 状态的成因（离线/终止的原因）。如实陈述——`NodeOffline` 与
    /// `Terminated` 是两件事，展示层不该把后者说成「暂时联系不上」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_cause: Option<String>,
    /// 会话**期望**的 mode（`ask` / `edit` / `allow` / `auto`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_mode: Option<String>,
    /// 执行体**实际强制**的 mode。与 `desired_mode` 不同即「执行体强制不了」，
    /// 展示层必须两个都显示并说明差异（不能只显示期望值假装已生效）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_mode: Option<String>,
    /// 仍在等主控决定的悬空审批数；`> 0` 表示会话**在等**，不是在跑。
    #[serde(default)]
    pub parked_approvals: u32,
    /// 会话**期望**使用的节点 provider profile 名（7.1）。`None` = 没点名，
    /// 由节点用它自己的默认。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_provider: Option<String>,
    /// **实际生效**的 provider profile 名。与期望不同即"应用不上"，
    /// 界面必须两个都显示（`provider_cause` 说明为什么）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// provider 应用不上的成因（节点侧如实回报）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_cause: Option<String>,
}

/// One session as the outside world sees it: mapping state joined with the
/// card-derived fields the WebUI renders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInfo {
    /// Originating channel name (`"feishu"`, `"web"`, ...).
    pub channel: String,
    /// Channel-neutral opaque reference within that channel.
    pub key: String,
    /// Live routing id — `None` for Spawning placeholders.
    pub session_id: Option<String>,
    /// `"spawning"` | `"active"` | `"dormant"`.
    pub status: String,
    /// Card phase emoji (`SEED`/`OnIt`/`DONE`/`CrossMark`) when a card exists.
    pub phase: Option<String>,
    /// Current turn's user prompt, when a card exists.
    pub user_prompt: Option<String>,
    pub last_active_unix: i64,
    /// Working directory for project sessions (WebUI-spawned).
    pub project_dir: Option<String>,
    /// （add-acp-model-selection）会话当前的模型 id；`None` = agent 未暴露
    /// 模型选项（webui 不显示模型 UI），或会话尚无模型信息（Spawning）。
    #[serde(default)]
    pub current_model: Option<String>,
    /// 该 ACP 会话可选的模型 id 列表（来自 agent 的 `configOptions`，非硬编码）；
    /// `None`/空 = 无模型选择面。webui 创建会话下拉的数据源。
    #[serde(default)]
    pub available_models: Option<Vec<String>>,
    /// 会话创建时绑定的执行后端 kind（add-composer-agent-binding；源自
    /// mapping 的 `pending_kind`，spawn 后不清除）。`None` = 配置的默认
    /// kind（解析留给展示层）。`#[serde(default)]` 兼容旧事件/旧快照。
    #[serde(default)]
    pub agent_kind: Option<String>,
    /// （extract-im-service 2.3）累计 token 用量（卡片 footer 的中立数据源）。
    /// `None` = 尚无 usage 事件。`#[serde(default)]` 兼容旧快照/旧事件。
    #[serde(default)]
    pub usage: Option<sebas_channels::card::AppUsage>,
    /// （wire-webui-sebas-agent-e2e D4）会话所属执行体：`"acp"` / `"native"`，
    /// 由复合后端在快照/事件中转时打标。`#[serde(default)]` 兼容旧报文
    /// （缺字段 = 未打标，展示层回退到 agent_kind / 默认执行体）。
    #[serde(default)]
    pub backend: Option<String>,
    /// （workbench-turn-queue D6）待生效提交全量视图（投递序，staging 先于
    /// turn 队列）。快照与每次会话事件都携带；`#[serde(default)]` 兼容旧
    /// 快照/旧事件。
    #[serde(default)]
    pub pending: Vec<crate::state::PendingSubmission>,
    /// （add-remote-execution-node 8.x）远端会话的节点/mode/悬空审批呈现信息；
    /// `None` = 主控本机会话（这些维度对它不存在）。`#[serde(default)]` 兼容旧
    /// 报文：旧客户端收不到该键，新客户端收到 `null` 时按本机会话处理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteSessionView>,
    /// （add-agent-mode-selection）操作者期望的会话 mode（控制面词汇
    /// `ask`/`edit`/`allow`/`auto`）：创建请求携带、中途切换立即更新。
    /// `None` = agent 默认行为。`#[serde(default)]` 兼容旧快照/旧事件。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_mode: Option<String>,
    /// （add-agent-mode-selection）执行体回报的**实际生效** mode（本机 =
    /// spawn argv 应用值 / `ModeChanged`；远端 = 节点回报，见 `remote`）。
    /// `None` = 执行体未声称任何 mode 生效——desired/effective 的差异如实
    /// 可见（execution-node spec："mode enforceability is declared, not
    /// assumed"）。`#[serde(default)]` 兼容旧快照/旧事件。
    #[serde(default)]
    pub effective_mode: Option<String>,
    /// （rail-declutter-unread D1/D2）会话累计「可见回复段」数——rail 未读
    /// 徽标的服务端数据源。口径见 [`count_chat_messages`]；随 `session.updated`
    /// 广播（transcript flush 多数时机不发事件，rail 的 10s 轮询兜底）。
    /// `#[serde(default)]` 兼容旧快照/旧事件。
    #[serde(default)]
    pub msg_count: u64,
}

/// 可见回复段的计数口径（rail-declutter-unread D2，用户拍板「可见回复段」）：
///
/// - 只数 `kind = "content"` 且 `element_type ∈ {markdown, error}` 的条目；
///   prompt、thinking、tool 一律不计；
/// - **相邻连续的 markdown 条目合并为一段**（ACP 路径逐 delta 落账，一段
///   流式回复 = 一串 delta 条目；与前端 transcript 把连续 markdown 拼进同
///   一个文本块同粒度），被 prompt / thinking / tool / error 打断则另起一段；
/// - 每条 `error` 条目独立计 1（前端逐条渲染错误气泡）；
/// - 空内容条目跳过、**不断段**（前端 `groupConversation` 同规则）。
///
/// 注意与 seen-boundary seam 的差异：seam 按**轮**分界，徽标按**段**计数
/// ——两者共用同一游标（D3）但聚合粒度不同，属有意为之，测试分别钉住。
pub fn count_chat_messages(entries: &[TurnEntry]) -> u64 {
    let mut count = 0u64;
    let mut in_markdown_run = false;
    for e in entries {
        if e.content.is_empty() {
            // 空条目对前端不可见：不计数、不打断当前段。
            continue;
        }
        if e.kind != "content" {
            in_markdown_run = false;
            continue;
        }
        match e.element_type.as_str() {
            "markdown" => {
                if !in_markdown_run {
                    count += 1;
                    in_markdown_run = true;
                }
            }
            "error" => {
                count += 1;
                in_markdown_run = false;
            }
            _ => in_markdown_run = false,
        }
    }
    count
}

impl SessionInfo {
    /// The flattened [`ChannelKey`] this session belongs to.
    pub fn channel_key(&self) -> ChannelKey {
        ChannelKey::new(self.channel.clone(), self.key.clone())
    }
}

/// Session change event published on the router's broadcast channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    /// A mapping was inserted (Spawning placeholder or restored Dormant).
    Created { session: SessionInfo },
    /// Status or phase changed (Spawning→Active, Dormant→Spawning resume,
    /// project_dir set, card emoji transition).
    Updated { session: SessionInfo },
    /// The mapping was removed (web close, terminal error, failed spawn).
    /// `channel`/`key` flatten the removed [`ChannelKey`].
    Removed { channel: String, key: String },
    /// （workbench-turn-queue D5）会话终结/关闭时未执行的待生效提交——core
    /// 在移除映射**之前**发出，携带被丢弃条目的 id + 文本，观察者据此给出
    /// 「未执行」提示。丢弃绝不静默。
    PendingDropped {
        channel: String,
        key: String,
        dropped: Vec<crate::state::PendingSubmission>,
    },
    /// Emitted by channel clients (never by the router itself) after a
    /// reconnect: subscribers should re-snapshot because the client resumed
    /// from a fresh snapshot and the view must converge. See the channel
    /// spec's "reconnect resumes from a snapshot" scenario.
    Resync,
}

/// One rendered block of a session's transcript, addressed by a monotonic
/// position. `kind` distinguishes the user's prompt from agent/tool output;
/// `element_type` tells the client how to render `content`
/// (`"markdown"` | `"thinking"` | `"tool"` | `"error"`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnEntry {
    /// 0-based monotonic position within the session's transcript.
    pub position: u64,
    /// `"prompt"` (user turn input) or `"content"` (agent/tool output).
    pub kind: String,
    /// `"markdown"` | `"thinking"` | `"tool"` | `"error"`.
    pub element_type: String,
    pub content: String,
    /// Unix seconds when this entry was appended. Lets the client render a
    /// flush-left timestamp next to each block (spec 4.1) and lets the
    /// client anchor the seen-boundary seam to a stable element identity
    /// that survives in-place card refresh (spec 4.4 — older refreshes
    /// don't bump `position`, so a seam anchored by `position` alone would
    /// drift onto a different element; the timestamp is the canonical
    /// identity that doesn't change once written).
    pub created_at_unix: u64,
    /// （workbench-agent-identity-and-process-folds 1.1/1.2）工具条目的结构化
    /// 折叠标题（如 `Read · src/main.rs` / `✓ Read · src/main.rs`）：工具名 +
    /// 关键参数摘要，前端二级折叠收起时显示。`None` = 旧持久化条目无标题，
    /// 前端回退通用标签。`#[serde(default)]` 兼容旧 JSON（缺字段 → None，
    /// 无需迁移）；`skip_serializing_if` 让 None 不上 wire（只增可选字段，
    /// 不破坏旧消费端）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl TurnEntry {
    pub fn prompt(position: u64, content: impl Into<String>) -> Self {
        Self::new(position, "prompt", "markdown", content)
    }

    pub fn markdown(position: u64, content: impl Into<String>) -> Self {
        Self::new(position, "content", "markdown", content)
    }

    pub fn thinking(position: u64, content: impl Into<String>) -> Self {
        Self::new(position, "content", "thinking", content)
    }

    /// 工具调用条目（workbench-conversation-view 1.3，design D2）：内容仍是
    /// 可读 markdown，但 `element_type = "tool"` 让客户端能把工具调用与正文
    /// 区分开（收进「用了 N 个工具」可展开组），不再靠 emoji 前缀当契约。
    pub fn tool(position: u64, content: impl Into<String>) -> Self {
        Self::new(position, "content", "tool", content)
    }

    /// spawn 失败等启动期错误条目（fail-fast-on-startup-errors 3.1）：
    /// `kind = "content"`（core 产生，非操作员提交）+ `element_type = "error"`，
    /// 前端据此渲染为带计数的错误气泡而非普通 markdown。（kind 词汇收敛为
    /// prompt|content 两值是 workbench-conversation-view 的 delta 契约。）
    pub fn error(position: u64, content: impl Into<String>) -> Self {
        Self::new(position, "content", "error", content)
    }

    fn new(position: u64, kind: &str, element_type: &str, content: impl Into<String>) -> Self {
        Self {
            position,
            kind: kind.into(),
            element_type: element_type.into(),
            content: content.into(),
            // The router stamps the wall-clock at push time so every
            // entry carries the moment it was appended, not the moment
            // the helper was called.
            created_at_unix: now_unix_secs(),
            title: None,
        }
    }

    /// 附加结构化标题（workbench-agent-identity-and-process-folds 1.2）：
    /// 仅工具条目使用，链在 [`TurnEntry::tool`] 之后；其余条目保持 `None`。
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// 折叠标题的硬上限（workbench-agent-identity-and-process-folds 1.2）：
/// 工具名 + 参数摘要整体 200 字符，防异常超长参数撑爆折叠栏。
const TITLE_MAX_CHARS: usize = 200;

/// 关键参数的偏好键序（design D2）：路径类 > 模式 > URL > 命令 > 查询，
/// 覆盖 read/edit/write、glob/grep、web fetch/search、bash 等主流工具的
/// 实际参数形态。
const KEY_ARG_PREFERRED: [&str; 11] = [
    "path",
    "file_path",
    "absolute_path",
    "file",
    "dir",
    "directory",
    "cwd",
    "pattern",
    "url",
    "command",
    "query",
];

/// 从工具 args JSON 提取关键参数摘要（workbench-agent-identity-and-process-folds
/// 1.2）：偏好键序取第一个**非空**字符串值；全未命中退化为对象里第一个
/// 字符串值；再没有（非对象 / 无字符串值）返回 `None`，标题退化为纯工具名。
fn key_arg_from_args(args: &serde_json::Value) -> Option<String> {
    let obj = args.as_object()?;
    for key in KEY_ARG_PREFERRED {
        if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    obj.values()
        .find_map(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 按**字符数**截断（非字节）：中文等多字节字符不切出半个 UTF-8 序列。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// （workbench-agent-identity-and-process-folds 1.2）工具条目的结构化标题：
/// `done=false` 为调用开始（`{tool} · {key_arg}`），`done=true` 带完成前缀
/// （`✓ {tool} · {key_arg}`）；提取不到关键参数就只有工具名（带/不带 ✓）。
/// 注意：`AcpEvent::ToolEnd` 不携带 args（wire 无 call id 可配对），完成态
/// 调用方传 `None`，标题即 `✓ {tool}`。
pub(crate) fn tool_entry_title(
    done: bool,
    tool_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    let mut title = match args.and_then(key_arg_from_args) {
        Some(arg) => format!("{tool_name} · {arg}"),
        None => tool_name.to_string(),
    };
    if done {
        title.insert_str(0, "✓ ");
    }
    truncate_chars(&title, TITLE_MAX_CHARS)
}

/// Wall-clock seconds since the UNIX epoch. Wrapped so tests can override
/// it; production just reads the OS clock once per call.
#[inline]
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_event_round_trips_through_serde() {
        // 1.1 验收：每个变体经 serde 往返后与原值一致。
        let info = SessionInfo {
            channel: "feishu".into(),
            key: "oc_1\0om_t".into(),
            session_id: Some("s1".into()),
            status: "active".into(),
            phase: Some("DONE".into()),
            user_prompt: Some("hello".into()),
            last_active_unix: 1234,
            project_dir: Some("/tmp/p".into()),
            current_model: Some("m1".into()),
            available_models: Some(vec!["m1".into(), "m2".into()]),
            agent_kind: Some("claude".into()),
            usage: None,
            // wire-webui-sebas-agent-e2e D4：执行体标签随快照/事件往返。
            backend: Some("native".into()),
            // workbench-turn-queue D6：pending 视图随 SessionInfo 往返。
            pending: vec![crate::state::PendingSubmission {
                id: 7,
                text: "queued behind the running turn".into(),
                position: 0,
                disposition: crate::state::PendingDisposition::Turn,
                priority: false,
            }],
            remote: None,
            desired_mode: None,
            effective_mode: None,
            // rail-declutter-unread：msg_count 随 SessionInfo 往返。
            msg_count: 0,
        };
        let cases = vec![
            SessionEvent::Created {
                session: info.clone(),
            },
            SessionEvent::Updated { session: info },
            SessionEvent::Removed {
                channel: "feishu".into(),
                key: "oc_2".into(),
            },
            // workbench-turn-queue 5.2：丢弃标注随事件往返。
            SessionEvent::PendingDropped {
                channel: "web".into(),
                key: "web-1".into(),
                dropped: vec![crate::state::PendingSubmission {
                    id: 3,
                    text: "never ran".into(),
                    position: 0,
                    disposition: crate::state::PendingDisposition::Turn,
                    priority: false,
                }],
            },
            SessionEvent::Resync,
        ];
        for ev in cases {
            let json = serde_json::to_string(&ev).expect("serialize");
            let back: SessionEvent = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, ev, "round-trip mismatch for {json}");
        }
    }

    #[test]
    fn session_event_uses_type_tag() {
        // wire 形态带 "type" tag，与 control RPC 的 cmd tag 姿态一致。
        let ev = SessionEvent::Removed {
            channel: "feishu".into(),
            key: "oc_x".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "removed");
        assert_eq!(json["channel"], "feishu");
        assert_eq!(json["key"], "oc_x");
    }

    #[test]
    fn channel_key_round_trips_through_flattened_info() {
        let k = ChannelKey::feishu("oc_x", Some("t1"));
        let info = SessionInfo {
            channel: k.channel_str().to_string(),
            key: k.reference.clone(),
            session_id: Some("s1".into()),
            status: "active".into(),
            phase: None,
            user_prompt: None,
            last_active_unix: 0,
            project_dir: None,
            current_model: None,
            available_models: None,
            agent_kind: None,
            usage: None,
            backend: None,
            pending: Vec::new(),
            remote: None,
            desired_mode: None,
            effective_mode: None,
            msg_count: 0,
        };
        assert_eq!(info.channel, "feishu");
        assert_eq!(info.key, "oc_x\0t1");
        assert_eq!(info.channel_key(), k);
    }

    #[test]
    fn turn_entry_round_trips_through_serde() {
        let e = TurnEntry::prompt(3, "fix the bug");
        let back: TurnEntry = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    /// workbench-agent-identity-and-process-folds 1.1：旧持久化 JSON 无
    /// `title` 字段仍反序列化（→ None，不报错）；带 title 的条目完整往返；
    /// None 序列化时**省略键**（wire 只在有值时多一个可选键）。
    #[test]
    fn turn_entry_title_field_is_additive() {
        // 旧形状：无 title 字段。
        let legacy = r#"{
            "position": 4,
            "kind": "content",
            "element_type": "tool",
            "content": "📖 **Read**",
            "created_at_unix": 42
        }"#;
        let back: TurnEntry = serde_json::from_str(legacy).unwrap();
        assert_eq!(back.title, None);
        assert_eq!(back.position, 4);
        assert_eq!(back.element_type, "tool");

        // 带 title：完整往返。
        let titled = TurnEntry::tool(0, "📖 **Read**")
            .with_title(tool_entry_title(false, "Read", Some(&serde_json::json!({"file_path": "src/main.rs"}))));
        let json = serde_json::to_string(&titled).unwrap();
        assert_eq!(serde_json::from_str::<TurnEntry>(&json).unwrap(), titled);
        assert!(json.contains(r#""title""#), "{json}");

        // None：键不上 wire。
        let untitled = TurnEntry::tool(1, "✓ **Read**");
        let json = serde_json::to_string(&untitled).unwrap();
        assert!(!json.contains("title"), "{json}");
    }

    /// workbench-agent-identity-and-process-folds 1.2：路径类键命中——
    /// `file_path` 提取为关键参数；同现 `path` 与 `command` 时路径键优先。
    #[test]
    fn tool_title_prefers_path_keys() {
        let args = serde_json::json!({"file_path": "src/main.rs"});
        assert_eq!(
            tool_entry_title(false, "Read", Some(&args)),
            "Read · src/main.rs"
        );
        // 路径组在偏好键序里先于 command/pattern。
        let args = serde_json::json!({"command": "cat f", "path": "src/"});
        assert_eq!(tool_entry_title(false, "Read", Some(&args)), "Read · src/");
    }

    /// workbench-agent-identity-and-process-folds 1.2：无路径键时按偏好键序
    /// 回退到 command / pattern。
    #[test]
    fn tool_title_falls_back_to_command_and_pattern() {
        let args = serde_json::json!({"command": "cargo test -p sebas-dispatch"});
        assert_eq!(
            tool_entry_title(false, "Bash", Some(&args)),
            "Bash · cargo test -p sebas-dispatch"
        );
        let args = serde_json::json!({"pattern": "TurnEntry", "path": ""});
        assert_eq!(
            tool_entry_title(false, "Grep", Some(&args)),
            "Grep · TurnEntry",
            "空字符串的偏好键视为未命中，继续向后找"
        );
    }

    /// workbench-agent-identity-and-process-folds 1.2：没有任何字符串参数
    /// （空对象 / 非字符串值 / 非对象 args / ToolEnd 无 args）时标题退化为
    /// 纯工具名；完成态带 `✓ ` 前缀。其余字符串值兜底（偏好键全未命中时
    /// 取对象里第一个字符串值）。
    #[test]
    fn tool_title_without_string_args_is_tool_name_only() {
        assert_eq!(
            tool_entry_title(false, "Bash", Some(&serde_json::json!({}))),
            "Bash"
        );
        assert_eq!(
            tool_entry_title(false, "Bash", Some(&serde_json::json!({"count": 3}))),
            "Bash"
        );
        // 非对象 args（null / 数组）同样只有工具名。
        assert_eq!(tool_entry_title(false, "Bash", Some(&serde_json::Value::Null)), "Bash");
        assert_eq!(
            tool_entry_title(false, "Bash", Some(&serde_json::json!(["a", "b"]))),
            "Bash"
        );
        // ToolEnd：wire 无 args → 传 None，只有 `✓ {tool}`。
        assert_eq!(tool_entry_title(true, "Bash", None), "✓ Bash");
        assert_eq!(tool_entry_title(false, "Bash", None), "Bash");
        // 偏好键全未命中：第一个字符串值兜底。
        assert_eq!(
            tool_entry_title(false, "Tool", Some(&serde_json::json!({"x": "y", "n": 1}))),
            "Tool · y"
        );
    }

    /// workbench-agent-identity-and-process-folds 1.2：标题整体 200 字符上限，
    /// 按字符截断（多字节中文不切半个、不 panic）。
    #[test]
    fn tool_title_caps_at_200_chars_on_char_boundaries() {
        // 300 个中文字符的 command：截到 200 字符。
        let long = "读".repeat(300);
        let args = serde_json::json!({ "command": long });
        let title = tool_entry_title(false, "Bash", Some(&args));
        assert_eq!(title.chars().count(), 200);
        assert!(title.starts_with("Bash · "));
        // 恰好 200 字符（"Bash · " 占 7 字符 + 193 字符参数）：不截断。
        let exact = "a".repeat(193);
        let title = tool_entry_title(false, "Bash", Some(&serde_json::json!({ "command": exact })));
        assert_eq!(title.chars().count(), 200);
        assert!(title.ends_with(&exact));
    }
}

/// （extract-im-service 2.3）usage 字段 serde 兼容：旧形状（无 usage）仍反
/// 序列化；带 usage 的形状完整往返。
#[test]
fn session_info_usage_field_is_additive() {
    use sebas_channels::card::AppUsage;

    let full = SessionInfo {
        channel: "feishu".into(),
        key: "oc_u".into(),
        session_id: Some("s1".into()),
        status: "active".into(),
        phase: Some("OnIt".into()),
        user_prompt: Some("p".into()),
        last_active_unix: 1,
        project_dir: None,
        current_model: None,
        available_models: None,
        agent_kind: None,
        backend: None,
        pending: Vec::new(),
        remote: None,
        usage: Some(AppUsage {
            model: Some("claude-x".into()),
            total_input: 10,
            total_output: 25,
        }),
        desired_mode: None,
        effective_mode: None,
        msg_count: 3,
    };
    let json = serde_json::to_string(&full).unwrap();
    let back: SessionInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(back, full);

    // 旧形状：无 usage 字段的 JSON 反序列化为 None。
    let legacy = r#"{"channel":"feishu","key":"oc_u","session_id":"s1","status":"active","phase":null,"user_prompt":null,"last_active_unix":1,"project_dir":null,"current_model":null,"available_models":null,"agent_kind":null}"#;
    let back: SessionInfo = serde_json::from_str(legacy).unwrap();
    assert_eq!(back.usage, None);
    // workbench-turn-queue：无 pending 字段同样可读（默认空栈）。
    assert_eq!(back.pending, Vec::new());
    // rail-declutter-unread：无 msg_count 字段的旧报文反序列化为 0。
    assert_eq!(back.msg_count, 0);
}

/// workbench-turn-queue 5.2：PendingDropped 携带被丢弃条目（id + 文本），
/// wire 形状带 snake_case 的 type 标签。
#[test]
fn pending_dropped_event_carries_the_dropped_entries() {
    let ev = SessionEvent::PendingDropped {
        channel: "web".into(),
        key: "web-drop".into(),
        dropped: vec![
            crate::state::PendingSubmission {
                id: 1,
                text: "one".into(),
                position: 0,
                disposition: crate::state::PendingDisposition::Staging,
                priority: false,
            },
            crate::state::PendingSubmission {
                id: 2,
                text: "two".into(),
                position: 1,
                disposition: crate::state::PendingDisposition::Turn,
                priority: true,
            },
        ],
    };
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["type"], "pending_dropped");
    assert_eq!(json["dropped"][1]["text"], "two");
    assert_eq!(json["dropped"][1]["disposition"], "turn");
    assert_eq!(json["dropped"][1]["priority"], true);
    let back: SessionEvent = serde_json::from_value(json).unwrap();
    assert_eq!(back, ev);
}

/// rail-declutter-unread D2（计数口径）：流式 delta 合并成段——一串相邻
/// markdown 计 1，被 tool/thinking 打断后另起一段，error 逐条各计 1。
#[test]
fn chat_message_count_merges_adjacent_markdown_deltas() {
    let log = vec![
        TurnEntry::prompt(0, "do it"),
        // 一段流式回复：3 个 delta = 1 段。
        TurnEntry::markdown(1, "let me "),
        TurnEntry::markdown(2, "check "),
        TurnEntry::markdown(3, "that."),
        // 工具噪声：不计、但打断当前段。
        TurnEntry::tool(4, "📖 **read_file**"),
        TurnEntry::tool(5, "✓ **read_file**"),
        // 工具后的新文本段：+1。
        TurnEntry::markdown(6, "done."),
        // thinking 噪声：不计、打断。
        TurnEntry::thinking(7, "hmm"),
        // 错误条目：逐条计 1。
        TurnEntry::error(8, "**spawn failed**: boom"),
        TurnEntry::error(9, "**spawn failed**: boom again"),
    ];
    // 段 1（三连 delta）+ 工具/thinking 打断后的段 2 + 两条 error 逐条 = 4。
    assert_eq!(count_chat_messages(&log), 4, "{log:?}");
}

/// rail-declutter-unread D2（计数口径）：process noise 不计数——只有
/// thinking / tool / prompt 的会话段数为 0；空条目不计数也不断段。
#[test]
fn chat_message_count_ignores_noise_and_empty_entries() {
    // 纯噪声 = 0。
    let noise = vec![
        TurnEntry::prompt(0, "hello"),
        TurnEntry::thinking(1, "thinking"),
        TurnEntry::tool(2, "📖 **bash**"),
        TurnEntry::tool(3, "✓ **bash**"),
    ];
    assert_eq!(count_chat_messages(&noise), 0);
    // 空内容条目：不计数、不打断相邻 markdown 的连续性（前端同规则）。
    let with_empty = vec![
        TurnEntry::markdown(0, "first"),
        TurnEntry {
            position: 1,
            kind: "content".into(),
            element_type: "markdown".into(),
            content: String::new(),
            created_at_unix: 1,
            title: None,
        },
        TurnEntry::markdown(2, " still same segment"),
    ];
    assert_eq!(count_chat_messages(&with_empty), 1);
    // 空 transcript = 0。
    assert_eq!(count_chat_messages(&[]), 0);
}
