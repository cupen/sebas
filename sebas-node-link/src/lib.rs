//! 主控 ↔ 执行节点的**链路契约**（add-remote-execution-node D0）。
//!
//! 本 crate 只放**两侧都要懂的类型**：协议版本、握手、能力清单、拒绝码。运行时
//! （主控侧的监听与注册表、节点侧的拨号与重试）各自实现，双方都依赖本 crate，
//! 因此协议定义不会被复制粘贴成两份。
//!
//! ## 演进规则（协议是长寿命契约，与内部类型不同）
//!
//! - 每个字段都按**可选 + 缺省**设计：新增字段必须能由不认识它的旧版本安全忽略，
//!   因此反序列化不使用 `deny_unknown_fields`——这是本 crate 与「节点配置文件」
//!   （打错字要报错）刻意相反的选择。
//! - 不兼容的改动必须提升 [`PROTOCOL_VERSION`]；主控在版本不受支持时**如实拒绝**
//!   并同时报出两个版本，而不是半工作（execution-node spec）。
//! - [`RejectCode::is_permanent`] 是节点侧重试策略的唯一依据：永久拒绝不重试，
//!   瞬时拒绝退避重试。把「该不该重试」写进协议，避免两端各自猜测。

use serde::{Deserialize, Serialize};

/// 当前协议版本。任何不兼容改动都必须提升它。
pub const PROTOCOL_VERSION: u32 = 1;

/// 节点标识长度上限。
pub const MAX_NODE_ID_LEN: usize = 64;

/// 校验节点标识：非空、长度受限、仅 ASCII 字母数字与 `-` `_` `.`。
///
/// 放在契约 crate 里而不是各侧各写一份：**允许什么 id 是双方共识的一部分**——
/// 若节点认为合法而主控认为非法，会在握手处出现"看起来像 bug"的拒绝。
/// 返回 trim 后的标识，或人可读的拒绝原因。
pub fn validate_node_id(raw: &str) -> Result<&str, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("节点标识不能为空".into());
    }
    let len = trimmed.chars().count();
    if len > MAX_NODE_ID_LEN {
        return Err(format!("节点标识过长（{len} 字符 > {MAX_NODE_ID_LEN}）"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "节点标识 {trimmed:?} 含非法字符（只允许 ASCII 字母、数字与 - _ .）"
        ));
    }
    Ok(trimmed)
}

/// 节点发往主控的第一个报文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// 节点实现的协议版本。
    pub protocol_version: u32,
    /// 稳定节点标识（i2）。
    pub node_id: String,
    /// 本次进入链路的凭据：首次配对照 join token，之后照长期凭据。
    pub auth: NodeAuth,
    /// 能力清单（主控据此决定可选项；不假定节点有什么）。
    #[serde(default)]
    pub manifest: CapabilityManifest,
}

/// 节点用于进入链路的凭据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeAuth {
    /// 一次性 join token（配对）：换取长期凭据。
    JoinToken {
        /// 操作者在主控侧签发的原始 token。
        token: String,
    },
    /// 长期凭据（已配对）。
    Credential {
        /// 主控签发时给出的原始凭据。
        secret: String,
    },
}

/// 节点自述的能力清单。全部字段缺省即空——**不假定、不猜测**。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityManifest {
    /// 节点上配置的 agent kind 及各自可达性（可达性由**节点自测**）。
    #[serde(default)]
    pub agent_kinds: Vec<AgentKindCapability>,
    /// 节点持有的 provider 清单（`upstream = control-plane-router` 时为空）。
    #[serde(default)]
    pub providers: Vec<String>,
    /// 逐执行体的 mode 强制能力（能强制才敢声称强制）。
    #[serde(default)]
    pub mode_enforcement: Vec<ModeEnforcement>,
}

/// 一个 agent kind 及其可达性。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentKindCapability {
    /// kind slug（开放注册表的键）。
    pub kind: String,
    /// 该 kind 是否可服务新会话。
    pub reachable: bool,
    /// 不可达时的成因（可达时为 `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
}

/// 某个执行体能否**强制**会话 mode（不能强制的只能如实降级为「建议」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeEnforcement {
    /// 执行体标识（如 `acp:claude` / `native`）。
    pub execution_body: String,
    /// 是否可强制 mode。
    pub enforces_mode: bool,
}

/// 主控对 [`Hello`] 的应答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAck {
    /// 主控实现的协议版本。
    pub protocol_version: u32,
    /// 结果。
    pub outcome: HelloOutcome,
    /// 主控 router 的地址（含 scheme，如 `http://10.0.0.5:8787`）。
    ///
    /// 节点配置 `upstream = control-plane-router` 时，用**主控告知的**这个地址把
    /// agent 的模型流量指回主控经 router 出网（设计 D8）。地址必须由主控给出：
    /// 节点猜不到（它可能在内网、可能经反代），因此 `None` 时节点**如实拒绝**
    /// 而不是猜一个（7.2）。旧主控不带该字段 → 缺省 `None`，解析不受影响。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router_url: Option<String>,
    /// 节点访问该 router 用的凭据（控制面未要求鉴权时缺省）。
    ///
    /// 这是**主控签发的 router 令牌**，不是 provider 凭据：节点仍不持有任何
    /// provider 密钥。节点只在内存里用它拼 spawn env，不落盘。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router_token: Option<String>,
}

/// 握手结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum HelloOutcome {
    /// 接受。配对成功时携带新签发的长期凭据（只出现这一次）。
    Accepted {
        /// 新签发的长期凭据；已完成配对、仅凭凭据接入时为 `None`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credential: Option<String>,
    },
    /// 拒绝，附机器可判别的码与人可读的成因。
    Rejected {
        /// 拒绝码。
        code: RejectCode,
        /// 人类可读成因。
        cause: String,
    },
}

/// 拒绝码。节点据此决定「重试」还是「停下并如实上报」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectCode {
    /// 协议版本不受支持（永久）。
    ProtocolVersionUnsupported,
    /// join token 不认识（永久）。
    InvalidJoinToken,
    /// join token 已过期（永久）。
    JoinTokenExpired,
    /// join token 已被消费（永久）。
    JoinTokenConsumed,
    /// 凭据已被吊销（永久）。
    CredentialRevoked,
    /// 凭据不正确（永久）。
    CredentialInvalid,
    /// 同一 node id 已有在线节点（永久）。
    NodeIdConflict,
    /// 报文本身不合法（永久）。
    MalformedHello,
    /// 主控暂时不可用（瞬时，可退避重试）。
    ControlPlaneUnavailable,
    /// 对方报出的拒绝码本端还不认识（对端比本端新）。
    ///
    /// `#[serde(other)]`：容忍未知码而不是让整帧解析失败——协议是长寿命契约，
    /// 旧节点不该因为主控多了一个拒绝码就断了握手。语义按**永久**处理：看不懂的
    /// 拒绝不重试（重试只会在日志里刷屏）。
    #[serde(other)]
    Unknown,
}

