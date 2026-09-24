//! 会话域：身份、状态视图与事件流的**形状**（add-domain-layer 3.1/3.2）。
//!
//! 类型自 `sebas-dispatch`（engine::events / state）与 `sebas-webui`
//! （session_backend）按 design D3 **原样搬迁**：字段、serde 属性、缺省
//! 语义逐字节不变；原 crate 以 `pub use` 原位再导出，调用点零改动。
//!
//! `SessionInfo` 是外部世界看到的一个会话：映射状态与 WebUI 渲染的卡片
//! 派生字段的连接视图。`SessionEvent` 是每次映射变更在广播通道上的事件。
//! `TurnEntry` 是会话 transcript 的一个渲染块，以单调 position 寻址。
//! 这些类型是 serde-native 的：它们以 NDJSON 跨 core session channel。
//!
//! 会话由扁平化的 [`ChannelKey`] 寻址：`channel` 是来源渠道名，`key` 是
//! core 不解释的渠道中立 reference。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use sebas_channels::card::AppUsage;
use sebas_channels::ChannelKey;

// 会话词汇的收敛定义（type-session-vocabularies）住在 [`crate::vocabulary`]；
// 这里原位再导出，`sebas_domain::session::*` 这条既有公开路径零改动。
pub use crate::vocabulary::{
    CardPhase, GateCategory, PermissionDecision, SessionMode, SessionPhase, TurnElementType,
    TurnKind,
};

/// 会话自广告的命令表（原 `sebas_acp::AvailableCommand`；type-session-vocabularies
/// 随「共享决策词汇」一并移入）。
///
/// **为什么搬**：`sebas-domain` 此前依赖 `sebas-acp`（仅为这一个结构），
/// 而本 change 要求唯一的审批决定类型定义在 domain 并被 ACP 取用——那条
/// 依赖边会造成 crate 循环。命令表本身是角色中立的会话视图概念，放在域层
/// 合理；`sebas_acp::AvailableCommand` 经 `pub use` 原位再导出，字段、serde
/// 属性与线形状逐字节不变。
///
/// 字段全部 `#[serde(default)]`：来源形状随 CLI/agent 版本漂移，缺字段反
/// 序列化为空值而不是报错（防御性映射的落点，上游映射函数保证不产生缺
/// name 的条目）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AvailableCommand {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub hint: Option<String>,
}

/// 控制面缺省 mode（session-parallel-liveness-and-unread-polish 3.2，design
/// D5b）：ask 是「每个受门控动作都要问」的确定性模式，不是「留给 agent 自己
/// 猜」。desired_mode 在内存/wire 模型中非空，旧数据（state.json 的 null）
/// 在 restore 反序列化点一次性落为 ask——迁移是 null 消失的唯一地点，之后
/// 任何投影都读到这个值，无读侧回退。
///
/// （type-session-vocabularies 2.4）`ASK_MODE` 作为字符串常量的**唯一**用途
/// 是给尚未类型化的持久行与展示默认值站岗；线/内存模型已改用
/// [`SessionMode`]。新代码请用 [`SessionMode::Ask`]。
pub const ASK_MODE: &str = "ask";

/// [`ASK_MODE`] 的 serde 缺省构造器（`#[serde(default = …)]` 形态要求
/// 同签名的函数），也是内存/线模型的缺省 mode。
///
/// （type-session-vocabularies 2.4）返回**类型化**的 [`SessionMode`]：字符串
/// 常量 [`ASK_MODE`] 只留给磁盘行（projects.db 的 `desired_mode TEXT`）站岗。
pub fn ask_mode() -> SessionMode {
    SessionMode::Ask
}

