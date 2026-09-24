//! 会话词汇的收敛定义（type-session-vocabularies）。
//!
//! 本模块把散落各 crate 的会话状态/模式/决策/回合内容词汇收敛为**单一定义**：
//!
//! - [`SessionPhase`]：控制面与执行节点的会话相位并集（各自只发自己的子集）；
//! - [`CardPhase`]：卡片 reaction 相位（飞书 `emoji_type` 词汇）；
//! - [`SessionMode`] + [`GateCategory`]：会话模式与门控粒度（原 `sebas-node-link`）；
//! - [`TurnKind`] / [`TurnElementType`]：回合条目的种类与元素类型；
//! - [`PermissionDecision`]：跨驱动审批决定（原 acp / webui / native / agent /
//!   node-link 五份并行枚举的唯一替代）。
//!
//! # 零变化基线（本 change 的红线）
//!
//! 每个取值的**线拼写逐字不变**（含 `"spawn-failed"` 的连字符与
//! `"permission_mode_result"` 的下划线），JSON 字段名与帧形状不变。
//!
//! # 前向容错
//!
//! 全部词汇都带未知值路径（[`wire_string_enum!`] 生成的 `Unknown(String)`，
//! 或 [`PermissionDecision::Unknown`]）：对端发来本 build 不认识的取值时，
//! **不报错、不丢帧**，而是标记为未知并**原样保留拼写**（序列化时回吐原串），
//! 使未知值可以继续被传递与诊断。先例见 `sebas-node-link` 的 `RejectCode`。
//!
//! 例外是「决定」：未知决定**不得**静默解除一条泊车审批——消费方必须先
//! 用 [`PermissionDecision::is_unknown`] 把它挡下（spec `agent-driver`
//! 「no parked approval is silently resolved」）。

use serde::{Deserialize, Serialize};

/// 生成「裸字符串 + 未知值路径」的封闭词汇枚举。
///
/// 线格式是**裸的 JSON 字符串**（`"active"`），不是标签信封——这是这些字段
/// 今天的形状，必须逐字保留。未知取值经 [`from_wire`] 落到 `Unknown(String)`
/// 并保留原串，序列化时原样回吐（round-trip 保真）。
macro_rules! wire_string_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident => $lit:literal
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        $vis enum $name {
            $(
                $(#[$vmeta])*
                $variant,
            )+
            /// 本 build 不认识的取值：保留对端原样拼写（前向容错，见模块文档）。
            Unknown(String),
        }

        impl $name {
            /// 线格式拼写（未知取值回吐原串）。
            pub fn as_str(&self) -> &str {
                match self {
                    $( Self::$variant => $lit, )+
                    Self::Unknown(raw) => raw.as_str(),
                }
            }

            /// 由线格式取值构造；不认识的取值 → [`Self::Unknown`]（**不报错**）。
            pub fn from_wire(raw: &str) -> Self {
                match raw {
                    $( $lit => Self::$variant, )+
                    other => Self::Unknown(other.to_string()),
                }
            }

            /// 是否为未知取值。
            pub fn is_unknown(&self) -> bool {
                matches!(self, Self::Unknown(_))
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = ::std::convert::Infallible;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self::from_wire(s))
            }
        }

        /// 由线格式取值构造（[`Self::from_wire`] 的 `From` 形态）。
        ///
        /// 与 `from_wire` **同语义**：不认识的取值落到 `Unknown`，不报错、不 panic
        /// ——这个词汇的容错性是 spec 的硬要求（未知值不得丢帧），因此「线值 → 类型」
        /// 的构造本来就是不可失败的，用 `From` 表达它符合 Rust 惯例，也让夹具/边界
        /// 代码少一层噪音。它**不**改变任何线拼写，也**不**削弱编译器闸门：对字面量
        /// 取值做 `match` 仍然必须穷尽全部变体。
        impl From<&str> for $name {
            fn from(raw: &str) -> Self {
                Self::from_wire(raw)
            }
        }

        /// 见 [`From<&str>`] 的说明。
        impl From<String> for $name {
            fn from(raw: String) -> Self {
                Self::from_wire(raw.as_str())
            }
        }

        /// 见 [`From<&str>`] 的说明。
        impl From<&String> for $name {
            fn from(raw: &String) -> Self {
                Self::from_wire(raw.as_str())
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let raw = <::std::borrow::Cow<'de, str>>::deserialize(d)?;
                Ok(Self::from_wire(raw.as_ref()))
            }
        }
    };
}