impl RejectCode {
    /// 该拒绝是否为**永久**：永久拒绝不应退避重试（重试只会刷日志）。
    pub fn is_permanent(&self) -> bool {
        !matches!(self, RejectCode::ControlPlaneUnavailable)
    }

    /// 稳定的字符串形式（日志与测试断言用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            RejectCode::ProtocolVersionUnsupported => "protocol_version_unsupported",
            RejectCode::InvalidJoinToken => "invalid_join_token",
            RejectCode::JoinTokenExpired => "join_token_expired",
            RejectCode::JoinTokenConsumed => "join_token_consumed",
            RejectCode::CredentialRevoked => "credential_revoked",
            RejectCode::CredentialInvalid => "credential_invalid",
            RejectCode::NodeIdConflict => "node_id_conflict",
            RejectCode::MalformedHello => "malformed_hello",
            RejectCode::ControlPlaneUnavailable => "control_plane_unavailable",
            RejectCode::Unknown => "unknown",
        }
    }
}

/// 便捷构造：拒绝应答。
pub fn rejected(protocol_version: u32, code: RejectCode, cause: impl Into<String>) -> HelloAck {
    HelloAck {
        protocol_version,
        outcome: HelloOutcome::Rejected {
            code,
            cause: cause.into(),
        },
        router_url: None,
        router_token: None,
    }
}

/// 便捷构造：接受应答。
pub fn accepted(credential: Option<String>) -> HelloAck {
    accepted_with_router(credential, None, None)
}

/// 便捷构造：接受应答，并把本主控的 router 端点一并告知节点（7.2）。
///
/// 单列一个构造函数而不是改 [`accepted`] 的签名：已有调用方（主控侧）不需要
/// 知道 router 的存在，缺省即「没告知」，节点据此如实拒绝而不是猜地址。
pub fn accepted_with_router(
    credential: Option<String>,
    router_url: Option<String>,
    router_token: Option<String>,
) -> HelloAck {
    HelloAck {
        protocol_version: PROTOCOL_VERSION,
        outcome: HelloOutcome::Accepted { credential },
        router_url,
        router_token,
    }
}

/// 读出应答里的拒绝信息（若有）。
pub fn rejection_of(ack: &HelloAck) -> Option<(RejectCode, &str)> {
    match &ack.outcome {
        HelloOutcome::Rejected { code, cause } => Some((*code, cause.as_str())),
        HelloOutcome::Accepted { .. } => None,
    }
}


// ── 会话协议（add-remote-execution-node group 3/4）────────────────────────────
//
// 粗粒度会话协议（设计 D2）：链路只承载**会话级**操作（prompt / cancel / close /
// 模型期望值 …），节点自己决定会话与 turn 怎么组。驱动内部词表（AcpEvent /
// AcpCommand）不过河，从而不会被冻结成 wire 协议。
//
// 编解码纪律与握手一致：字段可选 + 缺省，**容忍未知字段与未知 kind**。特别地，
// [`LogEntry::kind`] 用字符串而不是枚举——节点可能发出控制面还不认识的条目类型
// （新工具、新用量形态），控制面必须能安全忽略而不是整帧解析失败。

/// 链路帧：所有帧都是 JSON 文本，外层带 `frame` 标签。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case")]
pub enum Frame {
    /// 控制面 → 节点：一次会话操作；节点用同 `id` 的 [`Frame::Response`] 应答。
    Request {
        /// 关联号（由控制面分配，单调即可，无需全局唯一）。
        id: u64,
        /// 操作本身。
        op: SessionOp,
    },
    /// 节点 → 控制面：对某次请求的应答。
    Response {
        /// 与请求相同的关联号。
        id: u64,
        /// 结果。
        result: SessionResult,
    },
    /// 节点 → 控制面：主动事件（turn 批 / 状态 / 退出 / 回收水位线）。
    Event {
        /// 事件本身。
        event: SessionEvent,
    },
}

/// 会话模式（设计 D6：`desired` 由控制面持有，`effective` 由节点如实回报）。
///
/// 语义是**节点的解释**，执行体可以更细；执行体做不到的模式必须如实上报做不到，
/// 而不是假装生效（见 `Spawned.mode` 与 `effective_mode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// 每个受门控的动作都要问（**缺省**；`auto` 永远不是缺省）。
    #[default]
    Ask,
    /// 编辑类动作放行，其它受门控动作仍要问。
    Edit,
    /// 受门控动作一律放行，但仍留审计。
    Allow,
    /// 完全不门控；必须留审计痕迹（谁在什么时候把它打开过）。
    Auto,
}

/// 动作类别：门控的粒度。节点只做粗分类，执行体可以更细。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateCategory {
    /// 写文件 / 打补丁。
    Edit,
    /// 执行命令。
    Execute,
    /// 其它可能改变机器的动作。
    Other,
}

impl SessionMode {
    /// 稳定字符串（日志、审计与测试断言用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionMode::Ask => "ask",
            SessionMode::Edit => "edit",
            SessionMode::Allow => "allow",
            SessionMode::Auto => "auto",
        }
    }

    /// 解析：**不认识的模式返回 `None`**（调用方据此如实拒绝，而不是悄悄降级）。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ask" => Some(SessionMode::Ask),
            "edit" => Some(SessionMode::Edit),
            "allow" => Some(SessionMode::Allow),
            "auto" => Some(SessionMode::Auto),
            _ => None,
        }
    }

    /// 该模式是否对这类动作**直接放行、不问**。
    pub fn allows_without_asking(&self, category: GateCategory) -> bool {
        match self {
            SessionMode::Ask => false,
            SessionMode::Edit => matches!(category, GateCategory::Edit),
            SessionMode::Allow | SessionMode::Auto => true,
        }
    }

    /// 该模式是否完全不门控（`auto` 是唯一一个）。
    pub fn is_ungated(&self) -> bool {
        matches!(self, SessionMode::Auto)
    }
}

/// 审批决定。
///
/// 只有三档：`escalate`（带理由的一次性放行）是**原生内核专属**，在 ACP 路径上降级为
/// `allow_once`（unify-permission-approval-vocabulary）；节点链路因此不承载它。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// 只放行这一次。
    AllowOnce,
    /// 本会话内放行同类动作。
    AllowSession,
    /// 拒绝。
    Deny,
}

impl ApprovalDecision {
    /// 稳定字符串。
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalDecision::AllowOnce => "allow_once",
            ApprovalDecision::AllowSession => "allow_session",
            ApprovalDecision::Deny => "deny",
        }
    }
}