/// [`SessionMode`] 形态的缺省构造器（`#[serde(default = …)]`）。
pub fn ask_session_mode() -> SessionMode {
    SessionMode::Ask
}

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
    /// （type-session-vocabularies 2.4）会话**期望**的 mode：控制面词汇
    /// `ask` / `edit` / `allow` / `auto`，类型化为共享 [`SessionMode`]。
    /// 未知 mode 从对端进来时落到 [`SessionMode::Unknown`]（容忍，不拒收）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_mode: Option<SessionMode>,
    /// 执行体**实际强制**的 mode。与 `desired_mode` 不同即「执行体强制不了」，
    /// 展示层必须两个都显示并说明差异（不能只显示期望值假装已生效）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_mode: Option<SessionMode>,
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
    /// （type-session-vocabularies 2.2）会话相位：控制面与节点的**同一**共享
    /// 定义 [`SessionPhase`]。控制面只发 `spawning` / `active` / `dormant` /
    /// `spawn-failed` 四个值；其余并集取值属于节点侧。对端发来未知相位时落到
    /// [`SessionPhase::Unknown`]，不丢帧。
    pub status: SessionPhase,
    /// Card phase emoji (`Get`/`OnIt`/`DONE`/`CrossMark`) when a card exists.
    ///
    /// （type-session-vocabularies 2.3）类型化为 [`CardPhase`]，使展示层
    /// `SessionStatus::derive` 的映射成为类型化 `match`（漏一个取值即编译失败）
    /// 而不是字符串比较。
    pub phase: Option<CardPhase>,
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
    pub usage: Option<AppUsage>,
    /// （wire-webui-sebas-agent-e2e D4）会话所属执行体：`"acp"` / `"native"`，
    /// 由复合后端在快照/事件中转时打标。`#[serde(default)]` 兼容旧报文
    /// （缺字段 = 未打标，展示层回退到 agent_kind / 默认执行体）。
    #[serde(default)]
    pub backend: Option<String>,
    /// （workbench-turn-queue D6）待生效提交全量视图（投递序，staging 先于
    /// turn 队列）。快照与每次会话事件都携带；`#[serde(default)]` 兼容旧
    /// 快照/旧事件。
    #[serde(default)]
    pub pending: Vec<PendingSubmission>,
    /// （add-remote-execution-node 8.x）远端会话的节点/mode/悬空审批呈现信息；
    /// `None` = 主控本机会话（这些维度对它不存在）。`#[serde(default)]` 兼容旧
    /// 报文：旧客户端收不到该键，新客户端收到 `null` 时按本机会话处理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteSessionView>,
    /// （type-session-vocabularies 2.4）操作者期望的会话 mode：控制面词汇
    /// `ask`/`edit`/`allow`/`auto`，类型化后与节点链路、web UI 请求面共用
    /// [`SessionMode`]。缺省 `ask`；旧 core-channel 报文里的 null 在反序列化点
    /// 落为 ask（与 state.json restore 同一迁移语义），wire 上永远携带确定词。
    /// 对端发来本 build 不认识的 mode 时落到 [`SessionMode::Unknown`] 而非报错。
    #[serde(
        default = "ask_session_mode",
        deserialize_with = "deserialize_desired_mode"
    )]
    pub desired_mode: SessionMode,
    /// （add-agent-mode-selection）执行体回报的**实际生效** mode（本机 =
    /// spawn argv 应用值 / `ModeChanged`；远端 = 节点回报，见 `remote`）。
    /// `None` = 执行体未声称任何 mode 生效——desired/effective 的差异如实
    /// 可见（execution-node spec："mode enforceability is declared, not
    /// assumed"）。`#[serde(default)]` 兼容旧快照/旧事件。
    #[serde(default)]
    pub effective_mode: Option<SessionMode>,
    /// （rail-declutter-unread D1/D2）会话累计「可见回复段」数——rail 未读
    /// 徽标的服务端数据源。口径见 dispatch 侧 `count_chat_messages`；随
    /// `session.updated` 广播（transcript flush 多数时机不发事件，rail 的
    /// 10s 轮询兜底）。`#[serde(default)]` 兼容旧快照/旧事件。
    #[serde(default)]
    pub msg_count: u64,
    /// （session-slash-commands 2.1/2.2）agent 自广告的会话命令表（claude
    /// `get_server_info()` 握手 / 通用 ACP `available_commands_update`）。
    /// 空 = 无命令面板（native 等无发现能力会话恒空——诚实退化非错误）。
    /// `#[serde(default, skip_serializing_if)]` 双向兼容：旧 core 的报文无
    /// 此键 → 反序列化为空表；本侧表空 → 键不上 wire（与旧前端/旧消费端
    /// 的 wire 形状一致）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_commands: Vec<AvailableCommand>,
    /// （fix-pending-queue-liveness 2.3，design D3）**回合占用**的引擎事实：
    /// WORKING 相位 ∨ 泊车审批在等 ∨ spawn 窗口。呈现层据此驱动「turn 在飞」
    /// 判定（提交控件排队/停止形态），不再猜展示词 slug——`waiting` 等呈现
    /// 词只服务徽标配色。`#[serde(default)]` 兼容旧快照/旧事件；序列化只在
    /// `true` 时上 wire（webui 投影层职责），缺省 = 旧前端的
    /// `status_slug === 'working'` 回退判定（Migration Plan）。
    #[serde(default)]
    pub turn_engaged: bool,
    /// （session-parallel-liveness-and-unread-polish 1.3）spawn 失败的原因
    /// 原文（`MappingState::SpawnFailed { reason }` 的透传，不新造状态机）。
    /// `None` = 非 spawn-failed 会话。webui 投影层据此在会话行/详情/composer
    /// 就地呈现失败原因（session-lifecycle delta「failed spawn names the
    /// cause」）；`skip_serializing_if` 让非失败会话的报文保持干净。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_failure_reason: Option<String>,
    /// （review 3c 补口，session-unread-badge delta「status_slug = (MappingState,
    /// phase, parked approvals) 的投影」）**本地**会话当前悬停的泊车审批数
    /// （`StallRegistry.parked_count` 投影）。此前该维度只随 remote 视图下发，
    /// 本地泊车只进了 `turn_engaged`，`waiting` 投影对本地会话永远不亮。
    /// 类型与 remote 视图的 `parked_approvals`（u32）对齐。
    /// `#[serde(default)]` 兼容旧快照/旧事件。
    #[serde(default)]
    pub parked_approvals: u32,
    /// （fix-webui-approval-restore-and-session-identity 5.1，design D6）操作者
    /// 设置的会话 label。`None` = 未设置（命名回退首条 prompt 预览 / 短 id，
    /// 行为与旧版本完全一致——只影响「设置了 label」的会话）。
    /// `#[serde(default, skip_serializing_if)]` 兼容旧快照/旧事件，None 不上 wire。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// （3.2，design D5b）`desired_mode` 的反序列化兼容：旧 core-channel 报文
