//! 会话端口（design D2）：sebas-im 的全部逻辑只面向本 trait —— in-process
//! 装配与 `sebas im` 独立进程的通道客户端各自提供实现，IM 逻辑对传输零依赖。
//! 端口语义与核心会话通道请求一一对应（Snapshot/EnsureMessage/Cancel/Turns/
//! Subscribe/ApprovalAnswer/StateSnapshot/StateMutation）。

use async_trait::async_trait;
use sebas_channels::ChannelKey;
use sebas_dispatch::{SessionEvent, SessionInfo, TurnEntry};
use sebas_webui::session_backend::{PermissionDecision, PermissionNotice};
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::sync::broadcast;

/// 控制面决定（审批回传的 im 侧形状，与 webui `PermissionDecision` 同义）。
pub type Decision = PermissionDecision;

/// 审批卡按钮回调解析出的决定（卡片 behavior value 里的 `decision` 字段）。
pub fn parse_decision(raw: &str) -> Option<Decision> {
    match raw {
        "allow_once" => Some(PermissionDecision::AllowOnce),
        "allow_session" => Some(PermissionDecision::AllowSession),
        "deny" => Some(PermissionDecision::Deny),
        _ => None,
    }
}

/// watchdog 控制命令的 im 侧请求（control RPC 由装配方提供实现）。
#[derive(Debug, Clone)]
pub enum ControlRequest {
    Upgrade { dev: bool, dry_run: bool },
    Rollback,
    Restart,
    Services,
    System,
    Router(BTreeMap<String, String>),
    Webui,
    Confirm { token: String },
}

/// 随消息投递的本地附件引用（im 解析媒体后的产物；4.1/4.2）。
#[derive(Debug, Clone)]
pub struct ImAttachment {
    pub path: String,
    pub mime: Option<String>,
    pub name: Option<String>,
}

/// 会话端口：core 的会话面（观察 + 驱动 + 审批 + 状态库）。
#[async_trait]
pub trait CoreSessionPort: Send + Sync {
    /// 全量会话快照（/sessions 与视图重建的数据源）。
    async fn snapshot(&self) -> Vec<SessionInfo>;

    /// ensure 语义投递：未知 key 自动建会话、dormant 懒复活（2.1）。
    /// 附件（4.1）经服务端校验后以本地路径标记随文本投递。
    async fn ensure_message(
        &self,
        key: ChannelKey,
        message: String,
        attachments: Vec<ImAttachment>,
    ) -> Result<(), String>;

    /// 关闭会话（/new 的第一步；未知 key 视为已关闭，返回 Ok）。
    async fn close(&self, key: ChannelKey) -> Result<(), String>;

    /// 取消在飞 turn（2.2）。
    async fn cancel(&self, key: ChannelKey) -> Result<(), String>;

    /// turn 内容增量拉取；None = 会话不存在。
    async fn turns(&self, key: &ChannelKey, from: u64) -> Option<Vec<TurnEntry>>;

    /// 会话事件订阅（snapshot-then-events 语义由通道保证）。
    fn subscribe_sessions(&self) -> broadcast::Receiver<SessionEvent>;

    /// 审批请求订阅（原生内核 + ACP 桥统一面，2.4）；None = 无审批面。
    fn subscribe_approvals(&self) -> Option<broadcast::Receiver<PermissionNotice>>;

    /// 审批决定回传；false = 无待决请求（fail-closed，调用方置灰卡片）。
    async fn approval_answer(&self, request_id: &str, decision: Decision) -> bool;

    /// 状态库域快照（settings/providers）。
    async fn state_snapshot(&self, domain: &str) -> Option<Value>;

    /// 状态库域变更（settings/providers 表单提交的持久化路径）。
    async fn state_mutate(&self, domain: &str, payload: Value) -> Result<(), String>;
}

/// watchdog 控制端口：控制类命令直发 watchdog control RPC（spec:
/// dispatch-commands「Control commands forwarded to the watchdog」的 im 版）。
/// 装配方注入实现；None = 控制面不可用（bare-core 如实提示）。
#[async_trait]
pub trait ControlPort: Send + Sync {
    /// 返回给用户的纯文本结果/错误（反馈规格归 dispatch-commands）。
    async fn submit(&self, req: ControlRequest) -> Result<String, String>;
}

/// 无控制面的兜底实现：所有控制命令如实提示不可用。
pub struct NoControlPort;

#[async_trait]
impl ControlPort for NoControlPort {
    async fn submit(&self, _req: ControlRequest) -> Result<String, String> {
        Err("控制面不可用：watchdog 未注入控制凭据（bare-core 模式），\
             /upgrade 等命令需要通过 watchdog 启动 core。"
            .into())
    }
}
