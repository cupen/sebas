//! DB 引擎实现 StateStoreEngine trait。
//!
//! 通过 `StateHandle` 将 async 请求派发到 DB 写者线程。

use crate::sebas_state::writer::StateHandle;
use sebas_router::state_store::{PersistedState, StateStoreEngine};
use serde_json::Value;

/// 基于 SQLite 的状态存储引擎。
pub struct DbStateEngine {
    handle: StateHandle,
}

impl DbStateEngine {
    pub fn new(handle: StateHandle) -> Self {
        Self { handle }
    }
}

#[async_trait::async_trait]
impl StateStoreEngine for DbStateEngine {
    async fn load_persisted_state(&self) -> PersistedState {
        self.handle
            .exec(|conn| crate::sebas_state::repo::load_persisted_state(conn))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "DB 加载 PersistedState 失败, 回退默认");
                PersistedState::default()
            })
    }

    async fn save_persisted_state(&self, state: PersistedState) -> anyhow::Result<()> {
        self.handle
            .exec(move |conn| crate::sebas_state::repo::save_persisted_state(conn, &state))
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        // providers + aliases + settings 域都随 PersistedState 一次提交。
        sebas_router::state_store::notify_change("providers");
        Ok(())
    }

    async fn load_settings(&self) -> Result<Option<Value>, String> {
        self.handle
            .exec(|conn| {
                crate::sebas_state::repo::load_settings(conn)
                    .map(|opt| opt.map(|cfg| serde_json::to_value(&cfg).unwrap_or_default()))
            })
            .await
    }

    async fn save_settings(&self, cfg: Value) -> Result<(), String> {
        let card_cfg: sebas_feishu::cards::CardConfig = serde_json::from_value(cfg)
            .map_err(|e| format!("settings value 不是有效 CardConfig: {e}"))?;
        self.handle
            .exec(move |conn| crate::sebas_state::repo::save_settings(conn, &card_cfg))
            .await?;
        sebas_router::state_store::notify_change("settings");
        Ok(())
    }

    async fn load_projects(&self) -> Result<Vec<Value>, String> {
        self.handle
            .exec(|conn| {
                crate::sebas_state::repo::load_projects(conn)
                    .map(|rows| rows.into_iter().map(|r| serde_json::to_value(&r).unwrap_or_default()).collect())
            })
            .await
    }

    async fn save_projects(&self, projects: Vec<Value>) -> Result<(), String> {
        let rows: Vec<crate::sebas_state::repo::ProjectRow> = projects
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        self.handle
            .exec(move |conn| crate::sebas_state::repo::save_projects(conn, &rows))
            .await?;
        sebas_router::state_store::notify_change("projects");
        Ok(())
    }

    async fn add_project(&self, path: &str, name: &str, added_at: i64) -> Result<(), String> {
        let p = path.to_string();
        let n = name.to_string();
        self.handle
            .exec(move |conn| crate::sebas_state::repo::add_project(conn, &p, &n, added_at))
            .await?;
        sebas_router::state_store::notify_change("projects");
        Ok(())
    }

    async fn remove_project(&self, path: &str) -> Result<bool, String> {
        let p = path.to_string();
        let removed = self
            .handle
            .exec(move |conn| crate::sebas_state::repo::remove_project(conn, &p))
            .await?;
        if removed {
            sebas_router::state_store::notify_change("projects");
        }
        Ok(removed)
    }
}