/// / 旧快照里该字段是显式 `null`（Option 时代形态）或缺失——一律落为
/// `ask`。与 state.json 的 restore 迁移同一语义（迁移是 null 消失的唯一
/// 地点），不做读侧投影。
///
/// （type-session-vocabularies 2.4）值改为 [`SessionMode`]：未知 mode 走宽容的
/// [`SessionMode::from_wire`]（落到 `Unknown`），**不**因不认识而让整帧失败。
fn deserialize_desired_mode<'de, D>(d: D) -> Result<SessionMode, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<SessionMode> = serde::Deserialize::deserialize(d)?;
    Ok(opt.unwrap_or(SessionMode::Ask))
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
        dropped: Vec<PendingSubmission>,
    },
    /// （fix-pending-queue-liveness 2.2，design D5）回合停滞被看门狗强制
    /// 收尾：点名会话与释放的搁浅待执行提交数。分级通知的低档（warn）由
    /// 呈现层据此就地呈现——引擎不关心通知形状，只发事实。
    TurnStalled {
        channel: String,
        key: String,
        released: usize,
    },
    /// Emitted by channel clients (never by the router itself) after a
    /// reconnect: subscribers should re-snapshot because the client resumed
    /// from a fresh snapshot and the view must converge. See the channel
    /// spec's "reconnect resumes from a snapshot" scenario.
    Resync,
}