wire_string_enum! {
    /// 会话相位：**控制面与执行节点的取值并集**（design D1）。
    ///
    /// 一个类型而不是两侧各一个——两侧之间因此不需要转换函数（正是本 change
    /// 要消灭的漂移面）。类型不强制「角色只发自己的子集」，靠生产端约定：
    ///
    /// - 控制面 `SessionInfo.status` 只发 [`Self::Spawning`] / [`Self::Active`] /
    ///   [`Self::Dormant`] / [`Self::SpawnFailed`]；
    /// - 执行节点只发 [`Self::Spawning`] / [`Self::Active`] / [`Self::Idle`] /
    ///   [`Self::WaitingApproval`] / [`Self::Exited`] / [`Self::Closed`] /
    ///   [`Self::Terminated`]（[`Self::Failed`] 是既有终结判据里认得的取值）。
    ///
    /// `"spawn-failed"` 的连字符是线契约的一部分，不得改成下划线。
    pub enum SessionPhase {
        /// 正在启动（占位已建立、执行体尚未就绪）。
        Spawning => "spawning",
        /// 活着且可寻址。
        Active => "active",
        /// 活着但无在飞回复（控制面词汇）。
        Dormant => "dormant",
        /// 启动失败（控制面词汇；连字符是契约）。
        SpawnFailed => "spawn-failed",
        /// 活着、空闲待命（节点词汇）。
        Idle => "idle",
        /// 停在一条待批审批上（节点词汇）。
        WaitingApproval => "waiting_approval",
        /// 执行体已退出（节点词汇）。
        Exited => "exited",
        /// 会话已正常关闭（节点词汇）。
        Closed => "closed",
        /// 已被终止（节点词汇，含「节点侧已不存在」）。
        Terminated => "terminated",
        /// 失败的终态（节点既有终结判据认得的取值）。
        Failed => "failed",
    }
}

impl SessionPhase {
    /// 该相位是否意味着「会话已经结束」。
    ///
    /// 与既有 `is_terminal_phase` 判据逐字一致；未知取值**保守判为非终结**
    /// （与今天的字符串比较同一行为，也避免把不认识的新相位误判成结束）。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Terminated | Self::Closed | Self::Exited | Self::Failed
        )
    }
}

wire_string_enum! {
    /// 卡片 reaction 相位（飞书 `emoji_type` 词汇，大小写敏感）。
    ///
    /// 这些值直接是飞书 reaction API 的 `emoji_type`（任意 Unicode emoji 会被
    /// 拒绝，故只能用这套词表）；它们会作为 reaction 贴到根卡片上表达会话状态。
    /// 取值集与 `sebas_dispatch::card_state::phase` 的常量一一对应。
    pub enum CardPhase {
        /// 已收到（`card_state::phase::SEED`）。
        Get => "Get",
        /// 正在产出（`card_state::phase::WORKING`）。
        OnIt => "OnIt",
        /// 回合正常结束（`card_state::phase::DONE`）。
        Done => "DONE",
        /// 回合以错误结束（`card_state::phase::FAILED`）。
        CrossMark => "CrossMark",
    }
}