/// 一个悬空的审批请求（对账用：主控缺席期间 park 的请求，返回后要能全部看见）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParkedApproval {
    /// 目标会话。
    pub session_id: String,
    /// 请求标识（决定按它关联回来）。
    pub request_id: String,
    /// 工具/动作名。
    pub tool: String,
    /// 动作类别。
    pub category: GateCategory,
    /// 会话当时的模式（审计用）。
    pub mode: SessionMode,
}

/// 一份材料文件。`path` **相对材料根**；两侧都必须拒绝绝对路径与 `..`
/// （否则远端就有了一条写出自己目录之外的路径）。内容按文本传（skills / memory /
/// subagent 定义都是文本；二进制材料不在本期范围）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialFile {
    /// 相对路径（用 `/` 分隔）。
    pub path: String,
    /// 文本内容。
    pub content: String,
}

/// 校验材料相对路径。规则是**两侧共识**（同 `validate_node_id`）：节点按它决定
/// 往哪写，控制面按它决定发什么；不一致就会出现"控制面觉得没问题、节点拒绝写入"。
pub fn validate_material_path(raw: &str) -> Result<&str, String> {
    let path = raw.trim();
    if path.is_empty() {
        return Err("材料路径不能为空".into());
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(format!("材料路径 {path:?} 必须是相对路径"));
    }
    // Windows 盘符。
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        return Err(format!("材料路径 {path:?} 必须是相对路径"));
    }
    for segment in path.split(['/', '\\']) {
        if segment == ".." {
            return Err(format!("材料路径 {path:?} 不得包含 `..`"));
        }
    }
    Ok(path)
}

/// 控制面请求的会话操作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SessionOp {
    /// 建立一个会话。`session_id` 由**控制面**发行（设计 D3）。
    Spawn {
        /// 控制面发行的会话 id。
        session_id: String,
        /// 项目目录（在**节点**上的一份路径）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_dir: Option<String>,
        /// 期望的执行体 kind（如 `claude`）；缺省由节点决定。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_kind: Option<String>,
        /// 期望模型。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// 期望 mode（节点回报实际生效值）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        /// 期望的**节点本地 provider profile** 名字（7.1）。
        ///
        /// 凭据留在节点上，主控只选名字、不碰密钥。名字不认识或该 profile 在本
        /// 节点上不可用时，节点**如实拒绝**或回报实际生效值——不静默换成别的
        /// profile（那会让操作者以为选中的是另一个）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
    },
    /// 投递一轮输入。
    Prompt {
        /// 目标会话。
        session_id: String,
        /// 输入文本。
        text: String,
    },
    /// 取消在飞 turn（会话保留、可继续）。
    Cancel {
        /// 目标会话。
        session_id: String,
    },
    /// 关闭会话（终止子进程）。
    Close {
        /// 目标会话。
        session_id: String,
    },
    /// 期望模型变更（节点回报实际生效值）。
    SetModel {
        /// 目标会话。
        session_id: String,
        /// 期望模型 id。
        model_id: String,
    },
    /// 按 seq 回拉精确序列（合并只影响传输，不影响日志保真）。
    LogFrom {
        /// 目标会话。
        session_id: String,
        /// 起始 seq（含）。
        from_seq: u64,
    },
    /// 列出节点当前持有的会话（对账用）。
    ListSessions,
    /// 取某会话的状态快照（对账用）。
    Snapshot {
        /// 目标会话。
        session_id: String,
    },
    /// 期望模式变更（节点回报实际生效值；`auto` 会留审计痕迹）。
    SetMode {
        /// 目标会话。
        session_id: String,
        /// 期望模式（字符串；不认识的模式会被如实拒绝）。
        mode: String,
    },
    /// 回填一个审批决定，按 `request_id` 关联。
    ///
    /// 未知 id、或会话已终止时的迟到决定 → **可判别拒绝**且不产生任何效果。
    ApprovalAnswer {
        /// 目标会话。
        session_id: String,
        /// 请求标识。
        request_id: String,
        /// 决定。
        decision: ApprovalDecision,
    },
    /// 列出本节点当前**悬空**的审批请求（对账用）。
    ParkedApprovals,
    /// **节点 → 控制面**：拉取操作者级材料。`version = None` 表示"给我当前版本"。
    ///
    /// 方向与大多数操作相反，这是刻意的：材料由节点在**会话创建时**按需拉取，
    /// 而不是由控制面推下去（推送要求控制面知道节点拓扑与落点，且主控缺席时
    /// 节点连启动都做不了）。控制面的变更通知只带版本信号，内容只走这个应答。
    FetchMaterials {
        /// 期望版本；`None` = 当前版本。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    /// 连通性探测。
    Ping,
    /// 「**这个路径在节点上**是什么？」（3.3 的同一条原则：路径可用性由被 spawn 的
    /// 那一侧判定，见 spec `agent-workbench`）。
    ///
    /// 主控存的是 `(节点, 路径)`，但只有节点知道那个路径在自己这台机器上是否存在、
    /// 是不是目录——主控本机的 `stat` 与节点上的事实无关，拿它去判定等于把两台机器
    /// 混为一谈。
    ///
    /// 判定不出来（权限不足等）时**不是** `exists:false`，而是 typed `Rejected`：
    /// 「读不到」与「不存在」是两回事，把前者报成后者会让操作者以为路径没了。
    CheckPath {
        /// 待判定的路径（节点本地路径，原样使用，不做规范化猜测）。
        path: String,
    },
}