/// One rendered block of a session's transcript, addressed by a monotonic
/// position. `kind` distinguishes the user's prompt from agent/tool output;
/// `element_type` tells the client how to render `content`.
///
/// （type-session-vocabularies 4.2）两个字段都从裸 `String` 收敛为共享封闭
/// 词汇 [`TurnKind`] / [`TurnElementType`]：新增一个取值会让每个分支它的
/// 消费方**编译失败**，而不是静默落到默认分支。线形状不变（裸字符串拼写逐字
/// 保留）；本 build 不认识的取值落到各自的 `Unknown`，**照原样传递并渲染为
/// 通用块**，不丢弃、不报错。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnEntry {
    /// 0-based monotonic position within the session's transcript.
    pub position: u64,
    /// [`TurnKind::Prompt`] (user turn input) or [`TurnKind::Content`]
    /// (agent/tool output).
    pub kind: TurnKind,
    /// [`TurnElementType::Markdown`] | `Thinking` | `Tool` | `Error` |
    /// `PermissionModeResult`（见 [`TurnEntry::permission_mode_result`]）；
    /// `Notice` 见 [`TurnEntry::notice`]。
    pub element_type: TurnElementType,
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
    /// （fix-webui-qa-defects 5.1/5.2，design D5）错误条目的失败分类：
    /// `"spawn"`（spawn 失败）| `"stall"`（回合停滞强收）| `"generic"`
    /// （回合终态错误，含 refusal）。前端气泡标签据此如实分类渲染，不再
    /// 一律写死「spawn failed」。仅 `element_type = "error"` 的条目携带；
    /// `None` = 旧条目（前端回退中性「错误」标签）。serde 缺省兼容旧
    /// archive.json / turn 快照。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
}

/// 实时回合内容事件（workbench-live-conversation-flow 1.1）：transcript
/// 每追加一批条目就发布一条，core 通道以 `SessionStreamFrame::Turn` 帧转
/// 发、webui 以 `turn.append` WS 事件转播。`entries` 是同一 250ms 合并窗
/// 内该会话追加的条目（按落库顺序、position 单调）；日志仍是唯一事实——
/// 合并只影响传输分帧，消费端以快照重取收敛。与 [`SessionEvent::PendingDropped`]
/// 同构的 `(channel, key)` 寻址。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnStreamEvent {
    pub channel: String,
    pub key: String,
    pub entries: Vec<TurnEntry>,
}

/// 零输出回合合成提示的固定文案（close-acceptance-blind-spots 4.1）：
/// 说明「回合已结束且无输出」，措辞与停滞强收条目同风格（加粗导语 + 冒号
/// 说明）。
///
/// 唯一定义在域层（extend-test-model-scenarios 3.4）：ACP/IM 引擎面
/// （`sebas-dispatch`）与原生内核转录面（根 crate `agent_backend`）都要在
/// 「回合正常结束但零可见输出」时追加同一条提示，文案漂移会让两侧用户面的
/// 呈现无法对账。
pub const ZERO_OUTPUT_NOTICE: &str =
    "**回合已结束且无输出**：本轮回合未产生任何可见输出（正文、thinking、工具、错误皆无）。";

impl TurnEntry {
    pub fn prompt(position: u64, content: impl Into<String>) -> Self {
        Self::new(
            position,
            TurnKind::Prompt,
            TurnElementType::Markdown,
            content,
        )
    }

    pub fn markdown(position: u64, content: impl Into<String>) -> Self {
        Self::new(
            position,
            TurnKind::Content,
            TurnElementType::Markdown,
            content,
        )
    }

    pub fn thinking(position: u64, content: impl Into<String>) -> Self {
        Self::new(
            position,
            TurnKind::Content,
            TurnElementType::Thinking,
            content,
        )
    }

    /// 工具调用条目（workbench-conversation-view 1.3，design D2）：内容仍是
    /// 可读 markdown，但 `element_type = "tool"` 让客户端能把工具调用与正文
    /// 区分开（收进「用了 N 个工具」可展开组），不再靠 emoji 前缀当契约。
    pub fn tool(position: u64, content: impl Into<String>) -> Self {
        Self::new(position, TurnKind::Content, TurnElementType::Tool, content)
    }