wire_string_enum! {
    /// 会话模式（`desired` / `effective`）。
    ///
    /// 原定义住在 `sebas-node-link`（链路契约 crate）；domain 不该依赖链路契约
    /// （依赖方向反了），故定义移入本层，node-link 原位再导出（design D4），
    /// 其公开 API 与线拼写不变。
    pub enum SessionMode {
        /// 每个受门控的动作都要问（**缺省**；`auto` 永远不是缺省）。
        Ask => "ask",
        /// 编辑类动作放行，其它受门控动作仍要问。
        Edit => "edit",
        /// 受门控动作一律放行，但仍留审计。
        Allow => "allow",
        /// 完全不门控；必须留审计痕迹（谁在什么时候把它打开过）。
        Auto => "auto",
    }
}

impl Default for SessionMode {
    fn default() -> Self {
        Self::Ask
    }
}

impl SessionMode {
    /// 严格解析：**不认识的模式返回 `None`**（调用方据此如实拒绝，而不是悄悄
    /// 降级）。线反序列化走宽容的 [`Self::from_wire`]；本方法是**操作者输入与
    /// 节点边界**的判定入口，语义与搬迁前逐字一致。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ask" => Some(Self::Ask),
            "edit" => Some(Self::Edit),
            "allow" => Some(Self::Allow),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    /// 该模式是否对这类动作**直接放行、不问**。
    pub fn allows_without_asking(&self, category: GateCategory) -> bool {
        match self {
            Self::Ask => false,
            Self::Edit => matches!(category, GateCategory::Edit),
            Self::Allow | Self::Auto => true,
            // 未知模式**不放行**（fail closed）：一个不认识的模式绝不能等价于
            // 「全放行」，那是拿安全性换容错。
            Self::Unknown(_) => false,
        }
    }

    /// 该模式是否完全不门控（`auto` 是唯一一个）。
    pub fn is_ungated(&self) -> bool {
        matches!(self, Self::Auto)
    }
}

/// 动作类别：门控的粒度。节点只做粗分类，执行体可以更细。
///
/// 与 [`SessionMode`] 同批从 `sebas-node-link` 移入（`allows_without_asking`
/// 的入参需要与本类型同 crate 才能保留为固有方法），node-link 原位再导出。
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

wire_string_enum! {
    /// 回合条目的种类：操作员提交的输入，还是执行体产出的内容。
    pub enum TurnKind {
        /// 操作员提交的回合输入。
        Prompt => "prompt",
        /// 执行体产出的内容（正文 / thinking / 工具 / 错误 / 提示）。
        Content => "content",
    }
}

wire_string_enum! {
    /// 回合条目的元素类型：客户端据此决定 `content` 怎么渲染。
    pub enum TurnElementType {
        /// 可读 markdown 正文。
        Markdown => "markdown",
        /// 思维链增量。
        Thinking => "thinking",
        /// 工具调用（内容仍是可读 markdown，但客户端把它收进可展开组）。
        Tool => "tool",
        /// 启动期 / 回合终态错误（前端渲染为错误气泡）。
        Error => "error",
        /// 中性提示条（零输出回合的合成提示；**不复用 `error`**，零输出不是失败）。
        Notice => "notice",
        /// 权限卡自动模式切换的结果条目（wire 形状见 `TurnEntry::permission_mode_result`）。
        /// 下划线是契约的一部分。
        PermissionModeResult => "permission_mode_result",
    }
}