/// 一次操作的应答。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum SessionResult {
    /// 无返回值的成功。
    Ok,
    /// 会话已建立。`epoch` 是节点侧日志纪元（重置才递增）。
    Spawned {
        /// 日志纪元。
        epoch: u64,
        /// 实际生效的执行体 kind。
        agent_kind: String,
        /// 实际生效的模型（节点无模型概念时为 `None`）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// 实际生效的 mode（无法强制时如实回报，可能是 `None`）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        /// 实际生效的 provider profile（7.1）：节点本地 profile 名，或经主控 router
        /// 出网时的 `control-plane-router`；未选择任何 profile → `None`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        /// desired 与 effective provider 不同的成因（相同 → `None`）。
        /// **不静默降级**：差异与成因都要能被控制面呈现。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_cause: Option<String>,
        /// 本会话钉住的操作者级材料版本（未使用 → `None`）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        materials_version: Option<String>,
    },
    /// `LogFrom` 的结果。
    Log {
        /// 日志纪元（与快照对账用）。
        epoch: u64,
        /// 该区间的条目（按 seq 升序）。
        entries: Vec<LogEntry>,
        /// 该会话当前最后一条 seq（便于控制面推进游标）。
        last_seq: u64,
    },
    /// `ListSessions` 的结果。
    Sessions {
        /// 会话摘要列表。
        sessions: Vec<SessionSummary>,
    },
    /// `Snapshot` 的结果。
    Snapshot {
        /// 会话摘要。
        summary: SessionSummary,
        /// 日志纪元。
        epoch: u64,
        /// 日志最后一条 seq。
        last_seq: u64,
        /// 节点已回收到的 seq（水位线）：控制面据此把 <= 该值的缺口标为
        /// 「节点已回收」，而不是永远 pending。
        reclaimed_through_seq: u64,
    },
    /// `SetModel` 的应答：**实际生效值**（节点无法应用期望值时如实回报，
    /// 而不是把期望值回显成已生效）。
    ModelSet {
        /// 实际生效的模型。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    /// `ParkedApprovals` 的结果。
    ParkedApprovals {
        /// 当前悬空的审批请求。
        approvals: Vec<ParkedApproval>,
    },
    /// `ApprovalAnswer` 的结果：`applied=false` 表示该决定没有产生任何效果
    /// （会话已终止等），调用方据此区分「已生效」与「被丢弃」。
    ApprovalApplied {
        /// 是否真的生效了。
        applied: bool,
    },
    /// `FetchMaterials` 的应答：完整材料包（**内容只在应答里**，通知里永远不带）。
    Materials {
        /// 本次交付的版本号。节点把它钉在会话上，此后该会话不再跟随更新。
        version: String,
        /// 材料文件（按 `path` 升序，便于比对与断言）。
        files: Vec<MaterialFile>,
    },
    /// `Ping` 的应答。
    Pong,
    /// `CheckPath` 的应答：路径**在节点上**的判定结果。
    ///
    /// `exists:false` 只表示「确定不存在」（`NotFound` / 路径中间不是目录）；
    /// 读不到元数据是 typed `Rejected`，不走这里（未知 ≠ 不存在）。
    PathChecked {
        /// 路径是否存在于节点上。
        exists: bool,
        /// 是否是一个目录（`exists:false` 时恒为 `false`）。
        is_dir: bool,
    },
    /// 如实拒绝：带可判别码与成因，且**没有任何副作用**。
    Rejected {
        /// 拒绝码。
        code: SessionRejectCode,
        /// 人类可读成因。
        cause: String,
    },
}

/// 会话操作被拒绝的原因。与 [`RejectCode`] 同款：把「该不该重试」写进协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRejectCode {
    /// 会话不存在（永久）。
    UnknownSession,
    /// 会话已关闭：日志仍可查询（执行事实不因关闭消失），但不再接受输入
    /// （永久；要重新工作请重新 spawn）。
    SessionClosed,
    /// 审批请求标识未知（永久）——含「已被决议过的迟到决定」与「凭空来的 id」。
    UnknownApprovalRequest,
    /// 模式字符串不认识（永久）：节点**不**悄悄降级成别的模式。
    UnsupportedMode,
    /// 会话 id 已被占用（永久；重新 spawn 前应先 close）。
    DuplicateSession,
    /// 项目目录在节点上不可用（永久，直到路径变化）。
    UnusableProjectDir,
    /// 节点并发上限已满（瞬时：等现有会话结束）。
    OverCapacity,
    /// 节点存储触顶（瞬时：等清理或扩容）。
    StorageExhausted,
    /// 执行体 kind 未配置或不可达（永久，直到节点配置变化）。
    UnsupportedAgentKind,
    /// 请求的 provider profile / 上游在**本节点**上无法应用（永久，直到节点或
    /// 控制面配置变化）：profile 名字不认识、节点本地凭据环境变量缺失、
    /// `upstream = control-plane-router` 但控制面没告知 router 地址等。
    ///
    /// 用**一个**码配精确成因，而不是为每种情形各开一个码：调用方要做的事是
    /// 同一件——把成因呈现给操作者，改配置，不重试。
    ProviderUnavailable,
    /// 节点侧内部故障（瞬时）。
    NodeError,
    /// 对方报出的拒绝码本端还不认识（对端比本端新；容忍未知变体）。
    #[serde(other)]
    Unknown,
}

impl SessionRejectCode {
    /// 该拒绝是否为**永久**：永久拒绝不该被反复重试（只会在日志里刷屏）。
    pub fn is_permanent(&self) -> bool {
        !matches!(
            self,
            SessionRejectCode::OverCapacity
                | SessionRejectCode::StorageExhausted
                | SessionRejectCode::NodeError
        )
    }

    /// 稳定的字符串形式（日志与测试断言用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionRejectCode::UnknownSession => "unknown_session",
            SessionRejectCode::SessionClosed => "session_closed",
            SessionRejectCode::UnknownApprovalRequest => "unknown_approval_request",
            SessionRejectCode::UnsupportedMode => "unsupported_mode",
            SessionRejectCode::DuplicateSession => "duplicate_session",
            SessionRejectCode::UnusableProjectDir => "unusable_project_dir",
            SessionRejectCode::OverCapacity => "over_capacity",
            SessionRejectCode::StorageExhausted => "storage_exhausted",
            SessionRejectCode::UnsupportedAgentKind => "unsupported_agent_kind",
            SessionRejectCode::ProviderUnavailable => "provider_unavailable",
            SessionRejectCode::NodeError => "node_error",
            SessionRejectCode::Unknown => "unknown",
        }
    }
}

/// 节点主动上报的事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    /// 相位变化（spawning / active / idle / waiting_approval / closed …）。
    State {
        /// 目标会话。
        session_id: String,
        /// 相位名（字符串：控制面必须容忍它不认识的相位）。
        phase: String,
        /// 补充说明（成因等）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// turn 输出批（传输层已按时间窗合并；日志里的原始序列仍然完整）。
    TurnBatch {
        /// 目标会话。
        session_id: String,
        /// 该批所属的**日志纪元**。
        ///
        /// 为什么批也要带纪元：只有事件与回拉带纪元的话，控制面会在「刚学到纪元」
        /// 与「节点日志被重置」之间无法区分——前者是校正，后者是断裂，混为一谈就会
        /// 把两条时间线接成一条。`0` = 未知（不带该字段的旧发送方），接收方按校正处理。
        #[serde(default)]
        epoch: u64,
        /// 本批第一条的 seq（控制面据此判断是否有缺口）。
        from_seq: u64,
        /// 批内条目（按 seq 升序）。
        entries: Vec<LogEntry>,
        /// 合并缓冲区曾溢出、本批与上一批之间可能有被压缩的片段——
        /// 精确内容可按 seq 回拉。**绝不静默丢**。
        #[serde(default)]
        coalesced_overflow: bool,
    },
    /// 会话结束（子进程死亡 / 节点主动关闭）。
    Exited {
        /// 目标会话。
        session_id: String,
        /// 成因。
        cause: String,
    },
    /// **需要审批**：节点在此停住并上报，等控制面的决定。
    ///
    /// 节点侧没有任何本地裁决路径（6.6 的安全性质）：主控不可达时请求就**一直
    /// parked**——不超时拒绝、也不降级放行。
    ApprovalRequested {
        /// 目标会话。
        session_id: String,
        /// 请求标识（决定按它回来）。
        request_id: String,
        /// 工具/动作名。
        tool: String,
        /// 动作类别。
        category: GateCategory,
        /// 当时的模式。
        mode: SessionMode,
    },
    /// 门控已有结论。`source` 说明**是谁**给的结论（`control-plane` / `mode:auto` …），
    /// 这样「自动放行」与「人放行」在审计上不会混为一谈。
    GateResolved {
        /// 目标会话。
        session_id: String,
        /// 请求标识。
        request_id: String,
        /// 结论（决定字符串，或 `auto_allowed`）。
        decision: String,
        /// 结论来源。
        source: String,
    },
    /// 日志被回收：`reclaimed_through_seq` 之前的条目在节点上已不可得。
    Reclaimed {
        /// 目标会话。
        session_id: String,
        /// 已回收到的 seq（含）。
        reclaimed_through_seq: u64,
    },
    /// **控制面 → 节点**：材料有新版本（**只带版本号，不带内容**）。
    ///
    /// 节点收到后**不立刻**拉取，而是在下一个会话创建时按需拉取——这样：
    /// 已有会话继续用它钉住的旧版本（可复现），新会话拿到新版本，且控制面
    /// 不必知道节点在哪、目录怎么排。
    MaterialsChanged {
        /// 新版本号。
        version: String,
    },
    /// 日志纪元变更（节点重置/重装）：控制面据此把时间线标记为不连续，
    /// 而不是把两条时间线接成一条。
    EpochChanged {
        /// 目标会话。
        session_id: String,
        /// 新纪元。
        epoch: u64,
    },
}

