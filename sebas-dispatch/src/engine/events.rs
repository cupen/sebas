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
        }
    }
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
        usage: Some(AppUsage {
            model: Some("claude-x".into()),
            total_input: 10,
            total_output: 25,
        }),
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
