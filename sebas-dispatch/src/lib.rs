pub mod card_events;
pub mod card_state;
pub mod cards;
pub mod cards_ui;
pub mod commands;
pub mod crud;
pub mod engine;
pub mod error;
pub mod native_bridge;
pub mod provider_state;
pub mod settings;
pub mod state;
pub mod state_store;

pub use crate::engine::{
    DispatchHandle, MsgIdMap, Out, PendingApproval, RemoteSessionView, SessionEvent, SessionInfo,
    TurnEntry, TurnStreamEvent, count_chat_messages,
};
/// 错误条目失败分类词表（fix-webui-qa-defects 5.1/5.2）：`TurnEntry
/// ::failure_class` 的合法值。webui 标签映射与 wire 值同源。
pub use crate::engine::failure_class;
pub use cards::{CardConfig, ThinkingDisplay};
pub use commands::{Command, RouterAction, parse_command};
pub use crud::{CrudForm, CrudStore, FileStore, InMemoryStore, Item, ProviderForms};
pub use state::{
    MAX_PENDING_SUBMISSIONS, Mapping, PendingDisposition, PendingOpError, PendingSubmission,
    QueuedTurn, SessionIdentity, SessionMap,
};

/// 所有 SEBAS_STATE_FILE env 操作串行化（crud + provider_state 共享）。
#[doc(hidden)]
pub mod test_util {
    use std::sync::Mutex;
    /// 全局锁，保护 SEBAS_STATE_FILE 环境变量不被并行测试竞争。
    pub static STATE_FILE_LOCK: Mutex<()> = Mutex::new(());
    /// 锁住 STATE_FILE_LOCK，抗 poison。
    pub fn lock_state_file() -> std::sync::MutexGuard<'static, ()> {
        STATE_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}
