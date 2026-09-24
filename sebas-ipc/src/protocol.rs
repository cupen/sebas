//! 跨进程协议之家（unify-ipc-protocol-home）：各边界的 wire 类型与
//! 握手 / framing 助手。
//!
//! 这里放的是**形状**，不是行为：core session channel 的请求 / 响应 / 订阅帧、
//! 节点链路管理 op 与结果、握手（含版本协商）以及 NDJSON 分帧助手。各角色
//! （根 crate、webui、im、router、node）经普通 path 依赖取用同一份定义——
//! 「同一协议被声明两遍」在结构上不再可能。
//!
//! # 准入面（crate 文档「准入清单」的协议侧）
//!
//! 允许进来：**跨进程边界的 wire 类型**（含其 serde 形状与固有助手）、
//! **握手**（secret + 版本协商）、**framing 助手**（NDJSON 一行一帧）。
//!
//! 不允许进来：任何**角色实现**的关注点（core / webui / router / im / node
//! 的运行时类型、trait、句柄），以及任何**域表名**（持久层概念）。本 crate
//! 只依赖中立叶子（`sebas-domain` / `sebas-channels`），机械断言见根 crate 的
//! `tests/ipc_protocol_home_test.rs`。
//!
//! # 前向兼容（spec「Forward compatibility rules are explicit and enforced」）
//!
//! - 新增字段**必须**带 serde 默认值（旧对端不发也能读）；
//! - 取 wire 值的枚举**必须**保留未知值路径（`#[serde(other)] Unknown`），
//!   对端发来本 build 不认识的取值时**不报错、不丢帧**；
//! - 删字段 / 改字段名 / 改枚举取值 = **破坏性变更**，必须在 change 里显式
//!   声明并同步更新 golden fixture。
//!
//! # 编码与 framing 是兼容面
//!
//! NDJSON（一行一个 JSON 对象，`\n` 结尾）是 core session channel 的既有
//! framing，**不得**顺手改动：改它是一次显式声明的破坏性变更。助手
//! [`encode_line`] / [`decode_line`] 是这份 framing 的唯一实现。

use sebas_channels::ChannelKey;
use sebas_domain::session::{
    PendingApproval, PendingSubmission, PermissionDecision, PermissionNotice, SessionEvent,
    SessionIdentity, SessionInfo, SessionRejection, TurnEntry, TurnStreamEvent,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// framing 助手
// ---------------------------------------------------------------------------

/// 把一帧序列化为一行 NDJSON（含结尾 `\n`）——core session channel 的既有
/// framing 的唯一实现。
pub fn encode_line<T: Serialize>(frame: &T) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(frame)?;
    line.push('\n');
    Ok(line)
}

/// 解析一行 NDJSON 帧（容忍行尾空白 / `\r\n`）。
pub fn decode_line<T: DeserializeOwned>(line: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line.trim())
}

/// 线帧的固有读写（每个 wire 类型都免费拿到 `to_line` / `from_line`）。
///
/// 存在的理由：调用点不该各自拼 `serde_json::to_string` + `push('\n')`——
/// framing 是**兼容面**，只允许有一份实现（就是上面两个函数）。
pub trait WireFrame: Serialize + DeserializeOwned + Sized {
    /// 序列化为一行 NDJSON（含结尾 `\n`）。
    fn to_line(&self) -> Result<String, serde_json::Error> {
        encode_line(self)
    }

    /// 从一行 NDJSON 解析。
    fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        decode_line(line)
    }
}

impl<T: Serialize + DeserializeOwned + Sized> WireFrame for T {}

