//! 原生 sebas-agent 执行体桥（make-feishu-optional-webui-primary，design D2/D3）。
//!
//! `DispatchHandle` 持有一个可选的 [`NativeSessionBridge`]，把飞书/消息侧的
//! 会话执行路由到原生内核（`sebas-agent` 的 `SessionManager`），而不是 acp
//! 桥。桥为 `None` 时（无原生内核 / 纯映射测试），router 行为与现状完全
//! 一致——acp 桥是默认执行体。
//!
//! 原生会话的 key 约定与 webui 的 `NativeAgentBackend` 一致：
//! `chat_id = "agent-{hex}"`（`thread_id = None`）。webui 侧据此
//! 前缀把原生会话路由到 `NativeAgentBackend` 呈现（transcript + 审查卡）。

use sebas_channels::ChannelKey;
use std::sync::Arc;

/// 原生内核执行桥。`sebas-dispatch` 不直接依赖 `sebas-agent`——通过 trait
/// 反转依赖，core（`src/`）装配时注入具体实现；无桥 = 纯 acp 行为。
pub trait NativeSessionBridge: Send + Sync {
    /// 判断 `key` 是否属于原生内核会话（`agent-*` 前缀）。
    fn is_native(&self, key: &ChannelKey) -> bool;

    /// 向原生会话发送一条消息。`key` 必须是本桥管理的原生会话：
    /// - 已存在 → 送到该会话处理；
    /// - 不存在 → 创建原生会话并作为首条 prompt。
    ///
    /// 实现方负责 `Mapping` 登记与 `SessionEvent` 广播（与 acp 路径对齐）。
    /// 接收 `Arc<Self>`：具体桥需要把自身克隆进后台 task，`&self` 做不到。
    fn prompt(self: Arc<Self>, key: ChannelKey, text: String);

    /// 回填一个原生权限请求的决定（fail-closed：无答即拒）。返回是否
    /// 存在该待决请求（false = 无匹配或已过期）。
    fn answer_permission(&self, request_id: &str, decision: NativeApprovalDecision) -> bool;
}

/// 原生权限决定（trait 层的跨 crate 中立形状；具体桥映射到
/// `sebas_agent::policy::ApprovalAnswer`）。`Escalate` = 带理由的一次性放行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeApprovalDecision {
    AllowOnce,
    AllowSession,
    Deny,
    Escalate { reason: String },
}

/// Router 持有的桥类型（`None` = 未接线）。
pub type NativeBridge = Option<Arc<dyn NativeSessionBridge>>;