/// 会话日志的一条条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// 会话内单调递增的序号（节点是唯一写者，因此不会冲突）。
    pub seq: u64,
    /// 条目类型。**字符串**：节点可能发出控制面还不认识的新类型，控制面必须
    /// 容忍并忽略渲染，而不是整帧解析失败。
    pub kind: String,
    /// 文本内容（thinking / 输出 / 错误 …）。
    #[serde(default)]
    pub text: String,
    /// 结构化附加信息（工具名、用量数字、拒绝成因 …），形状由 kind 决定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// 落账时间（unix 秒）。保留期策略按它回收；`0` = 未知（旧条目或旧发送方），
    /// 回收时**保守处理**（不回收年龄未知的条目）。
    #[serde(default)]
    pub at_unix: i64,
}

/// 会话摘要（列表与快照共用；不含日志正文）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    /// 会话 id（控制面发行）。
    pub session_id: String,
    /// 相位名。
    pub phase: String,
    /// 实际生效的执行体 kind。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,
    /// 实际生效的模型。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// **实际生效**的 mode（节点真正在强制/遵守的那个）。无法强制 mode 的执行体
    /// 如实回报 `None`（「没有可声称生效的 mode」），而不是把期望值回显成已生效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// 控制面持有的**期望** mode。两者不同即说明执行体无法强制期望值——
    /// 界面与对账据此如实呈现差异，而不是把期望值回显成已生效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_mode: Option<String>,
    /// **实际生效**的 provider profile（7.1）：节点本地 profile 名，或
    /// `control-plane-router`（经主控 router 出网）；未选择 → `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// 控制面持有的**期望** provider。两者不同即说明期望值没能照做——
    /// 界面据此呈现差异，而不是把期望值回显成已生效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_provider: Option<String>,
    /// desired 与 effective provider 不同的成因（相同 → `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_cause: Option<String>,
    /// 该会话**钉住**的操作者级材料版本（未使用材料 → `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materials_version: Option<String>,
    /// 日志最后一条 seq（空日志为 0）。
    pub last_seq: u64,
    /// 日志纪元。
    pub epoch: u64,
}

/// 便捷构造：拒绝应答。
pub fn session_rejected(id: u64, code: SessionRejectCode, cause: impl Into<String>) -> Frame {
    Frame::Response {
        id,
        result: SessionResult::Rejected {
            code,
            cause: cause.into(),
        },
    }
}