    /// spawn 失败等启动期错误条目（fail-fast-on-startup-errors 3.1）：
    /// `kind = "content"`（core 产生，非操作员提交）+ `element_type = "error"`，
    /// 前端据此渲染为带计数的错误气泡而非普通 markdown。（kind 词汇收敛为
    /// prompt|content 两值是 workbench-conversation-view 的 delta 契约。）
    pub fn error(position: u64, content: impl Into<String>) -> Self {
        Self::new(
            position,
            TurnKind::Content,
            TurnElementType::Error,
            content,
        )
    }

    /// 零输出回合的合成提示条目（close-acceptance-blind-spots 4.1，design
    /// D3）：真实回合正常结束但未产生任何可见输出（正文、thinking、工具、
    /// 错误皆无——如 claude 对未知命令零输出结束回合）时，引擎在回合收尾点
    /// 追加一条本类型条目，回合在时间线上可见、不再不可见地消失。
    ///
    /// `kind = "content"` + `element_type = "notice"`：**不复用 `error`**——
    /// 那会触发失败语义（前端红泡、failure_class 分类），而零输出不是失败；
    /// 也不复用 `markdown` 加约定文案——语义不可区分、测试无法精确断言。
    /// 前端渲染为中性信息条（非错误红泡）。不计入可见回复段数
    /// （`count_chat_messages` 与前端 `unitSegmentCount` 都跳过 notice）。
    pub fn notice(position: u64, content: impl Into<String>) -> Self {
        Self::new(
            position,
            TurnKind::Content,
            TurnElementType::Notice,
            content,
        )
    }

    /// 权限卡「本会话不再询问」触发的自动模式切换结果条目
    /// （permission-mode-auto-gate 事件契约）。
    ///
    /// wire 形状（上游 e2e 依赖，勿随意改动）：
    /// - `kind = "content"`、`element_type = "permission_mode_result"`；
    /// - `content` = JSON 载荷 `{ "request_id": String, "ok": bool,
    ///   "mode": String, "detail": String }`；
    /// - 语义：`ok=false` = 该 request_id 的权限卡点击后，SetMode(auto) 被
    ///   执行体拒绝/不可达——**当前调用已放行（不回滚）**，detached 前端
    ///   （im）应把对应权限卡翻成如实失败态；`ok=true` 预留给显式成功上报，
    ///   当前不发（成功面=点击时的卡面翻转）。
    pub fn permission_mode_result(position: u64, payload: serde_json::Value) -> Self {
        Self::new(
            position,
            TurnKind::Content,
            TurnElementType::PermissionModeResult,
            payload.to_string(),
        )
    }

    fn new(
        position: u64,
        kind: TurnKind,
        element_type: TurnElementType,
        content: impl Into<String>,
    ) -> Self {
        Self {
            position,
            kind,
            element_type,
            content: content.into(),
            // The router stamps the wall-clock at push time so every
            // entry carries the moment it was appended, not the moment
            // the helper was called.（时钟原语唯一实现在
            // `sebas_domain::prim::now_unix`——add-domain-layer 2.5。）
            created_at_unix: u64::try_from(crate::prim::now_unix()).unwrap_or(0),
            title: None,
            failure_class: None,
        }
    }

    /// 附加结构化标题（workbench-agent-identity-and-process-folds 1.2）：
    /// 仅工具条目使用，链在 [`TurnEntry::tool`] 之后；其余条目保持 `None`。
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// 附加失败分类（fix-webui-qa-defects 5.1，design D5）：仅错误条目使用，
    /// 链在 [`TurnEntry::error`] 之后。分类词表见 [`TurnEntry`] 的
    /// `failure_class` 字段文档。
    pub fn with_failure_class(mut self, class: impl Into<String>) -> Self {
        self.failure_class = Some(class.into());
        self
    }
}

/// 一条待批权限请求的读模型行（fix-webui-approval-restore-and-session-identity
/// 1.1，design D1）：按会话枚举当前泊车审批（request_id / 工具 / 参数）。
/// 与推送通道独立——WebUI 打开/刷新会话时主动拉取它重建审批面，与 WS
/// `permission.requested` 按 `request_id` 幂等合并。serde-native：跨 core
/// session channel 传输。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingApproval {
    /// 权限请求 id（agent-driver 命名空间，如 `claude:tc-N`）。
    pub request_id: String,
    /// 被门控的工具名。
    pub tool_name: String,
    /// 工具调用参数（原样 JSON）。
    pub args: Value,
}

