//! Core session channel protocol (openspec/changes/add-core-session-channel,
//! tasks 4.1/4.2): newline-delimited JSON frames over a Unix stream socket.
//!
//! Mirrors `RpcControlRequest`'s serde shape (`cmd` tag on requests, `cmd`
//! tag on responses). Session data types (`SessionInfo`, `SessionEvent`,
//! `TurnEntry`, `SessionRejection`) come from the router/webui crates so the
//! wire types and the trait types can never drift apart.
//!
//! ## Request/response (one connection per mutation)
//!
//! 1. client → server: one line of JSON, `CoreChannelRequest` (with the
//!    shared secret already checked in the handshake line before it).
//! 2. server → client: one line of JSON, `CoreChannelResponse`
//!    (`accepted` or `rejected` with the typed rejection).
//!
//! ## Subscription (one dedicated streaming connection)
//!
//! 1. client sends `CoreChannelRequest::Subscribe`.
//! 2. server → client: `SessionStreamFrame::Snapshot` (the state at subscribe
//!    time), then `SessionStreamFrame::Event` for every session event as it
//!    happens. The snapshot comes before any event (spec: "snapshot then
//!    subscribe" ordering is server-side subscribe-first + snapshot, so a
//!    mutation racing the subscribe is captured by the snapshot — no gap; the
//!    events it also produced are idempotent full-state updates — no visible
//!    duplicate). A lagging subscriber is dropped (connection closed) rather
//!    than delivered a gap; the client re-snapshots on reconnect.

use sebas_channels::ChannelKey;
use sebas_dispatch::{PendingSubmission, SessionEvent, SessionInfo, TurnEntry};
use sebas_webui::session_backend::{PermissionDecision, PermissionNotice};
use serde::{Deserialize, Serialize};

/// （extract-im-service 4.1）随消息投递的本地附件引用：im 进程把媒体解析到
/// 本地（`[media] download_dir`），核心与执行体在同一台机器上直接按路径
/// 读取（不传字节流）。serde 兼容：`#[serde(default)]` 挂在宿主字段上。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Attachment {
    /// 投递给执行体的文本标记（本地路径引用；agent 可用文件读取工具消化）。
    pub fn marker(&self) -> String {
        let mime = self.mime.as_deref().unwrap_or("application/octet-stream");
        let name = self.name.clone().unwrap_or_else(|| {
            std::path::Path::new(&self.path)
                .file_name()
                .map(|n| n.display().to_string())
                .unwrap_or_else(|| self.path.clone())
        });
        format!("[附件: {} ({mime}) 路径 {}]", name, self.path)
    }
}