/// 读出应答里的拒绝信息（若有）。
pub fn session_rejection_of(result: &SessionResult) -> Option<(SessionRejectCode, &str)> {
    match result {
        SessionResult::Rejected { code, cause } => Some((*code, cause.as_str())),
        _ => None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> CapabilityManifest {
        CapabilityManifest {
            agent_kinds: vec![
                AgentKindCapability {
                    kind: "claude".into(),
                    reachable: true,
                    cause: None,
                },
                AgentKindCapability {
                    kind: "opencode".into(),
                    reachable: false,
                    cause: Some("binary not on PATH".into()),
                },
            ],
            providers: vec!["anthropic".into()],
            mode_enforcement: vec![ModeEnforcement {
                execution_body: "acp:claude".into(),
                enforces_mode: false,
            }],
        }
    }

    #[test]
    fn hello_round_trips_with_a_join_token() {
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION,
            node_id: "dev-box".into(),
            auth: NodeAuth::JoinToken {
                token: "tok-123".into(),
            },
            manifest: sample_manifest(),
        };
        let json = serde_json::to_string(&hello).unwrap();
        assert_eq!(serde_json::from_str::<Hello>(&json).unwrap(), hello);
        // auth 是内部标签，便于机器判别与演进。
        assert!(json.contains("\"kind\":\"join_token\""), "{json}");
    }

    #[test]
    fn hello_round_trips_with_a_credential() {
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION,
            node_id: "dev-box".into(),
            auth: NodeAuth::Credential {
                secret: "s3cr3t".into(),
            },
            manifest: CapabilityManifest::default(),
        };
        let json = serde_json::to_string(&hello).unwrap();
        assert_eq!(serde_json::from_str::<Hello>(&json).unwrap(), hello);
        assert!(json.contains("\"kind\":\"credential\""), "{json}");
    }

    #[test]
    fn manifest_and_optional_fields_default_when_absent() {
        // 只给最小报文：旧版本节点（没有 manifest）必须仍能被解析。
        let minimal = r#"{"protocol_version":1,"node_id":"n","auth":{"kind":"credential","secret":"s"}}"#;
        let hello: Hello = serde_json::from_str(minimal).unwrap();
        assert_eq!(hello.manifest, CapabilityManifest::default());
        assert!(hello.manifest.agent_kinds.is_empty());
        assert!(hello.manifest.mode_enforcement.is_empty());
    }

    #[test]
    fn unknown_fields_are_tolerated_for_forward_compatibility() {
        // 未来的节点会多发字段；旧主控必须能安全忽略（与节点配置文件的严格
        // 解析刻意相反：协议是长寿命契约，配置是打错字要报错）。
        let with_extras = r#"{
            "protocol_version": 1,
            "node_id": "n",
            "auth": {"kind": "credential", "secret": "s"},
            "manifest": {"agent_kinds": [], "providers": [], "mode_enforcement": [], "future_thing": 7},
            "future_top_level": {"a": 1}
        }"#;
        let hello: Hello = serde_json::from_str(with_extras).unwrap();
        assert_eq!(hello.node_id, "n");
    }

    #[test]
    fn ack_round_trips_both_outcomes() {
        let ok = accepted(Some("new-secret".into()));
        let json = serde_json::to_string(&ok).unwrap();
        assert_eq!(serde_json::from_str::<HelloAck>(&json).unwrap(), ok);
        assert!(json.contains("\"result\":\"accepted\""), "{json}");
        assert!(rejection_of(&ok).is_none());

        let no = rejected(PROTOCOL_VERSION, RejectCode::CredentialRevoked, "revoked at 12:00");
        let json = serde_json::to_string(&no).unwrap();
        assert_eq!(serde_json::from_str::<HelloAck>(&json).unwrap(), no);
        assert!(json.contains("\"result\":\"rejected\""), "{json}");
        let (code, cause) = rejection_of(&no).unwrap();
        assert_eq!(code, RejectCode::CredentialRevoked);
        assert_eq!(cause, "revoked at 12:00");
    }

    #[test]
    fn accepted_without_credential_omits_the_field() {
        let ack = accepted(None);
        let json = serde_json::to_string(&ack).unwrap();
        assert!(!json.contains("credential"), "非配对接受不应带凭据：{json}");
    }

    #[test]
    fn only_unavailability_is_transient() {
        let all = [
            RejectCode::ProtocolVersionUnsupported,
            RejectCode::InvalidJoinToken,
            RejectCode::JoinTokenExpired,
            RejectCode::JoinTokenConsumed,
            RejectCode::CredentialRevoked,
            RejectCode::CredentialInvalid,
            RejectCode::NodeIdConflict,
            RejectCode::MalformedHello,
            RejectCode::ControlPlaneUnavailable,
            RejectCode::Unknown,
        ];
        for code in all {
            let expect_permanent = code != RejectCode::ControlPlaneUnavailable;
            assert_eq!(
                code.is_permanent(),
                expect_permanent,
                "{} 的重试语义与预期不符",
                code.as_str()
            );
        }
    }

    #[test]
    fn reject_codes_have_stable_snake_case_wire_names() {
        // 线上名字是契约的一部分：改名会让两侧对「同一拒绝」理解不同。
        let json = serde_json::to_string(&HelloOutcome::Rejected {
            code: RejectCode::JoinTokenConsumed,
            cause: "used".into(),
        })
        .unwrap();
        assert!(json.contains("join_token_consumed"), "{json}");
        assert_eq!(
            serde_json::from_str::<RejectCode>("\"node_id_conflict\"").unwrap(),
            RejectCode::NodeIdConflict
        );
    }

    #[test]
    fn protocol_version_is_advertised_on_both_sides() {
        let ack = accepted(None);
        assert_eq!(ack.protocol_version, PROTOCOL_VERSION);
        assert_eq!(rejected(0, RejectCode::ProtocolVersionUnsupported, "x").protocol_version, 0);
    }

    #[test]
    fn node_id_validation_is_shared_consensus() {
        assert_eq!(validate_node_id("  dev-box ").unwrap(), "dev-box");
        for ok in ["a", "node_1", "box.example", "Node-9"] {
            assert!(validate_node_id(ok).is_ok(), "{ok} 应合法");
        }
        assert!(validate_node_id("").is_err());
        assert!(validate_node_id("   ").is_err());
        assert!(validate_node_id("dev box").is_err());
        assert!(validate_node_id("dev/box").is_err());
        assert!(validate_node_id("节点").is_err());
        assert!(validate_node_id(&"a".repeat(MAX_NODE_ID_LEN + 1)).is_err());
        assert!(validate_node_id(&"a".repeat(MAX_NODE_ID_LEN)).is_ok());
    }

    // ── 会话协议（group 3/4）────────────────────────────────────────────────

    #[test]
    fn frame_round_trips_for_every_shape() {
        let frames = vec![
            Frame::Request {
                id: 7,
                op: SessionOp::Spawn {
                    session_id: "s-1".into(),
                    project_dir: Some("/srv/repo".into()),
                    agent_kind: Some("claude".into()),
                    model: Some("m".into()),
                    mode: Some("ask".into()),
                    provider: Some("anthropic".into()),
                },
            },
            Frame::Request {
                id: 8,
                op: SessionOp::Prompt {
                    session_id: "s-1".into(),
                    text: "hi".into(),
                },
            },
            Frame::Request {
                id: 9,
                op: SessionOp::LogFrom {
                    session_id: "s-1".into(),
                    from_seq: 3,
                },
            },
            Frame::Request {
                id: 10,
                op: SessionOp::CheckPath {
                    path: "/srv/repo".into(),
                },
            },
            Frame::Response {
                id: 7,
                result: SessionResult::Spawned {
                    epoch: 1,
                    agent_kind: "claude".into(),
                    model: None,
                    mode: Some("ask".into()),
                    provider: Some("anthropic".into()),
                    provider_cause: None,
                    materials_version: Some("v3".into()),
                },
            },
            Frame::Response {
                id: 10,
                result: SessionResult::PathChecked {
                    exists: true,
                    is_dir: true,
                },
            },
            Frame::Response {
                id: 9,
                result: SessionResult::Log {
                    epoch: 1,
                    entries: vec![LogEntry {
                        seq: 3,
                        kind: "output".into(),
                        text: "hello".into(),
                        data: None,
                        at_unix: 0,
                    }],
                    last_seq: 3,
                },
            },
            Frame::Event {
                event: SessionEvent::TurnBatch {
                    session_id: "s-1".into(),
                    epoch: 1,
                    from_seq: 1,
                    entries: vec![],
                    coalesced_overflow: true,
                },
            },
            Frame::Event {
                event: SessionEvent::EpochChanged {
                    session_id: "s-1".into(),
                    epoch: 2,
                },
            },
        ];
        for frame in frames {
            let json = serde_json::to_string(&frame).unwrap();
            assert_eq!(serde_json::from_str::<Frame>(&json).unwrap(), frame, "{json}");
        }
    }

    #[test]
    fn unknown_log_kinds_and_fields_are_tolerated() {
        // 节点可能发出控制面还不认识的条目类型与字段：必须能安全忽略。
        let raw = r#"{
            "frame": "event",
            "event": {
                "event": "turn_batch",
                "session_id": "s-1",
                "epoch": 3,
                "from_seq": 5,
                "entries": [
                    {"seq": 5, "kind": "some_future_kind", "text": "x", "future_field": 1},
                    {"seq": 6, "kind": "output", "text": "y", "data": {"tool": "bash"}}
                ],
                "coalesced_overflow": false,
                "future_top": true
            }
        }"#;
        let frame: Frame = serde_json::from_str(raw).unwrap();
        match frame {
            Frame::Event {
                event: SessionEvent::TurnBatch { entries, .. },
            } => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].kind, "some_future_kind");
                assert_eq!(entries[1].data.as_ref().unwrap()["tool"], "bash");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn optional_session_op_fields_may_be_absent() {
        let raw = r#"{"frame":"request","id":1,"op":{"op":"spawn","session_id":"s"}}"#;
        match serde_json::from_str::<Frame>(raw).unwrap() {
            Frame::Request { op, .. } => match op {
                SessionOp::Spawn {
                    session_id,
                    project_dir,
                    agent_kind,
                    model,
                    mode,
                    provider,
                } => {
                    assert_eq!(session_id, "s");
                    assert!(project_dir.is_none() && agent_kind.is_none());
                    assert!(model.is_none() && mode.is_none());
                    assert!(provider.is_none(), "旧发送方不带 provider → 缺省 None");
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn only_transient_session_rejections_are_retryable() {
        let all = [
            SessionRejectCode::UnknownSession,
            SessionRejectCode::SessionClosed,
            SessionRejectCode::UnknownApprovalRequest,
            SessionRejectCode::UnsupportedMode,
            SessionRejectCode::DuplicateSession,
            SessionRejectCode::UnusableProjectDir,
            SessionRejectCode::OverCapacity,
            SessionRejectCode::StorageExhausted,
            SessionRejectCode::UnsupportedAgentKind,
            SessionRejectCode::ProviderUnavailable,
            SessionRejectCode::NodeError,
            SessionRejectCode::Unknown,
        ];
        for code in all {
            let transient = matches!(
                code,
                SessionRejectCode::OverCapacity
                    | SessionRejectCode::StorageExhausted
                    | SessionRejectCode::NodeError
            );
            assert_eq!(code.is_permanent(), !transient, "{}", code.as_str());
        }
    }

    #[test]
    fn session_rejection_helper_maps_onto_a_frame() {
        let frame = session_rejected(
            11,
            SessionRejectCode::UnusableProjectDir,
            "路径不存在",
        );
        match frame {
            Frame::Response { id, result } => {
                assert_eq!(id, 11);
                let (code, cause) = session_rejection_of(&result).unwrap();
                assert_eq!(code, SessionRejectCode::UnusableProjectDir);
                assert_eq!(cause, "路径不存在");
            }
            other => panic!("{other:?}"),
        }
        assert!(session_rejection_of(&SessionResult::Ok).is_none());
    }

    // ── mode 与审批（group 6）────────────────────────────────────────────────

    #[test]
    fn mode_defaults_to_ask_and_never_to_auto() {
        assert_eq!(SessionMode::default(), SessionMode::Ask);
        assert!(!SessionMode::default().is_ungated());
        assert!(!SessionMode::default().allows_without_asking(GateCategory::Edit));
    }

    #[test]
    fn mode_round_trips_and_unknown_modes_are_rejected_not_defaulted() {
        for mode in [
            SessionMode::Ask,
            SessionMode::Edit,
            SessionMode::Allow,
            SessionMode::Auto,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(serde_json::from_str::<SessionMode>(&json).unwrap(), mode);
            assert_eq!(SessionMode::parse(mode.as_str()), Some(mode));
            // 大小写与空白容错。
            assert_eq!(SessionMode::parse(&format!("  {} ", mode.as_str())), Some(mode));
        }
        assert_eq!(SessionMode::parse("yolo"), None, "不认识的模式必须被如实拒绝");
        assert_eq!(SessionMode::parse(""), None);
    }

    #[test]
    fn mode_decides_which_categories_go_ungated() {
        // ask：全问。
        for c in [GateCategory::Edit, GateCategory::Execute, GateCategory::Other] {
            assert!(!SessionMode::Ask.allows_without_asking(c));
        }
        // edit：只放编辑。
        assert!(SessionMode::Edit.allows_without_asking(GateCategory::Edit));
        assert!(!SessionMode::Edit.allows_without_asking(GateCategory::Execute));
        assert!(!SessionMode::Edit.allows_without_asking(GateCategory::Other));
        // allow/auto：全放（auto 连审计门控都没有）。
        for c in [GateCategory::Edit, GateCategory::Execute, GateCategory::Other] {
            assert!(SessionMode::Allow.allows_without_asking(c));
            assert!(SessionMode::Auto.allows_without_asking(c));
        }
        assert!(SessionMode::Auto.is_ungated());
        assert!(!SessionMode::Allow.is_ungated(), "allow 仍留审计门控");
    }

    #[test]
    fn approval_ops_and_events_round_trip() {
        let frames = vec![
            Frame::Request {
                id: 1,
                op: SessionOp::ApprovalAnswer {
                    session_id: "s-1".into(),
                    request_id: "s-1:req-1".into(),
                    decision: ApprovalDecision::AllowOnce,
                },
            },
            Frame::Request {
                id: 2,
                op: SessionOp::ParkedApprovals,
            },
            Frame::Request {
                id: 3,
                op: SessionOp::SetMode {
                    session_id: "s-1".into(),
                    mode: "allow".into(),
                },
            },
            Frame::Response {
                id: 2,
                result: SessionResult::ParkedApprovals {
                    approvals: vec![ParkedApproval {
                        session_id: "s-1".into(),
                        request_id: "s-1:req-1".into(),
                        tool: "bash".into(),
                        category: GateCategory::Execute,
                        mode: SessionMode::Ask,
                    }],
                },
            },
            Frame::Response {
                id: 1,
                result: SessionResult::ApprovalApplied { applied: true },
            },
            Frame::Event {
                event: SessionEvent::ApprovalRequested {
                    session_id: "s-1".into(),
                    request_id: "s-1:req-1".into(),
                    tool: "bash".into(),
                    category: GateCategory::Execute,
                    mode: SessionMode::Ask,
                },
            },
            Frame::Event {
                event: SessionEvent::GateResolved {
                    session_id: "s-1".into(),
                    request_id: "s-1:req-1".into(),
                    decision: "allow_once".into(),
                    source: "control-plane".into(),
                },
            },
        ];
        for frame in frames {
            let json = serde_json::to_string(&frame).unwrap();
            assert_eq!(serde_json::from_str::<Frame>(&json).unwrap(), frame, "{json}");
        }
    }

    #[test]
    fn approval_decisions_have_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::AllowSession).unwrap(),
            "\"allow_session\""
        );
        assert_eq!(ApprovalDecision::Deny.as_str(), "deny");
    }

    #[test]
    fn the_gate_reject_codes_are_permanent() {
        assert!(SessionRejectCode::UnknownApprovalRequest.is_permanent());
        assert!(SessionRejectCode::UnsupportedMode.is_permanent());
    }

    // ── 材料过河（7.3 / 7.4 / 7.5）──────────────────────────────────────────

    #[test]
    fn material_paths_are_validated_by_the_shared_rule() {
        for ok in ["SKILL.md", "skills/beads/SKILL.md", "memory/notes.md", "a/b/c.txt"] {
            assert_eq!(validate_material_path(ok).unwrap(), ok);
        }
        for bad in [
            "",
            "   ",
            "/etc/passwd",
            "\\windows\\system32",
            "C:/windows",
            "../escape.md",
            "skills/../../escape.md",
            "a/../../b",
        ] {
            assert!(validate_material_path(bad).is_err(), "{bad} 应被拒绝");
        }
        assert_eq!(validate_material_path("  SKILL.md  ").unwrap(), "SKILL.md");
    }

    #[test]
    fn material_ops_and_events_round_trip() {
        let frames = vec![
            Frame::Request {
                id: 1,
                op: SessionOp::FetchMaterials {
                    version: Some("v3".into()),
                },
            },
            Frame::Request {
                id: 2,
                op: SessionOp::FetchMaterials { version: None },
            },
            Frame::Response {
                id: 1,
                result: SessionResult::Materials {
                    version: "v3".into(),
                    files: vec![
                        MaterialFile {
                            path: "skills/beads/SKILL.md".into(),
                            content: "# beads".into(),
                        },
                        MaterialFile {
                            path: "memory/notes.md".into(),
                            content: "hi".into(),
                        },
                    ],
                },
            },
            Frame::Event {
                event: SessionEvent::MaterialsChanged {
                    version: "v4".into(),
                },
            },
        ];
        for frame in frames {
            let json = serde_json::to_string(&frame).unwrap();
            assert_eq!(serde_json::from_str::<Frame>(&json).unwrap(), frame, "{json}");
        }
    }

    #[test]
    fn a_change_notification_carries_no_content() {
        // 通知**只有版本号**：形状断言，防止有人日后顺手把内容塞进去。
        let json = serde_json::to_string(&Frame::Event {
            event: SessionEvent::MaterialsChanged {
                version: "v9".into(),
            },
        })
        .unwrap();
        assert!(json.contains("v9"));
        assert!(
            !json.contains("files") && !json.contains("content"),
            "通知不得携带内容：{json}"
        );
    }

    #[test]
    fn a_summary_can_report_the_pinned_material_version() {
        let summary = SessionSummary {
            session_id: "s-1".into(),
            phase: "active".into(),
            agent_kind: Some("echo".into()),
            model: None,
            mode: Some("ask".into()),
            desired_mode: Some("ask".into()),
            provider: Some("anthropic".into()),
            desired_provider: Some("anthropic".into()),
            provider_cause: None,
            materials_version: Some("v3".into()),
            last_seq: 4,
            epoch: 1,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert_eq!(
            serde_json::from_str::<SessionSummary>(&json).unwrap(),
            summary
        );
        // 不使用材料的会话：字段缺省即不出现。
        let bare = serde_json::to_string(&SessionSummary {
            materials_version: None,
            provider: None,
            desired_provider: None,
            provider_cause: None,
            ..summary
        })
        .unwrap();
        assert!(!bare.contains("materials_version"), "{bare}");
        assert!(!bare.contains("provider"), "未选择 profile 时不出现：{bare}");
    }

    // ── 路径判定（CheckPath）与 provider 归属（7.1 / 7.2）──────────────────

    #[test]
    fn check_path_round_trips_and_carries_no_field_guessing() {
        let req = Frame::Request {
            id: 3,
            op: SessionOp::CheckPath {
                path: "/srv/repo".into(),
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(serde_json::from_str::<Frame>(&json).unwrap(), req);

        for result in [
            SessionResult::PathChecked {
                exists: true,
                is_dir: true,
            },
            SessionResult::PathChecked {
                exists: true,
                is_dir: false,
            },
            SessionResult::PathChecked {
                exists: false,
                is_dir: false,
            },
        ] {
            let json = serde_json::to_string(&result).unwrap();
            assert_eq!(
                serde_json::from_str::<SessionResult>(&json).unwrap(),
                result
            );
        }
    }

    #[test]
    fn spawn_provider_fields_are_optional_and_default_when_absent() {
        // 旧控制面不带 provider：节点必须能解析，并把它当作「未选择」。
        let raw = r#"{"frame":"request","id":1,"op":{"op":"spawn","session_id":"s"}}"#;
        match serde_json::from_str::<Frame>(raw).unwrap() {
            Frame::Request { op, .. } => match op {
                SessionOp::Spawn { provider, .. } => assert!(provider.is_none()),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
        // 旧节点回报的 Spawned 不带 provider 字段：也必须能解析。
        let raw = r#"{"frame":"response","id":1,"result":{"result":"spawned",
            "epoch":1,"agent_kind":"echo"}}"#;
        match serde_json::from_str::<Frame>(raw).unwrap() {
            Frame::Response {
                result: SessionResult::Spawned { provider, provider_cause, .. },
                ..
            } => {
                assert!(provider.is_none() && provider_cause.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_ack_can_carry_the_control_planes_router_endpoint() {
        // 告知：字段出现，节点据此把模型流量指回主控。
        let ack = accepted_with_router(
            None,
            Some("http://10.0.0.5:8787".into()),
            Some("router-token".into()),
        );
        let json = serde_json::to_string(&ack).unwrap();
        assert_eq!(serde_json::from_str::<HelloAck>(&json).unwrap(), ack);
        assert!(json.contains("router_url"), "{json}");

        // 不告知：字段不出现（节点必须如实拒绝，而不是猜一个地址）。
        let bare = serde_json::to_string(&accepted(None)).unwrap();
        assert!(!bare.contains("router"), "{bare}");
        // 旧主控的应答（完全没有这两个字段）也必须能解析成 None。
        let legacy = r#"{"protocol_version":1,"outcome":{"result":"accepted"}}"#;
        let ack: HelloAck = serde_json::from_str(legacy).unwrap();
        assert!(ack.router_url.is_none() && ack.router_token.is_none());
    }

    #[test]
    fn unknown_reject_codes_are_tolerated_instead_of_breaking_the_frame() {
        // 对端比本端新：未知码归为 `Unknown` 而不是解析失败（协议演进的基本盘）。
        assert_eq!(
            serde_json::from_str::<RejectCode>("\"some_future_code\"").unwrap(),
            RejectCode::Unknown
        );
        assert_eq!(
            serde_json::from_str::<SessionRejectCode>("\"some_future_code\"").unwrap(),
            SessionRejectCode::Unknown
        );
        // 已知码不受影响。
        assert_eq!(
            serde_json::from_str::<SessionRejectCode>("\"unknown_session\"").unwrap(),
            SessionRejectCode::UnknownSession
        );
        assert!(SessionRejectCode::Unknown.is_permanent());
        assert!(RejectCode::Unknown.is_permanent());
    }
}