// ---------------------------------------------------------------------------
// 会话通道载荷
// ---------------------------------------------------------------------------

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
    /// 聚焦即拉起（workbench-live-conversation-flow 3.1）：无 prompt 拉起
    /// 会话子进程（占位 fresh / Dormant resume），幂等（已活/在途 → started
    /// = false）。
    Activate { key: ChannelKey },
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
    /// 从归档条目重建会话（fix-webui-qa-defects 2.2，design D1）：core 侧
    /// `web_restore_session`——原 key 重建 Dormant 映射 + 转写回放。detached
    /// webui 的 restore handler 先调这里、成功后才消费本地归档条目。
    RestoreSession {
        key: ChannelKey,
        /// 归档时刻的原路由 id；`None` = 旧条目（引擎按 key 合成）。
        #[serde(default)]
        session_id: Option<String>,
        /// 原项目路径；`None`/空 = 无项目会话。
        #[serde(default)]
        project_dir: Option<String>,
        /// 归档的对话快照。
        #[serde(default)]
        transcript: Vec<TurnEntry>,
        /// （fix-webui-approval-restore-and-session-identity 3.2，design D3）
        /// 归档条目携带的会话身份，恢复时原样带回引擎。`#[serde(default)]`：
        /// 旧客户端不发这个字段 = 全空身份（恢复维持现默认）。
        #[serde(default)]
        identity: SessionIdentity,
        /// （fix-webui-qa-defects-round4 3.1）归档时刻的操作者 label 与首条
        /// prompt 预览——命名来源随快照迁回。`#[serde(default)]`：旧客户端
        /// 不发 = `None`（回退现状短 id）。
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        prompt_preview: Option<String>,
    },
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
    /// 待批审批读模型（fix-webui-approval-restore-and-session-identity 1.2）：
    /// detached webui 打开/刷新会话时向 core 拉取当前泊车审批。
    PendingApprovals { key: ChannelKey },
    /// 设置/清空会话 label（fix-webui-approval-restore-and-session-identity
    /// 5.1，design D6）；`None` = 清空。
    SetSessionLabel { key: ChannelKey, label: Option<String> },
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
    /// 对端发来本 build 不认识的 `cmd`（对端比本端新）。
    ///
    /// `#[serde(other)]`：容忍未知命令而不是让整帧解析失败——连接不关、
    /// 帧不丢，服务端回一条类型化拒绝（spec「An unknown enum value does not
    /// fail the message」）。语义上**永不执行**任何动作（fail closed）。
    #[serde(other)]
    Unknown,
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
    /// 对端发来本 build 不认识的 `op`：如实拒绝，绝不当作已知操作执行。
    #[serde(other)]
    Unknown,
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
    /// 对端发来本 build 不认识的 `result`：按「操作失败」如实呈现，不假装成功。
    #[serde(other)]
    Unknown,
}

// 节点管理面视图已与 webui 的 `NodeInfo` 合一为
// `sebas_domain::node::NodeView`（add-domain-layer 4.1）；core 通道上的
// wire 形状逐字节不变（`local: false` 不上 wire）。
pub use sebas_domain::node::NodeView;

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
    /// 待批审批读模型应答（fix-webui-approval-restore-and-session-identity
    /// 1.2）：会话当前泊车的审批全量（空表 = 无泊车）。
    PendingApprovals { requests: Vec<PendingApproval> },
    /// Activate 的应答：started = 本次调用真正触发了拉起（false = 已活着
    /// 或已在途，幂等无操作）。
    Activated { started: bool },
    /// Typed rejection — names the reason; nothing was mutated.
    Rejected {
        #[serde(flatten)]
        rejection: SessionRejection,
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
    /// 节点侧路径判定结果（add-remote-execution-node 8.1）。`within_workspace`
    /// （add-workspace-root）是节点以它自己的 workspace root 做的 containment
    /// 判定；serde 缺省 `true` 与节点链路同姿态——旧 core 的应答缺字段视为界内。
    NodePath {
        exists: bool,
        is_dir: bool,
        #[serde(default = "default_true")]
        within_workspace: bool,
    },
    /// 对端发来本 build 不认识的 `cmd`（对端比本端新）：按「操作不可用」如实
    /// 呈现，不假装成功。
    #[serde(other)]
    Unknown,
}