/// One request over the core session channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CoreChannelRequest {
    /// Full external session snapshot.
    Snapshot,
    /// Create a session (optionally rooted in a project directory).
    Spawn {
        prompt: String,
        project_dir: Option<String>,
        /// （add-acp-model-selection）创建时请求的模型 id（None = 默认模型）。
        #[serde(default)]
        model: Option<String>,
        /// （add-agent-mode-selection）创建时请求的权限模式（控制面词汇
        /// `ask`/`edit`/`allow`/`auto`；None = agent 默认行为）。
        /// `#[serde(default)]`：旧客户端不发这个字段，行为与今日一致。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        /// 目标 agent id（workbench-agent-wire-fix D2）：`[acp.agents.*]`
        /// 配置键名或保留值 `"native"`。driver 名与 `acp:` 前缀不再是合法
        /// 值——agent 是 wire 上唯一的执行体词汇。
        agent: String,
        /// 项目所在的**执行节点**（add-remote-execution-node 5.1/8.2）。
        ///
        /// `None` 与 `Some(LOCAL_NODE_ID)` 都表示主控本机（行为与今日完全一致）；
        /// 别的值表示"在那个节点上建"，由 core 经节点链路建立。`#[serde(default)]`：
        /// 旧客户端不发这个字段，语义就是本机，不需要新协议版本。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<String>,
    },
    /// Create a 0-turn placeholder session WITHOUT spawning an agent child
    /// (P2 fix: an empty prompt must not reach the agent — opencode hangs on
    /// `session/prompt ""`). The requested model/执行体 hint are remembered
    /// on the mapping; the first message spawns with them
    /// （add-composer-agent-binding：占位帧同样携带 backend——composer 建
    /// 0-turn 会话是常态路径，hint 不上线则用户选的 agent 被静默丢弃）。
    CreatePlaceholder {
        project_dir: Option<String>,
        /// （add-acp-model-selection）创建时请求的模型 id（None = 默认模型）。
        #[serde(default)]
        model: Option<String>,
        /// （add-agent-mode-selection）创建时请求的权限模式（占位记住，
        /// 首条消息 spawn 时消费；None = agent 默认行为）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        /// 目标 agent id，语义与 `Spawn.agent` 一致（workbench-agent-wire-fix
        /// D2）。占位帧必须携带——composer 建 0-turn 会话是常态路径，agent
        /// 不上线则用户选的 agent 被静默丢弃。
        agent: String,
        /// 目标执行节点，语义与 `Spawn.node` 一致。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<String>,
    },
    /// 中程切换会话模型（add-acp-model-selection）：`session/set_config_option`。
    SetSessionModel { key: ChannelKey, model_id: String },
    /// （add-agent-mode-selection）中程切换会话权限模式：本机走
    /// `AcpCommand::SetMode`（claude 驱动运行时切换），远端走节点链路
    /// `SessionOp::SetMode`。执行体接受与否经事件流/节点回报反馈。
    SetSessionMode { key: ChannelKey, mode: String },
    /// Send a message to an existing session. Attachments（extract-im-service
    /// 4.1）随 serde default 增列：旧报文缺省空附件，行为不变。服务端校验
    /// 每个附件路径存在后，以本地路径引用随文本投递（同机部署路径共享）。
    Message {
        key: ChannelKey,
        message: String,
        #[serde(default)]
        attachments: Vec<Attachment>,
    },
    /// （extract-im-service 2.1）IM 前端 ensure 语义的消息投递：未知 key 按
    /// 入站文本历史语义自动建会话、dormant 会话懒复活；已知 active key 等
    /// 价 `Message`。与 `Message` 的差别仅在服务端跳过存在性预检——webui
    /// 的「未知即拒绝」语义不受影响。
    EnsureMessage {
        key: ChannelKey,
        message: String,
        #[serde(default)]
        attachments: Vec<Attachment>,
    },
    /// （extract-im-service 2.2）取消该会话在飞 turn；会话保留、可继续对话。
    /// 会话未知 → typed rejection。
    Cancel { key: ChannelKey },
    /// Close (kill) a session.
    Close { key: ChannelKey },
    /// （workbench-turn-queue 4.2/D7）按 id 移除一个未开始的待生效提交。
    RemovePending { key: ChannelKey, pending_id: u64 },
    /// （workbench-turn-queue 4.2/D7）把一个未开始的提交重排到其处置组内
    /// `to_index` 位置。
    MovePending {
        key: ChannelKey,
        pending_id: u64,
        to_index: usize,
    },
    /// Fetch rendered transcript content at/after a monotonic position.
    Turns { key: ChannelKey, from: u64 },
    /// Mark the focused session.
    SetFocus { key: Option<ChannelKey> },
    /// Ask for the focused session.
    Focused,
    /// Start the event stream (see module docs for the frame order).
    Subscribe,
    /// Snapshot a domain of the core state store (add-state-store).
    StateSnapshot { domain: String },
    /// Mutate a domain of the core state store (add-state-store).
    StateMutation {
        domain: String,
        payload: serde_json::Value,
    },
    /// （add-fetch-models）providers 域抓取 op（D5：provider 域上的只读动作，
    /// 不新增存储域）：按 provider 名解析 base url 与密钥，core 侧执行一次
    /// 只读 GET 上游 `/models`，返回 id 列表。不写任何状态；失败回 typed
    /// rejection（无可用 base url / 上游错误，均净化）。
    FetchModels { provider: String },
    /// Subscribe to state change notifications (add-state-store).
    StateSubscribe,
    /// （wire-webui-sebas-agent-e2e）回填一个审批决定：request_id 来自订阅流
    /// 上的 `ApprovalRequested` 帧。无待决请求 → typed rejection。
    ApprovalAnswer {
        request_id: String,
        decision: PermissionDecision,
    },
    /// 节点链路管理（add-remote-execution-node 2.7）：签发配对 token / 列出节点 /
    /// 吊销节点。**必须经 core**——只有 core 进程持有注册表的写者句柄，另开一个
    /// 进程写同一个文件会与前者的内存副本互相覆盖。
    NodeLink { op: NodeLinkOp },
    /// 请**节点自己**判定一个路径（add-remote-execution-node 8.1）。
    ///
    /// 必须经 core：只有 core 持有到节点的链路。工作台不能自己 stat 一个远端
    /// 路径——那台机器上有没有这个目录，只有那台机器知道。
    NodePathCheck {
        /// 目标节点。
        node_id: String,
        /// 待判定的路径（节点上的绝对路径）。
        path: String,
    },
}

