pub mod card_events;
pub mod card_state;
pub mod cards;
pub mod cards_ui;
pub mod commands;
pub mod crud;
pub mod error;
pub mod native_bridge;
pub mod provider_state;
pub mod router;
pub mod settings;
pub mod state;
pub mod state_store;

pub use commands::{Command, GatewayAction, parse_command};
pub use crud::{CrudForm, CrudStore, FileStore, InMemoryStore, Item, ProviderForms};
pub use crate::router::{MsgIdMap, Out, RouterHandle, SessionEvent, SessionInfo, TurnEntry};
pub use cards::{CardConfig, ThinkingDisplay};
pub use state::{Mapping, SessionMap};

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