/// `NodePath::within_workspace` 的 serde 缺省：缺字段 = 界内（兼容旧应答）。
fn default_true() -> bool {
    true
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
    /// 实时回合内容帧（workbench-live-conversation-flow 1.1）：同一合并窗
    /// 内某会话追加的 transcript 条目。日志仍是唯一事实——本帧是增量补充，
    /// 乱序/迟到/丢失由消费端以快照重取收敛，不参与重连重放。
    Turn {
        event: TurnStreamEvent,
    },
    /// （fix-webui-streaming-liveness 5.2，D6）重新同步信号：core 侧检测到
    /// 该订阅的 turn 流落后（合并器 Lagged，增量已有不可弥补缺口）时发出。
    /// 客户端收到后按 `SessionEvent::Resync` 语义重取受影响会话的快照；
    /// 连接保持——断连重连只是最后手段，不再是丢帧的常规收敛路径。
    Resync,
    /// 对端发来本 build 不认识的 `frame`（对端比本端新）：**忽略这一帧**，
    /// 连接保持、后续帧照收（spec「no frame is dropped」的反面——未知帧被
    /// 明确跳过并记日志，而不是整条流断掉）。
    #[serde(other)]
    Unknown,
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
    /// 对端发来本 build 不认识的 `frame`：忽略这一帧，连接保持（订阅者据此
    /// 不至于因为 core 多了一种帧类型而永久断连）。
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------------
// 握手与版本协商
// ---------------------------------------------------------------------------

/// 本 build 实现的 core session channel 协议版本。
///
/// 与 `sebas_node_link::PROTOCOL_VERSION` 同款演进机制：不兼容的改动提升它，
/// 对端报出更高版本时**如实拒绝并指名双方版本**，而不是让请求被静默误读。
pub const PROTOCOL_VERSION: u32 = 1;

/// [`ChannelHandshake::version`] 的 serde 缺省：**不发版本 = 版本 1**（旧
/// 客户端），使版本字段成为纯 additive 演进（D7）。
pub fn default_protocol_version() -> u32 {
    PROTOCOL_VERSION
}

/// The handshake line sent by the client immediately after connecting,
/// before any request. Wrong/absent secret → the server closes the
/// connection without reading a request.
///
/// `version` 是 additive 字段（`#[serde(default)]`）：旧客户端不发它，
/// 服务端按版本 1 服务；旧服务端读到新客户端的版本字段只是忽略未知键。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelHandshake {
    pub secret: String,
    #[serde(default = "default_protocol_version")]
    pub version: u32,
}

impl Default for ChannelHandshake {
    fn default() -> Self {
        Self {
            secret: String::new(),
            version: PROTOCOL_VERSION,
        }
    }
}

impl ChannelHandshake {
    /// 客户端握手（secret + 本端版本）。
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            version: PROTOCOL_VERSION,
        }
    }

    /// 服务端协商：客户端版本被支持则返回本端版本，否则返回**类型化拒绝**
    /// （指名客户端版本与本端支持版本）。
    pub fn negotiate(&self) -> Result<u32, ChannelHandshakeAck> {
        if self.version <= PROTOCOL_VERSION {
            Ok(PROTOCOL_VERSION)
        } else {
            Err(ChannelHandshakeAck::VersionUnsupported {
                version: PROTOCOL_VERSION,
                client_version: self.version,
            })
        }
    }

    /// 序列化为一行 NDJSON。
    pub fn to_line(&self) -> Result<String, serde_json::Error> {
        encode_line(self)
    }

    /// 从一行 NDJSON 解析。
    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        decode_line(line)
    }
}

/// 服务端对握手的应答：`ok`（携带本端版本）或**类型化版本拒绝**。
///
/// 旧客户端把 `{"handshake":"ok","version":1}` 读成只有 `handshake` 键的
/// 结构体（未知键忽略），行为不变；新客户端读到旧服务端的 `{"handshake":"ok"}`
/// 时 `version` 靠 serde 默认值落为 1。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "handshake", rename_all = "snake_case")]
pub enum ChannelHandshakeAck {
    /// 握手通过：`version` 是服务端实现的协议版本。
    Ok {
        #[serde(default = "default_protocol_version")]
        version: u32,
    },
    /// 版本不受支持：`version` = 本端支持版本，`client_version` = 对端声明版本。
    VersionUnsupported { version: u32, client_version: u32 },
    /// 对端发来本 build 不认识的握手结果：一律当**拒绝**处理（fail closed，
    /// 绝不把看不懂的应答当作握手成功）。
    #[serde(other)]
    Unknown,
}

