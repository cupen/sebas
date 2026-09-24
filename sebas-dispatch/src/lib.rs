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
pub mod state;
pub mod state_store;
/// 测试夹具：进程内内存状态引擎 + 换引擎 guard（retire-legacy-state-json 3.2
/// 之后，没有引擎不再等于「回退读文件」，测试需要自带一个引擎）。
#[doc(hidden)]
pub mod test_engine;

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

/// 既有的 env 串行锁（`SEBAS_STATE_FILE` 已退休，但别名/会话路径等仍用
/// `SEBAS_*` 环境变量；保留锁与其调用点，避免并行测试互踩 env）。
#[doc(hidden)]
pub mod test_util {
    use std::sync::Mutex;
    /// 全局锁，保护测试里的进程级 env 变更不被并行测试竞争。
    pub static STATE_FILE_LOCK: Mutex<()> = Mutex::new(());
    /// 锁住 STATE_FILE_LOCK，抗 poison。
    pub fn lock_state_file() -> std::sync::MutexGuard<'static, ()> {
        STATE_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}