/// 跨驱动权限决定：**唯一**共享定义（`allow_once` / `allow_session` / `deny` /
/// `escalate`）。
///
/// `escalate` = 带操作者理由的一次性放行（会话策略本身不因此放宽）。没有
/// escalate 等价物的执行体（ACP、节点链路）把它降级为 `allow_once` 并记日志
/// ——这是层间**唯一**的语义适配（spec `agent-driver`）。
///
/// # 线形状
///
/// 与既有定义逐字一致：内部标签信封 `{"decision":"allow_once"}`；
/// `escalate` 额外带 `"reason"`。节点链路一侧用裸字符串（见 [`bare_decision`]）。
/// 未知标签 **不报错**：落到 [`Self::Unknown`] 并保留原标签（序列化回吐）。
///
/// # 未知决定不得静默解除审批
///
/// spec 明写：无法解释的决定不得静默解决一条泊车审批。消费方在解除之前必须
/// 用 [`Self::is_unknown`]（或 [`Self::is_answerable`]）挡下未知值。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PermissionDecision {
    /// 只放行这一次。
    AllowOnce,
    /// 本会话内放行同类动作。
    AllowSession,
    /// 拒绝。
    Deny,
    /// 带理由的一次性放行（原生内核专属；ACP / 节点链路降级为 `allow_once`）。
    Escalate {
        /// 操作者陈述的升级理由（驱动侧落日志；会话策略不变）。
        reason: String,
    },
    /// 本 build 不认识的标签：保留对端原样拼写。**不得**用它解除任何审批。
    Unknown(String),
}

impl PermissionDecision {
    /// 线格式标签（未知取值回吐原串）。
    pub fn as_str(&self) -> &str {
        match self {
            Self::AllowOnce => "allow_once",
            Self::AllowSession => "allow_session",
            Self::Deny => "deny",
            Self::Escalate { .. } => "escalate",
            Self::Unknown(raw) => raw.as_str(),
        }
    }

    /// 由线格式标签（+ 可选理由）构造；不认识的标签 → [`Self::Unknown`]。
    pub fn from_wire(tag: &str, reason: Option<String>) -> Self {
        match tag {
            "allow_once" => Self::AllowOnce,
            "allow_session" => Self::AllowSession,
            "deny" => Self::Deny,
            "escalate" => Self::Escalate {
                reason: reason.unwrap_or_default(),
            },
            other => Self::Unknown(other.to_string()),
        }
    }

    /// 是否为未知取值（**不得**据此解除审批）。
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// 这个决定是否可以被**兑现**成一次审批答复。
    ///
    /// 未知决定返回 `false`：spec `agent-driver` 要求「无法解释的决定不得静默
    /// 解决一条泊车审批」。其余四值都是可兑现的（`escalate` 在无等价物的边界
    /// 上降级为 `allow_once`）。
    pub fn is_answerable(&self) -> bool {
        !self.is_unknown()
    }

    /// `escalate` 的操作者理由（其余取值 `None`）。
    pub fn escalate_reason(&self) -> Option<&str> {
        match self {
            Self::Escalate { reason } => Some(reason.as_str()),
            _ => None,
        }
    }

    /// 降级到「没有 escalate 等价物」的执行体：`escalate` → `allow_once`。
    ///
    /// 返回是否发生了降级，调用方据此记日志（spec：降级必须被记录）。
    /// 未知决定**原样返回**（降级不是解释未知值的地方——消费方必须先用
    /// [`Self::is_answerable`] 挡下）。
    pub fn downgrade_without_escalate(&self) -> (Self, Option<&str>) {
        match self {
            Self::Escalate { reason } => (Self::AllowOnce, Some(reason.as_str())),
            other => (other.clone(), None),
        }
    }
}

impl Serialize for PermissionDecision {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let len = if matches!(self, Self::Escalate { .. }) {
            2
        } else {
            1
        };
        let mut map = s.serialize_map(Some(len))?;
        map.serialize_entry("decision", self.as_str())?;
        if let Self::Escalate { reason } = self {
            map.serialize_entry("reason", reason)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for PermissionDecision {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        /// 标签信封的读取形状：`decision` 必需，`reason` 只有 escalate 会带。
        /// 其余未知键照旧忽略（与既有 derive 行为一致）。
        #[derive(Deserialize)]
        struct Tagged {
            decision: String,
            #[serde(default)]
            reason: Option<String>,
        }
        let raw = Tagged::deserialize(d)?;
        Ok(Self::from_wire(&raw.decision, raw.reason))
    }
}

/// 把 [`PermissionDecision`] 按**裸字符串**读写的 serde 适配器。
///
/// 节点链路的 `SessionOp::ApprovalAnswer.decision` 今天就是裸字符串
/// （`"allow_once"`），不是信封——合一类型后靠 `#[serde(with = ...)]` 保住这个
/// 既有形状（spec：每个边界保留它此前的序列化拼写）。
pub mod bare_decision {
    use super::PermissionDecision;
    use serde::{Deserialize, Deserializer, Serializer};

