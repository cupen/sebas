//! 进程内内存状态引擎（测试夹具）。
//!
//! retire-legacy-state-json 3.2 删掉了 `state_store` 的文件回退，于是「没有
//! 引擎」不再等于「回退读 `state.json` / `providers.json`」，而是**状态不可
//! 用**。原先靠 `SEBAS_STATE_FILE` 指向临时文件做隔离的单测因此需要另一种
//! 隔离手段：本模块提供
//!
//! - [`MemoryEngine`]：一个纯内存的 `StateStoreEngine` 实现（persisted
//!   state / settings / projects / session map 全在内存）；
//! - [`install_fresh`]：拿全局锁 → 装一个**全新**引擎 → 返回 guard，drop 时
//!   清空引擎并解锁。全局引擎是进程级的，所以这类测试必须被这把锁串行化。
//!
//! 它**不是**生产代码路径：生产只有 `init_engine` 一次。放在 lib 里（而非
//! `#[cfg(test)]`）是为了让 `tests/` 集成测试也能复用同一份夹具。

use crate::state_store::{PersistedState, StateStoreEngine};
use sebas_models::agent::AgentRow;
use sebas_models::project::{ProjectRow, project_id_for_on};
use sebas_models::session_map::SessionMapRow;
use std::sync::{Arc, Mutex, MutexGuard};

/// 进程内的全部状态。
#[derive(Default)]
struct MemoryInner {
    state: Mutex<PersistedState>,
    settings: Mutex<Option<serde_json::Value>>,
    projects: Mutex<Vec<ProjectRow>>,
    session_map: Mutex<Vec<SessionMapRow>>,
    /// agent 目录行（add-agent-settings-and-session-titles 1.3）。
    agents: Mutex<Vec<AgentRow>>,
}

/// 纯内存状态引擎。
#[derive(Clone, Default)]
pub struct MemoryEngine {
    inner: Arc<MemoryInner>,
}

impl MemoryEngine {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl StateStoreEngine for MemoryEngine {
    async fn load_persisted_state(&self) -> PersistedState {
        self.inner.state.lock().unwrap().clone()
    }

    async fn save_persisted_state(&self, state: PersistedState) -> anyhow::Result<()> {
        *self.inner.state.lock().unwrap() = state;
        Ok(())
    }

    async fn load_settings(&self) -> Result<Option<serde_json::Value>, String> {
        Ok(self.inner.settings.lock().unwrap().clone())
    }

    async fn save_settings(&self, cfg: serde_json::Value) -> Result<(), String> {
        *self.inner.settings.lock().unwrap() = Some(cfg);
        Ok(())
    }

    async fn load_projects(&self) -> Result<Vec<ProjectRow>, String> {
        let mut projects = self.inner.projects.lock().unwrap().clone();
        projects.sort_by_key(|p| (p.sort_order, p.added_at));
        Ok(projects)
    }

    async fn save_projects(&self, projects: Vec<ProjectRow>) -> Result<(), String> {
        *self.inner.projects.lock().unwrap() = projects;
        Ok(())
    }

    async fn add_project(
        &self,
        node_id: &str,
        path: &str,
        name: &str,
        added_at: i64,
    ) -> Result<(), String> {
        let mut projects = self.inner.projects.lock().unwrap();
        if projects.iter().any(|p| p.path == path) {
            return Ok(());
        }
        projects.push(ProjectRow {
            id: Some(project_id_for_on(node_id, path)),
            path: path.to_string(),
            name: name.to_string(),
            branch_at: 0,
            added_at,
            sort_order: 0,
            node_id: node_id.to_string(),
            default_agent: None,
            branch: None,
        });
        Ok(())
    }

    async fn remove_project(&self, path: &str) -> Result<bool, String> {
        let mut projects = self.inner.projects.lock().unwrap();
        let before = projects.len();
        projects.retain(|p| p.path != path);
        Ok(projects.len() != before)
    }

    async fn set_project_default_agent(&self, id: &str, agent: &str) -> Result<(), String> {
        let mut projects = self.inner.projects.lock().unwrap();
        for p in projects.iter_mut() {
            if p.id.as_deref() == Some(id) {
                p.default_agent = Some(agent.to_string());
            }
        }
        Ok(())
    }

    async fn load_agents(&self) -> Result<Vec<AgentRow>, String> {
        let mut agents = self.inner.agents.lock().unwrap().clone();
        agents.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(agents)
    }