impl ChannelHandshakeAck {
    /// 握手通过的应答（携带本端版本）。
    pub fn ok() -> Self {
        Self::Ok {
            version: PROTOCOL_VERSION,
        }
    }

    /// 是否握手通过。
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }

    /// 本端 / 对端声明的协议版本。
    pub fn version(&self) -> u32 {
        match self {
            Self::Ok { version } => *version,
            Self::VersionUnsupported { version, .. } => *version,
            Self::Unknown => PROTOCOL_VERSION,
        }
    }

    /// 类型化拒绝的可读文本：**指名客户端版本与本端支持版本**；握手通过
    /// 与未知应答返回 `None` / 通用文案。
    pub fn cause(&self) -> Option<String> {
        match self {
            Self::Ok { .. } => None,
            Self::VersionUnsupported {
                version,
                client_version,
            } => Some(format!(
                "protocol version unsupported: client={client_version} supported={version}"
            )),
            Self::Unknown => Some("unknown handshake response".to_string()),
        }
    }

    /// 序列化为一行 NDJSON。
    pub fn to_line(&self) -> Result<String, serde_json::Error> {
        encode_line(self)
    }

    /// 从一行 NDJSON 解析。
    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        decode_line(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
        let json = serde_json::to_string(v).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, v, "round-trip mismatch for {json}");
    }

    // ---- 3.1：版本字段带默认值，序列化含版本字段 ----

    /// 缺省（旧客户端报文）按版本 1 处理，且能被协商接受。
    #[test]
    fn handshake_without_version_is_treated_as_version_one() {
        let legacy = r#"{"secret":"s3cret"}"#;
        let hs = ChannelHandshake::from_line(legacy).expect("legacy handshake must decode");
        assert_eq!(hs.version, 1, "缺版本必须按版本 1 处理");
        assert_eq!(hs, ChannelHandshake::new("s3cret"));
        assert_eq!(hs.negotiate(), Ok(PROTOCOL_VERSION));
    }

    /// 本端握手**总是**带上版本字段（additive wire 变化，7.1 点名）。
    #[test]
    fn handshake_serializes_the_version_field() {
        let hs = ChannelHandshake::new("s3cret");
        let line = hs.to_line().unwrap();
        assert_eq!(line, "{\"secret\":\"s3cret\",\"version\":1}\n");
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["version"], 1);
        roundtrip(&hs);
    }

    /// 服务端应答携带本端版本，且往返一致。
    #[test]
    fn handshake_ack_carries_the_local_version() {
        let ack = ChannelHandshakeAck::ok();
        assert_eq!(ack.to_line().unwrap(), "{\"handshake\":\"ok\",\"version\":1}\n");
        assert!(ack.is_ok());
        roundtrip(&ack);
    }

    // ---- 3.2：版本不匹配 → 类型化拒绝，指名双方版本 ----

    #[test]
    fn future_version_is_rejected_naming_both_versions() {
        let hs = ChannelHandshake {
            secret: "s3cret".into(),
            version: 9,
        };
        let ack = hs.negotiate().expect_err("version 9 must be rejected");
        assert_eq!(
            ack,
            ChannelHandshakeAck::VersionUnsupported {
                version: PROTOCOL_VERSION,
                client_version: 9,
            }
        );
        assert!(!ack.is_ok());
        let cause = ack.cause().expect("rejection must carry a cause");
        assert!(cause.contains("client=9"), "拒绝文本须指名客户端版本: {cause}");
        assert!(
            cause.contains(&format!("supported={PROTOCOL_VERSION}")),
            "拒绝文本须指名本端支持版本: {cause}"
        );
        // 线上原文同时携带两个版本号（可机读，不只靠文案）。
        let line = ack.to_line().unwrap();
        assert_eq!(
            line,
            "{\"handshake\":\"version_unsupported\",\"version\":1,\"client_version\":9}\n"
        );
        roundtrip(&ack);
    }

    // ---- 3.4②：旧服务端 × 带版本字段的新客户端（靠默认值解码成功） ----

    #[test]
    fn old_server_ack_without_version_decodes_with_default() {
        // 旧服务端的原文：没有 version 键。
        let ack = ChannelHandshakeAck::from_line("{\"handshake\":\"ok\"}").unwrap();
        assert_eq!(ack, ChannelHandshakeAck::Ok { version: 1 });
        assert!(ack.is_ok(), "旧服务端应答必须被新客户端认成握手成功");
    }

    // ---- 未知握手结果一律当拒绝（fail closed） ----

    #[test]
    fn unknown_ack_is_not_a_success() {
        let ack = ChannelHandshakeAck::from_line("{\"handshake\":\"hibernate\"}").unwrap();
        assert_eq!(ack, ChannelHandshakeAck::Unknown);
        assert!(!ack.is_ok(), "看不懂的握手应答绝不能当成功");
        assert!(ack.cause().is_some());
    }

    // ---- 未知命令 / 帧 / 操作不失败解码（6.1） ----

    #[test]
    fn unknown_wire_values_decode_instead_of_failing() {
        let req: CoreChannelRequest =
            serde_json::from_str(r#"{"cmd":"teleport","payload":1}"#).unwrap();
        assert_eq!(req, CoreChannelRequest::Unknown);
        let resp: CoreChannelResponse = serde_json::from_str(r#"{"cmd":"hologram"}"#).unwrap();
        assert_eq!(resp, CoreChannelResponse::Unknown);
        let frame: SessionStreamFrame = serde_json::from_str(r#"{"frame":"hologram"}"#).unwrap();
        assert_eq!(frame, SessionStreamFrame::Unknown);
        let frame: StateStreamFrame = serde_json::from_str(r#"{"frame":"hologram"}"#).unwrap();
        assert_eq!(frame, StateStreamFrame::Unknown);
        let op: NodeLinkOp = serde_json::from_str(r#"{"op":"hologram"}"#).unwrap();
        assert_eq!(op, NodeLinkOp::Unknown);
        let outcome: NodeLinkOutcome = serde_json::from_str(r#"{"result":"hologram"}"#).unwrap();
        assert_eq!(outcome, NodeLinkOutcome::Unknown);
        // 未知取值的往返保真（原样回吐标签，不归一成别的拼写）。
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"cmd":"unknown"}"#,
            "未知命令回吐本端拼写（不假装认识对端的取值）"
        );
    }

    // ---- framing 助手 ----

    #[test]
    fn framing_helpers_are_ndjson_one_line_per_frame() {
        let line = encode_line(&CoreChannelRequest::StateSubscribe).unwrap();
        assert_eq!(line, "{\"cmd\":\"state_subscribe\"}\n");
        let back: CoreChannelRequest = decode_line(&line).unwrap();
        assert_eq!(back, CoreChannelRequest::StateSubscribe);
        // 容忍 `\r\n`（Windows 文本模式回显）与行尾空白。
        let back: CoreChannelRequest = decode_line("{\"cmd\":\"state_subscribe\"}\r\n").unwrap();
        assert_eq!(back, CoreChannelRequest::StateSubscribe);
    }

    /// Attachment 的既有助手不变（搬迁不改行为）。
    #[test]
    fn attachment_marker_is_unchanged() {
        let a = Attachment {
            path: "/tmp/img.png".into(),
            mime: Some("image/png".into()),
            name: Some("img.png".into()),
        };
        assert_eq!(a.marker(), "[附件: img.png (image/png) 路径 /tmp/img.png]");
    }
}