/// 处置（workbench-turn-queue D1）：`staging` = 并入首条消息；`turn` = 按序
/// 执行的待执行回合。serde 形状随 `SessionInfo` 上 wire。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingDisposition {
    Staging,
    Turn,
}

/// pending submission 的观察视图（workbench-turn-queue design D1）：core 已
/// 接受、尚未开始执行的一次提交。`position` 是投递序里的下标（staging 先于
/// turn 队列），每次构建视图时重算。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingSubmission {
    pub id: u64,
    pub text: String,
    pub position: usize,
    pub disposition: PendingDisposition,
    pub priority: bool,
}

/// 会话身份（fix-webui-approval-restore-and-session-identity 3.1/3.2，design
/// D3）：归档条目携带、恢复重建时原样带回的四项。全 `Option`——旧归档条目
/// 缺字段时如实回退既有默认（agent 显示回退、模型目录清空），不做数据迁移。
/// serde-native：跨归档文件与 core session channel 传输。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionIdentity {
    /// 创建时绑定的执行后端 kind（`[acp.agents.*]` 配置键）；`None` = 配置默认。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,
    /// 归档时刻的期望 mode（控制面词汇 ask/edit/allow/auto）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_mode: Option<String>,
    /// 归档时刻的当前模型 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_model: Option<String>,
    /// 归档时刻的可选模型目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_models: Option<Vec<String>>,
}