    async fn put_agent(&self, row: AgentRow) -> Result<(), String> {
        let mut agents = self.inner.agents.lock().unwrap();
        agents.retain(|r| r.id != row.id);
        agents.push(row);
        Ok(())
    }

    async fn delete_agent(&self, id: &str) -> Result<bool, String> {
        let mut agents = self.inner.agents.lock().unwrap();
        let before = agents.len();
        agents.retain(|r| r.id != id);
        Ok(agents.len() != before)
    }

    async fn clear_project_default_agent(&self, agent: &str) -> Result<usize, String> {
        let mut projects = self.inner.projects.lock().unwrap();
        let mut cleared = 0;
        for p in projects.iter_mut() {
            if p.default_agent.as_deref() == Some(agent) {
                p.default_agent = None;
                cleared += 1;
            }
        }
        Ok(cleared)
    }

    async fn load_session_map(&self) -> Result<Vec<SessionMapRow>, String> {
        Ok(self.inner.session_map.lock().unwrap().clone())
    }

    async fn save_session_entry(&self, entry: SessionMapRow) -> Result<(), String> {
        let mut rows = self.inner.session_map.lock().unwrap();
        rows.retain(|r| !(r.chat_id == entry.chat_id && r.thread_id == entry.thread_id));
        rows.push(entry);
        Ok(())
    }

    async fn delete_session_entry(
        &self,
        chat_id: String,
        thread_id: Option<String>,
    ) -> Result<(), String> {
        let mut rows = self.inner.session_map.lock().unwrap();
        rows.retain(|r| !(r.chat_id == chat_id && r.thread_id == thread_id));
        Ok(())
    }
}

/// 全局串行锁：所有「换引擎跑一段」的测试都必须持有它，否则并行测试会互相
/// 看到对方的引擎（进程级槽）。
///
/// **可重入**（同线程二次获取不阻塞、也不提前释放）：夹具函数常常在一个作用域
/// 里连着装好几次引擎（比如端到端用例逐场景重装），而 std `Mutex` 不可重入
/// ——不可重入会让第二次 `install_fresh` 自锁死。跨线程仍然互斥。
static SERIAL: Mutex<()> = Mutex::new(());

thread_local! {
    /// 当前线程是否已持有 [`SERIAL`]。
    static HOLDS_SERIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 取全局串行锁。返回 `None` = 本线程已持有（重入），调用方不要再释放。
fn acquire_serial() -> Option<MutexGuard<'static, ()>> {
    if HOLDS_SERIAL.with(|c| c.get()) {
        return None;
    }
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    HOLDS_SERIAL.with(|c| c.set(true));
    Some(guard)
}

/// 装一个全新内存引擎并返回 guard；guard drop 时清空引擎。
///
/// guard 同时持有全局串行锁（重入时只在最外层释放），因此同名测试天然互斥。
#[must_use = "guard 掉出作用域就清空引擎，必须绑到测试局部变量"]
pub struct EngineGuard {
    _lock: Option<MutexGuard<'static, ()>>,
}

impl Drop for EngineGuard {
    fn drop(&mut self) {
        crate::state_store::clear_engine();
        if self._lock.is_some() {
            HOLDS_SERIAL.with(|c| c.set(false));
        }
    }
}

/// 安装一个全新的内存引擎（见模块文档）。
pub fn install_fresh() -> EngineGuard {
    let lock = acquire_serial();
    crate::state_store::install_engine(Box::new(MemoryEngine::new()));
    EngineGuard { _lock: lock }
}

/// 安装一个**已填充**的内存引擎（预置 persisted state），返回 `(engine, guard)`。
/// 预置 state 用于「库里已有值」的用例。
pub fn install_fresh_with(state: PersistedState) -> (MemoryEngine, EngineGuard) {
    let lock = acquire_serial();
    let engine = MemoryEngine::new();
    *engine.inner.state.lock().unwrap() = state;
    crate::state_store::install_engine(Box::new(engine.clone()));
    (engine, EngineGuard { _lock: lock })
}

/// 清空引擎（模拟「状态库不可用」），返回 guard；guard drop 时同样清空。
/// 与 [`install_fresh`] 共用同一把锁，因此两者互斥。
pub fn install_none() -> EngineGuard {
    let lock = acquire_serial();
    crate::state_store::clear_engine();
    EngineGuard { _lock: lock }
}