/// 节点链路管理操作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NodeLinkOp {
    /// 签发一个一次性配对 token（`ttl_secs` 缺省 900）。
    IssueJoinToken {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ttl_secs: Option<u64>,
    },
    /// 列出已注册节点。
    ListNodes,
    /// 吊销一个节点的凭据（此后不可接入，也不能靠重新配对绕过）。
    RevokeNode { node_id: String },
}

/// 节点链路管理结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum NodeLinkOutcome {
    /// 已签发：原始 token **只出现这一次**。
    JoinToken { token: String, expires_unix: i64 },
    /// 节点列表。
    Nodes { nodes: Vec<NodeView> },
    /// 吊销结果：`found` 为假表示没有这个节点（如实回报，不假装成功）。
    Revoked { node_id: String, found: bool },
    /// 节点链路未启用（`[node_link] enabled = false`）。
    Disabled { cause: String },
    /// 操作失败（落盘/读表等），带成因。
    Failed { cause: String },
}

/// 节点在管理面上的视图（不含凭据哈希等敏感字段）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeView {
    /// 稳定节点标识。
    pub id: String,
    /// `online` / `offline` / `revoked`。
    pub status: String,
    /// 最后一次成功握手时间（unix 秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_unix: Option<i64>,
    /// 首次配对时间（unix 秒）。
    pub created_unix: i64,
}

/// One response over the core session channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CoreChannelResponse {
    /// Snapshot result.
    Snapshot { sessions: Vec<SessionInfo> },
    /// Spawn result: the new session key.
    Spawned { key: ChannelKey },
    /// Message/close/focus accepted; nothing to return.
    Ok,
    /// （workbench-turn-queue 5.2）close 的结果：`discarded_pending` = 随之
    /// 丢弃的未执行待生效提交条数。
    Closed { discarded_pending: usize },
    /// （workbench-turn-queue 4.2）pending 管理操作成功：返回操作后的全量
    /// pending 视图（客户端据此对账，design D8）。
    PendingList { pending: Vec<PendingSubmission> },
    /// Turn-content result.
    Turns { entries: Vec<TurnEntry> },
    /// Focused-session result.
    Focused { key: Option<ChannelKey> },
    /// Typed rejection — names the reason; nothing was mutated.
    Rejected {
        #[serde(flatten)]
        rejection: sebas_webui::session_backend::SessionRejection,
    },
    /// State snapshot result (add-state-store).
    StateSnapshot {
        domain: String,
        payload: serde_json::Value,
    },
    /// State mutation accepted.
    StateMutationOk,
    /// （add-fetch-models）抓取结果：上游 model id 列表（只读呈现，无落盘）。
    Models {
        provider: String,
        models: Vec<String>,
    },
    /// 节点链路管理结果（add-remote-execution-node 2.7）。
    NodeLink(NodeLinkOutcome),
    /// 节点侧路径判定结果（add-remote-execution-node 8.1）。
    NodePath { exists: bool, is_dir: bool },
}

/// One frame of the subscription stream (task 4.2): exactly one snapshot
/// frame first, then event frames — interleaved with approval frames when a
/// native-kernel session gates a tool call (wire-webui-sebas-agent-e2e).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "frame", rename_all = "snake_case")]
pub enum SessionStreamFrame {
    Snapshot {
        sessions: Vec<SessionInfo>,
    },
    Event {
        event: SessionEvent,
    },
    /// A gated tool call awaits an operator decision; answer via
    /// [`CoreChannelRequest::ApprovalAnswer`]. Not replayed on reconnect —
    /// a request with no reachable client fails closed at the kernel.
    ApprovalRequested {
        notice: PermissionNotice,
    },
}

/// One frame of the **state** subscription stream (add-state-store 4.2).
/// Exactly one full snapshot frame first (all domains), then one `Changed`
/// frame per merged change batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "frame", rename_all = "snake_case")]
pub enum StateStreamFrame {
    /// 全域快照（providers / settings / projects / sessions）。
    Snapshot { domains: serde_json::Value },
    /// 某域发生变更（一串提交可合并为一帧）。
    Changed { scope: String },
}