impl SessionIdentity {
    /// 全空 = 旧归档条目（恢复路径维持现默认，wire 上可省键）。
    pub fn is_empty(&self) -> bool {
        self.agent_kind.is_none()
            && self.desired_mode.is_none()
            && self.current_model.is_none()
            && self.available_models.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 形状钉：SessionInfo 序列化形状与搬迁前一致（黄金样本同源）。
    #[test]
    fn session_info_wire_shape_is_unchanged() {
        let info = SessionInfo {
            channel: "web".into(),
            key: "web-1".into(),
            session_id: Some("s1".into()),
            status: SessionPhase::Active,
            phase: None,
            user_prompt: None,
            last_active_unix: 7,
            project_dir: None,
            current_model: None,
            available_models: None,
            agent_kind: None,
            usage: None,
            backend: Some("acp".into()),
            pending: vec![],
            remote: None,
            desired_mode: SessionMode::Ask,
            effective_mode: None,
            msg_count: 0,
            available_commands: vec![],
            turn_engaged: false,
            spawn_failure_reason: None,
            parked_approvals: 0,
            label: None,
        };
        let v = serde_json::to_value(&info).unwrap();
        assert_eq!(v["channel"], "web");
        assert_eq!(v["desired_mode"], "ask");
        assert_eq!(v["backend"], "acp");
        // skip_serializing_if 的字段不上 wire。
        assert!(v.get("remote").is_none());
        assert!(v.get("label").is_none());
        assert!(v.get("available_commands").is_none());
        // 旧报文（desired_mode = null）→ ask 迁移语义。
        let legacy: SessionInfo = serde_json::from_str(
            r#"{"channel":"web","key":"k","status":"spawning","last_active_unix":0,"desired_mode":null}"#,
        )
        .unwrap();
        assert_eq!(legacy.desired_mode, SessionMode::Ask);
    }

    #[test]
    fn session_event_tagged_round_trip() {
        let e = SessionEvent::TurnStalled {
            channel: "web".into(),
            key: "k".into(),
            released: 2,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "turn_stalled");
        let back: SessionEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn pending_submission_disposition_snake_case() {
        let p = PendingSubmission {
            id: 1,
            text: "t".into(),
            position: 0,
            disposition: PendingDisposition::Turn,
            priority: false,
        };
        assert_eq!(serde_json::to_value(&p).unwrap()["disposition"], "turn");
    }

    #[test]
    fn turn_entry_constructors_set_wire_fields() {
        let t = TurnEntry::tool(3, "📖 x").with_title("Read · a");
        assert_eq!(t.element_type, TurnElementType::Tool);
        assert_eq!(t.title.as_deref(), Some("Read · a"));
        let e = TurnEntry::error(4, "boom").with_failure_class("spawn");
        assert_eq!(e.failure_class.as_deref(), Some("spawn"));
        let n = TurnEntry::notice(5, "no output");
        assert_eq!(n.element_type, TurnElementType::Notice);
        assert!(n.failure_class.is_none());
    }

    #[test]
    fn session_identity_skips_empty_options_on_wire() {
        let id = SessionIdentity::default();
        assert!(id.is_empty());
        assert_eq!(serde_json::to_value(&id).unwrap(), serde_json::json!({}));
    }
}

/// Typed rejection for a session mutation（原 `sebas_webui::session_backend`
/// 定义，add-domain-layer 3.2 原位再导出；spec: rejections name the reason;
/// nothing is mutated on rejection）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum SessionRejection {
    /// No session exists for the given key.
    UnknownSession { key: String },
    /// The requested project directory is not a usable directory.
    /// Deliberately carries no path details — no existence disclosure.
    UnusableProjectDir,
    /// The core is at its session capacity.
    Capacity { limit: usize },
    /// The request could not be delivered to the session authority, or the
    /// targeted surface is not hosted by this backend.（fix-webui-qa-defects
    /// -round5 1.3）Display 文案如实说「操作不可用」——「核心不可达」的说法
    /// 只保留给 core.reachability 真实可达性信号。
    Unavailable { cause: String },
    /// The targeted execution backend cannot serve the request even though
    /// the core is reachable (e.g. native without provider credentials), or
    /// the caller named a backend hint the core does not know.
    BackendUnavailable { backend: String, cause: String },
    /// （workbench-turn-queue 5.1，design D5）spawn 窗口 staging 队列已满：
    /// 提交未被接受、也不顶掉已暂存条目。携带上限，提交面据此可见拒绝。
    QueueFull { limit: usize },
    /// （workbench-turn-queue D7）pending submission 管理操作的类型化拒绝。
    PendingRejected { reason: PendingReason },
    /// （workbench-interaction-polish 1.1）会话存在但没有在飞 turn——取消
    /// 无从谈起，如实拒绝而非伪造成功。`key` 是编码会话键（诊断用）。
    #[serde(rename = "idle_session")]
    Idle { key: String },
    /// 对端发来本 build 不认识的拒绝码（对端比本端新）。
    ///
    /// `#[serde(other)]`（unify-ipc-protocol-home 6.1）：容忍未知码而不是让
    /// **整个响应帧**解析失败——旧客户端不该因为新 core 多了一种拒绝理由就
    /// 连"被拒绝了"这件事都读不出来。语义按「操作不可用」呈现，绝不假装成功。
    #[serde(other)]
    Unknown,
}

/// pending submission 管理拒绝的具体原因（workbench-turn-queue D7）。
/// 随 [`SessionRejection::PendingRejected`] 迁入（变体载荷与宿主同迁）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingReason {
    /// id 不在待执行栈里，也从未开始过。
    Unknown,
    /// 该提交已经开轮（或已被激活合并）——绝不回滚在跑的回合。
    AlreadyStarted,
    /// 不能把普通提交移到优先（/btw）提交之前。
    PriorityConflict,
    /// 目标位置超出该处置组的范围。
    OutOfRange,
}

impl std::fmt::Display for PendingReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PendingReason::Unknown => write!(f, "待执行提交不存在"),
            PendingReason::AlreadyStarted => write!(f, "该提交已开始执行"),
            PendingReason::PriorityConflict => write!(f, "不能越过优先提交排序"),
            PendingReason::OutOfRange => write!(f, "目标位置越界"),
        }
    }
}