    /// 裸字符串序列化。
    pub fn serialize<S: Serializer>(d: &PermissionDecision, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(d.as_str())
    }

    /// 裸字符串反序列化（未知标签 → `Unknown`，不报错）。
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PermissionDecision, D::Error> {
        let raw = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        Ok(PermissionDecision::from_wire(raw.as_ref(), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单值往返：每个已知取值都必须恰好序列化成它既有的拼写。
    #[test]
    fn session_phase_spellings_are_pinned() {
        let cases = [
            (SessionPhase::Spawning, "spawning"),
            (SessionPhase::Active, "active"),
            (SessionPhase::Dormant, "dormant"),
            (SessionPhase::SpawnFailed, "spawn-failed"),
            (SessionPhase::Idle, "idle"),
            (SessionPhase::WaitingApproval, "waiting_approval"),
            (SessionPhase::Exited, "exited"),
            (SessionPhase::Closed, "closed"),
            (SessionPhase::Terminated, "terminated"),
            (SessionPhase::Failed, "failed"),
        ];
        for (phase, spelling) in cases {
            assert_eq!(phase.as_str(), spelling);
            assert_eq!(
                serde_json::to_value(&phase).unwrap(),
                serde_json::json!(spelling),
                "phase {spelling} 的线拼写变了"
            );
            let back: SessionPhase =
                serde_json::from_value(serde_json::json!(spelling)).unwrap();
            assert_eq!(back, phase, "phase {spelling} 往返不相等");
        }
    }

    /// 未知相位必须被接受、标记为未知、且原样保留拼写（前向容错）。
    #[test]
    fn unknown_phase_is_tolerated_and_preserved() {
        let phase: SessionPhase = serde_json::from_value(serde_json::json!("hibernating")).unwrap();
        assert!(phase.is_unknown());
        assert_eq!(phase, SessionPhase::Unknown("hibernating".into()));
        assert_eq!(phase.as_str(), "hibernating");
        assert_eq!(
            serde_json::to_value(&phase).unwrap(),
            serde_json::json!("hibernating"),
            "未知相位必须原样回吐，不得归一成别的拼写"
        );
    }

    /// 未知相位保守判为非终结。
    #[test]
    fn unknown_phase_is_not_terminal() {
        assert!(!SessionPhase::Unknown("hibernating".into()).is_terminal());
        for terminal in ["terminated", "closed", "exited", "failed"] {
            assert!(
                SessionPhase::from_wire(terminal).is_terminal(),
                "{terminal} 应当是终结相位"
            );
        }
        for live in ["spawning", "active", "idle", "waiting_approval", "dormant"] {
            assert!(!SessionPhase::from_wire(live).is_terminal(), "{live} 不是终结");
        }
    }

    /// 卡相位大小写敏感（飞书 `emoji_type`），拼写逐字保留。
    #[test]
    fn card_phase_spellings_are_pinned() {
        let cases = [
            (CardPhase::Get, "Get"),
            (CardPhase::OnIt, "OnIt"),
            (CardPhase::Done, "DONE"),
            (CardPhase::CrossMark, "CrossMark"),
        ];
        for (phase, spelling) in cases {
            assert_eq!(phase.as_str(), spelling);
            assert_eq!(serde_json::to_value(&phase).unwrap(), serde_json::json!(spelling));
        }
        // 大小写敏感：小写形式不是已知取值，落到 Unknown。
        assert!(CardPhase::from_wire("onit").is_unknown());
        assert!(CardPhase::from_wire("done").is_unknown());
    }

    /// mode 的四值拼写 + 未知值容错 + `parse` 的严格语义（如实拒绝）。
    #[test]
    fn session_mode_spellings_and_unknown() {
        for (mode, spelling) in [
            (SessionMode::Ask, "ask"),
            (SessionMode::Edit, "edit"),
            (SessionMode::Allow, "allow"),
            (SessionMode::Auto, "auto"),
        ] {
            assert_eq!(mode.as_str(), spelling);
            assert_eq!(serde_json::to_value(&mode).unwrap(), serde_json::json!(spelling));
        }
        // 宽容的线读取。
        let unknown: SessionMode = serde_json::from_value(serde_json::json!("turbo")).unwrap();
        assert_eq!(unknown, SessionMode::Unknown("turbo".into()));
        assert_eq!(serde_json::to_value(&unknown).unwrap(), serde_json::json!("turbo"));
        // 严格的边界解析：不认识的模式**如实拒绝**，不悄悄降级。
        assert_eq!(SessionMode::parse(" AUTO "), Some(SessionMode::Auto));
        assert_eq!(SessionMode::parse("turbo"), None);
        assert_eq!(SessionMode::default(), SessionMode::Ask);
    }

    /// 未知 mode **不放行**门控（fail closed），且不是 `auto`。
    #[test]
    fn unknown_mode_never_gates_open() {
        let unknown = SessionMode::Unknown("turbo".into());
        for category in [GateCategory::Edit, GateCategory::Execute, GateCategory::Other] {
            assert!(
                !unknown.allows_without_asking(category),
                "未知模式不得直接放行 {category:?}"
            );
        }
        assert!(!unknown.is_ungated());
    }

    /// 门控语义与搬迁前逐字一致。
    #[test]
    fn session_mode_gate_semantics_are_preserved() {
        for c in [GateCategory::Edit, GateCategory::Execute, GateCategory::Other] {
            assert!(!SessionMode::Ask.allows_without_asking(c));
            assert!(SessionMode::Allow.allows_without_asking(c));
            assert!(SessionMode::Auto.allows_without_asking(c));
        }
        assert!(SessionMode::Edit.allows_without_asking(GateCategory::Edit));
        assert!(!SessionMode::Edit.allows_without_asking(GateCategory::Execute));
        assert!(!SessionMode::Edit.allows_without_asking(GateCategory::Other));
        assert!(SessionMode::Auto.is_ungated());
        assert!(!SessionMode::Allow.is_ungated(), "allow 仍留审计门控");
        assert!(!SessionMode::default().is_ungated());
    }

    /// 回合词汇六值 + kind 两值的拼写。
    #[test]
    fn turn_vocabulary_spellings_are_pinned() {
        let cases = [
            (TurnElementType::Markdown, "markdown"),
            (TurnElementType::Thinking, "thinking"),
            (TurnElementType::Tool, "tool"),
            (TurnElementType::Error, "error"),
            (TurnElementType::Notice, "notice"),
            (TurnElementType::PermissionModeResult, "permission_mode_result"),
        ];
        for (t, spelling) in cases {
            assert_eq!(t.as_str(), spelling);
            assert_eq!(serde_json::to_value(&t).unwrap(), serde_json::json!(spelling));
            let back: TurnElementType =
                serde_json::from_value(serde_json::json!(spelling)).unwrap();
            assert_eq!(back, t);
        }
        // 下划线是契约：写成连字符就不是同一个取值。
        assert!(TurnElementType::from_wire("permission-mode-result").is_unknown());

        for (k, spelling) in [(TurnKind::Prompt, "prompt"), (TurnKind::Content, "content")] {
            assert_eq!(k.as_str(), spelling);
            assert_eq!(serde_json::to_value(&k).unwrap(), serde_json::json!(spelling));
        }
    }

    /// 未知元素类型必须被接受并保留原值（消费方渲染成通用块，不丢弃）。
    #[test]
    fn unknown_element_type_is_tolerated_and_preserved() {
        let t: TurnElementType =
            serde_json::from_value(serde_json::json!("hologram")).unwrap();
        assert!(t.is_unknown());
        assert_eq!(t.as_str(), "hologram");
        let k: TurnKind = serde_json::from_value(serde_json::json!("system")).unwrap();
        assert!(k.is_unknown());
        assert_eq!(k.as_str(), "system");
    }

    /// 决策四值往返 + 信封形状逐字不变。
    #[test]
    fn permission_decision_tags_are_pinned() {
        assert_eq!(
            serde_json::to_value(PermissionDecision::AllowOnce).unwrap(),
            serde_json::json!({"decision": "allow_once"})
        );
        assert_eq!(
            serde_json::to_value(PermissionDecision::AllowSession).unwrap(),
            serde_json::json!({"decision": "allow_session"})
        );
        assert_eq!(
            serde_json::to_value(PermissionDecision::Deny).unwrap(),
            serde_json::json!({"decision": "deny"})
        );
        assert_eq!(
            serde_json::to_value(PermissionDecision::Escalate {
                reason: "r".into()
            })
            .unwrap(),
            serde_json::json!({"decision": "escalate", "reason": "r"})
        );
    }

    /// 未知决策被接受、标记为不可兑现、并保留标签。
    #[test]
    fn unknown_decision_is_tolerated_but_not_answerable() {
        let d: PermissionDecision =
            serde_json::from_value(serde_json::json!({"decision": "sudo_always"})).unwrap();
        assert!(d.is_unknown());
        assert!(!d.is_answerable(), "未知决定不得被兑现成审批答复");
        assert_eq!(d.as_str(), "sudo_always");
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({"decision": "sudo_always"}),
            "未知决策必须原样回吐"
        );
    }

    /// `escalate` 降级语义：无等价物的边界拿到 `allow_once`，且回报降级理由。
    #[test]
    fn escalate_downgrades_to_allow_once_with_reason() {
        let escalate = PermissionDecision::Escalate {
            reason: "needs sudo".into(),
        };
        let (downgraded, reason) = escalate.downgrade_without_escalate();
        assert_eq!(downgraded, PermissionDecision::AllowOnce);
        assert_eq!(reason, Some("needs sudo"));

        // 其余取值不受降级影响。
        for d in [
            PermissionDecision::AllowOnce,
            PermissionDecision::AllowSession,
            PermissionDecision::Deny,
        ] {
            let (same, reason) = d.downgrade_without_escalate();
            assert_eq!(same, d);
            assert!(reason.is_none());
        }
    }

    /// 节点链路的裸字符串适配器：读进去、写出来都是裸字符串，且与信封互不干扰。
    #[test]
    fn bare_decision_adapter_keeps_the_plain_string_shape() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Holder {
            #[serde(with = "bare_decision")]
            decision: PermissionDecision,
        }

        let holder = Holder {
            decision: PermissionDecision::AllowSession,
        };
        let value = serde_json::to_value(&holder).unwrap();
        assert_eq!(value, serde_json::json!({"decision": "allow_session"}));

        let back: Holder = serde_json::from_value(value).unwrap();
        assert_eq!(back, holder);

        // 未知标签在裸形状下同样被容忍并保留。
        let unknown: Holder =
            serde_json::from_value(serde_json::json!({"decision": "sudo_always"})).unwrap();
        assert_eq!(
            unknown.decision,
            PermissionDecision::Unknown("sudo_always".into())
        );
        assert_eq!(
            serde_json::to_value(&unknown).unwrap(),
            serde_json::json!({"decision": "sudo_always"})
        );
    }
}