/// 4.2 验收：state 订阅流（快照帧 + 变更帧）serde 往返后与原值一致。
#[test]
fn state_stream_frame_round_trips() {
    let frames = vec![
        StateStreamFrame::Snapshot {
            domains: serde_json::json!({
                "providers": {},
                "settings": null,
                "projects": [],
                "sessions": []
            }),
        },
        StateStreamFrame::Changed {
            scope: "providers".into(),
        },
        StateStreamFrame::Changed {
            scope: "settings".into(),
        },
    ];
    for f in &frames {
        let json = serde_json::to_string(f).unwrap();
        let back: StateStreamFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, f, "round-trip mismatch for {json}");
    }
    // Wire shape carries the "frame" tag.
    assert_eq!(
        serde_json::to_value(&frames[0]).unwrap()["frame"],
        "snapshot"
    );
    assert_eq!(
        serde_json::to_value(&frames[1]).unwrap()["frame"],
        "changed"
    );
    assert_eq!(
        serde_json::to_value(&frames[1]).unwrap()["scope"],
        "providers"
    );
}

/// 4.2：StateChange wire 形状带 `cmd` tag（与通道请求同风格）。
#[test]
fn state_change_wire_shape() {
    use sebas_dispatch::state_store::StateChange;
    let changed = StateChange::Changed {
        scope: "projects".into(),
    };
    let json = serde_json::to_value(&changed).unwrap();
    assert_eq!(json["cmd"], "changed");
    assert_eq!(json["scope"], "projects");
    let back: StateChange = serde_json::from_value(json).unwrap();
    assert_eq!(back, changed);
}