impl std::fmt::Display for SessionRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionRejection::UnknownSession { key } => write!(f, "会话不存在: {key}"),
            SessionRejection::UnusableProjectDir => {
                write!(f, "项目目录不可用（不是目录或无法访问）")
            }
            SessionRejection::Capacity { limit } => write!(f, "会话数已达上限 {limit}"),
            // fix-webui-qa-defects-round5 1.3：Unavailable 的文案不再自称
            // 「核心不可达」——该变体同时承载真实可达性失败（core 通道客户端）
            // 与语义不可用（不承载队列/注册表/远端放置），后者在 core 可达时
            // 报「核心不可达」是误导 QA 实锤的文案。「核心不可达」的说法只
            // 保留给 core.reachability 真实可达性信号（前端 fatal 横幅）。
            SessionRejection::Unavailable { cause } => write!(f, "操作不可用: {cause}"),
            SessionRejection::BackendUnavailable { backend, cause } => {
                write!(f, "执行体不可用: {backend} — {cause}")
            }
            SessionRejection::QueueFull { limit } => {
                write!(f, "待执行队列已满（上限 {limit}）：这条消息没有提交")
            }
            SessionRejection::PendingRejected { reason } => write!(f, "{reason}"),
            SessionRejection::Idle { key } => {
                write!(f, "会话空闲（无在飞回复，无需取消）: {key}")
            }
            SessionRejection::Unknown => write!(
                f,
                "核心拒绝了这次操作，但拒绝理由是当前版本不认识的（核心比界面新）"
            ),
        }
    }
}

/// One gated tool call awaiting an operator decision (webui review card).
/// `session_id` is the encoded session key; `request_id` equals the kernel's
/// `tool_use_id` and is what the backend's `answer_permission` takes back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionNotice {
    pub request_id: String,
    /// Encoded session key (URL-safe, as used in routes).
    pub session_id: String,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub reason: String,
}

/// The operator's answer to a [`PermissionNotice`].
///
/// （type-session-vocabularies 3.1）定义已收敛到 [`crate::vocabulary::PermissionDecision`]
/// ——唯一共享定义（四值 + 未知值路径），此处经文件顶部的 `pub use` 再导出，
/// `sebas_domain::session::PermissionDecision` 这条既有公开路径不变。

#[cfg(test)]
mod webui_wire_tests {
    use super::*;

    /// 形状钉：tag/code 词表与旧 webui 定义逐字节一致（add-domain-layer 3.2）。
    #[test]
    fn session_rejection_code_tags_are_pinned() {
        let cases = [
            (SessionRejection::UnknownSession { key: "k".into() }, "unknown_session"),
            (SessionRejection::UnusableProjectDir, "unusable_project_dir"),
            (SessionRejection::Capacity { limit: 1 }, "capacity"),
            (SessionRejection::Unavailable { cause: "c".into() }, "unavailable"),
            (
                SessionRejection::BackendUnavailable { backend: "b".into(), cause: "c".into() },
                "backend_unavailable",
            ),
            (SessionRejection::QueueFull { limit: 2 }, "queue_full"),
            (
                SessionRejection::PendingRejected { reason: PendingReason::Unknown },
                "pending_rejected",
            ),
            (SessionRejection::Idle { key: "k".into() }, "idle_session"),
        ];
        for (rejection, tag) in cases {
            assert_eq!(serde_json::to_value(&rejection).unwrap()["code"], tag);
        }
        assert_eq!(
            serde_json::to_value(&PendingReason::PriorityConflict).unwrap(),
            "priority_conflict"
        );
    }

    #[test]
    fn permission_decision_tags_are_pinned() {
        assert_eq!(
            serde_json::to_value(&PermissionDecision::AllowOnce).unwrap(),
            serde_json::json!({"decision": "allow_once"})
        );
        assert_eq!(
            serde_json::to_value(&PermissionDecision::Escalate { reason: "r".into() }).unwrap(),
            serde_json::json!({"decision": "escalate", "reason": "r"})
        );
    }

    #[test]
    fn display_texts_survive_the_move() {
        assert_eq!(
            SessionRejection::Idle { key: "k".into() }.to_string(),
            "会话空闲（无在飞回复，无需取消）: k"
        );
        assert_eq!(PendingReason::OutOfRange.to_string(), "目标位置越界");
    }
}