/// The handshake line sent by the client immediately after connecting,
/// before any request. Wrong/absent secret → the server closes the
/// connection without reading a request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelHandshake {
    pub secret: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_webui::session_backend::SessionRejection;

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: &T) {
        let json = serde_json::to_string(v).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, v, "round-trip mismatch for {json}");
    }

    /// 4.1 验收：每个请求/响应变体经 serde 往返后与原值一致。
    #[test]
    fn every_request_and_response_variant_round_trips() {
        let key = ChannelKey::feishu("oc_1", Some("om_t"));
        let requests = vec![
            CoreChannelRequest::Snapshot,
            CoreChannelRequest::Spawn {
                prompt: "hello".into(),
                project_dir: Some("/tmp/p".into()),
                model: Some("m1".into()),
                mode: Some("allow".into()),
                agent: "claudecode".into(),
                node: None,
            },
            CoreChannelRequest::CreatePlaceholder {
                project_dir: Some("/tmp/p".into()),
                model: Some("m1".into()),
                mode: None,
                agent: "claudecode".into(),
                node: None,
            },
            CoreChannelRequest::SetSessionModel {
                key: key.clone(),
                model_id: "m2".into(),
            },
            CoreChannelRequest::Message {
                key: key.clone(),
                message: "msg".into(),
                attachments: vec![Attachment {
                    path: "/tmp/img.png".into(),
                    mime: Some("image/png".into()),
                    name: Some("img.png".into()),
                }],
            },
            CoreChannelRequest::EnsureMessage {
                key: key.clone(),
                message: "hello from im".into(),
                attachments: vec![],
            },
            CoreChannelRequest::Cancel { key: key.clone() },
            CoreChannelRequest::Close { key: key.clone() },
            CoreChannelRequest::RemovePending {
                key: key.clone(),
                pending_id: 7,
            },
            CoreChannelRequest::MovePending {
                key: key.clone(),
                pending_id: 7,
                to_index: 1,
            },
            CoreChannelRequest::Turns {
                key: key.clone(),
                from: 3,
            },
            CoreChannelRequest::SetFocus {
                key: Some(key.clone()),
            },
            CoreChannelRequest::SetFocus { key: None },
            CoreChannelRequest::Focused,
            CoreChannelRequest::Subscribe,
            CoreChannelRequest::StateSnapshot {
                domain: "providers".into(),
            },
            CoreChannelRequest::StateMutation {
                domain: "settings".into(),
                payload: serde_json::json!({"key": "card_config", "value": {}}),
            },
            CoreChannelRequest::FetchModels {
                provider: "deepseek".into(),
            },
            CoreChannelRequest::StateSubscribe,
            CoreChannelRequest::ApprovalAnswer {
                request_id: "toolu_1".into(),
                decision: PermissionDecision::AllowOnce,
            },
        ];
        for r in &requests {
            roundtrip(r);
        }

        let responses = vec![
            CoreChannelResponse::Snapshot { sessions: vec![] },
            CoreChannelResponse::Spawned { key: key.clone() },
            CoreChannelResponse::Ok,
            CoreChannelResponse::Closed {
                discarded_pending: 2,
            },
            CoreChannelResponse::PendingList {
                pending: vec![PendingSubmission {
                    id: 7,
                    text: "queued".into(),
                    position: 0,
                    disposition: sebas_dispatch::PendingDisposition::Turn,
                    priority: true,
                }],
            },
            CoreChannelResponse::Turns {
                entries: vec![TurnEntry::prompt(0, "p"), TurnEntry::markdown(1, "m")],
            },
            CoreChannelResponse::Focused {
                key: Some(key.clone()),
            },
            CoreChannelResponse::Rejected {
                rejection: SessionRejection::UnknownSession { key: "k".into() },
            },
            CoreChannelResponse::Rejected {
                rejection: SessionRejection::UnusableProjectDir,
            },
            CoreChannelResponse::Rejected {
                rejection: SessionRejection::Capacity { limit: 8 },
            },
            CoreChannelResponse::Rejected {
                rejection: SessionRejection::Unavailable { cause: "c".into() },
            },
            CoreChannelResponse::StateSnapshot {
                domain: "providers".into(),
                payload: serde_json::json!({"providers": {}}),
            },
            CoreChannelResponse::StateMutationOk,
            CoreChannelResponse::Models {
                provider: "p".into(),
                models: vec!["m1".into()],
            },
        ];
        for r in &responses {
            roundtrip(r);
        }

        roundtrip(&ChannelHandshake {
            secret: "s3cret".into(),
        });
    }

    /// 4.2 验收：一个 snapshot 帧后跟 event 帧，按原序解析回同一序列。
    #[test]
    fn stream_frame_parses_back_in_order() {
        let info = SessionInfo {
            channel: "feishu".into(),
            key: "oc_1".into(),
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
            backend: Some("native".into()),
            pending: Vec::new(),
            remote: None,
            desired_mode: None,
            effective_mode: None,
            msg_count: 0,
        };
        let frames = vec![
            SessionStreamFrame::Snapshot {
                sessions: vec![info.clone()],
            },
            SessionStreamFrame::Event {
                event: SessionEvent::Updated {
                    session: info.clone(),
                },
            },
            SessionStreamFrame::ApprovalRequested {
                notice: PermissionNotice {
                    request_id: "toolu_9".into(),
                    session_id: "feishu%3Aagent-1".into(),
                    tool_name: "bash".into(),
                    args: serde_json::json!({"command": "ls"}),
                    reason: "policy ask".into(),
                },
            },
            SessionStreamFrame::Event {
                event: SessionEvent::Removed {
                    channel: "feishu".into(),
                    key: "oc_1".into(),
                },
            },
            SessionStreamFrame::Event {
                event: SessionEvent::Resync,
            },
        ];
        let mut parsed = Vec::new();
        for f in &frames {
            let json = serde_json::to_string(f).unwrap();
            parsed.push(serde_json::from_str::<SessionStreamFrame>(&json).unwrap());
        }
        assert_eq!(parsed, frames);
        // The wire shape carries the "frame" tag.
        assert_eq!(
            serde_json::to_value(&frames[0]).unwrap()["frame"],
            "snapshot"
        );
        assert_eq!(serde_json::to_value(&frames[3]).unwrap()["frame"], "event");
    }

    /// wire-webui-sebas-agent-e2e 1.1 验收：旧客户端报文（Spawn 无 backend 字段）
    /// 仍可反序列化 —— additive 兼容。
    #[test]
    fn legacy_wire_shapes_still_deserialize() {
        let legacy_spawn = r#"{"cmd":"spawn","prompt":"hi","project_dir":null,"model":null}"#;
        // 旧帧（backend 词汇）不再可读——wire 收紧是本 change 的 BREAKING 面
        // （workbench-agent-wire-fix D2）：legacy Spawn 反序列化必须失败。
        let req: Result<CoreChannelRequest, _> = serde_json::from_str(legacy_spawn);
        assert!(req.is_err(), "legacy backend frame must be rejected");
        // 快照条目的旧格式（无 backend/current_model 字段）同样可反序列化。
        let legacy_info = r#"{"channel":"feishu","key":"oc_1","session_id":null,"status":"active","last_active_unix":0}"#;
        let info: SessionInfo = serde_json::from_str(legacy_info).unwrap();
        assert_eq!(info.current_model, None);
        assert_eq!(info.backend, None);
        assert_eq!(info.agent_kind, None);
    }
}
