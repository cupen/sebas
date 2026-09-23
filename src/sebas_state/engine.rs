//! DB 引擎实现 StateStoreEngine trait。
//!
//! 通过 `StateHandle` 将 async 请求派发到 DB 写者线程。存储侧动作经
//! `sebas_models` 的 ActiveRecord / 域查询完成（extract-sebas-db 4.3：
//! 在 `handle.exec` 闭包里调用生成的方法）；dispatch 的端口 trait 与
//! `MemoryEngine` 测试替身不变——ActiveRecord 是存储侧实现模式，不是
//! 跨进程 API 的变化（design D3b）。
//!
//! single-state-dir D2/D5：引擎持有**两个**写者句柄——settings 域
//! （providers/aliases/settings，有界）与 projects 域（projects/
//! session_map，增长）各归各库。projects 库不可用时（open 失败的降级
//! 形态），项目域方法如实返回点名原因的 Err（spec「a database's
//! unavailability is reported, not hidden」），settings 域照常工作；
//! 绝不拿空表/缺省值冒充现状。

use crate::sebas_state::writer::StateHandle;
use sebas_dispatch::state_store::{PersistedState, StateStoreEngine};
use serde_json::Value;

/// 基于 SQLite 的状态存储引擎（两库双写者）。
pub struct DbStateEngine {
    /// settings.db 句柄：providers / model_aliases / settings。
    settings: StateHandle,
    /// projects.db 句柄：projects / session_map。`None` = 库不可用（降级）。
    projects: Option<StateHandle>,
    /// projects 库不可用时的原因（进 typed rejection，如实点名）。
    projects_cause: String,
}

impl DbStateEngine {
    /// 只接 settings 库（projects 域不可用的降级形态，默认原因）。
    pub fn new(settings: StateHandle) -> Self {
        Self::with_unavailable_projects(settings, "projects 数据库未打开".to_string())
    }

    /// 两库都接（正常运行形态）。
    pub fn with_projects(settings: StateHandle, projects: StateHandle) -> Self {
        Self {
            settings,
            projects: Some(projects),
            projects_cause: String::new(),
        }
    }

    /// projects 库 open 失败的降级装配：带上 open 的真实失败原因，项目域
    /// 的每个方法都以此如实拒绝（run.rs 启动路径使用）。
    pub fn with_unavailable_projects(settings: StateHandle, cause: String) -> Self {
        Self {
            settings,
            projects: None,
            projects_cause: cause,
        }
    }

    /// projects 域句柄；不可用时返回点名原因的 Err（不假装成功）。
    fn projects(&self) -> Result<&StateHandle, String> {
        self.projects
            .as_ref()
            .ok_or_else(|| format!("projects 数据库不可用: {}", self.projects_cause))
    }
}

#[async_trait::async_trait]
impl StateStoreEngine for DbStateEngine {
    async fn load_persisted_state(&self) -> PersistedState {
        self.settings
            .exec(crate::sebas_state::repo::load_persisted_state)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "failed to load PersistedState from DB, using defaults");
                PersistedState::default()
            })
    }

    async fn save_persisted_state(&self, state: PersistedState) -> anyhow::Result<()> {
        self.settings
            .exec(move |conn| crate::sebas_state::repo::save_persisted_state(conn, &state))
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        // providers + aliases + settings 域都随 PersistedState 一次提交
        // （同一 settings 连接的单事务——design D3/D4）。
        sebas_dispatch::state_store::notify_change("providers");
        Ok(())
    }

    async fn load_settings(&self) -> Result<Option<Value>, String> {
        self.settings
            .exec(|conn| {
                crate::sebas_state::repo::load_settings(conn)
                    .map(|opt| opt.map(|cfg| serde_json::to_value(&cfg).unwrap_or_default()))
            })
            .await
    }

    async fn save_settings(&self, cfg: Value) -> Result<(), String> {
        let card_cfg: sebas_feishu::cards::CardConfig = serde_json::from_value(cfg)
            .map_err(|e| format!("settings value 不是有效 CardConfig: {e}"))?;
        self.settings
            .exec(move |conn| crate::sebas_state::repo::save_settings(conn, &card_cfg))
            .await?;
        sebas_dispatch::state_store::notify_change("settings");
        Ok(())
    }

    async fn load_projects(&self) -> Result<Vec<Value>, String> {
        self.projects()?
            .exec(|conn| {
                sebas_models::project::load_projects(conn).map(|rows| {
                    rows.into_iter()
                        .map(|r| serde_json::to_value(&r).unwrap_or_default())
                        .collect()
                })
            })
            .await
    }

    async fn save_projects(&self, projects: Vec<Value>) -> Result<(), String> {
        let rows: Vec<sebas_models::project::ProjectRow> = projects
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        self.projects()?
            .exec(move |conn| sebas_models::project::save_projects(conn, &rows))
            .await?;
        sebas_dispatch::state_store::notify_change("projects");
        Ok(())
    }

    async fn set_project_default_agent(&self, id: &str, agent: &str) -> Result<(), String> {
        let id = id.to_string();
        let agent = agent.to_string();
        self.projects()?
            .exec(move |conn| {
                sebas_models::project::set_project_default_agent(conn, &id, &agent)
            })
            .await?;
        sebas_dispatch::state_store::notify_change("projects");
        Ok(())
    }

    async fn add_project(&self, path: &str, name: &str, added_at: i64) -> Result<(), String> {
        let p = path.to_string();
        let n = name.to_string();
        self.projects()?
            .exec(move |conn| sebas_models::project::add_project(conn, &p, &n, added_at))
            .await?;
        sebas_dispatch::state_store::notify_change("projects");
        Ok(())
    }

    async fn remove_project(&self, path: &str) -> Result<bool, String> {
        let p = path.to_string();
        let removed = self
            .projects()?
            .exec(move |conn| sebas_models::project::remove_project(conn, &p))
            .await?;
        if removed {
            sebas_dispatch::state_store::notify_change("projects");
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sebas_state::writer::StateWriter;
    use tempfile::tempdir;

    /// 3.5 验收：projects.db 打不开时项目面呈现 unavailable（typed
    /// rejection 点名原因），settings 面照常可用。
    #[tokio::test]
    async fn unavailable_projects_db_degrades_honestly_and_settings_keep_working() {
        let dir = tempdir().unwrap();
        // settings.db 正常打开。
        let settings =
            StateWriter::start_settings(dir.path().join("settings.db")).expect("settings ok");
        // projects.db 打不开：路径是一个**目录**（SQLite 无法在目录上建库）。
        let blocked = dir.path().join("projects.db");
        std::fs::create_dir_all(&blocked).unwrap();
        let projects_cause = StateWriter::start_projects(blocked.clone())
            .err()
            .expect("目录路径上 open 必失败")
            .trim()
            .to_string();

        let engine = DbStateEngine::with_unavailable_projects(settings.handle().clone(), projects_cause);

        // 项目面：每个方法都如实拒绝并点名原因，不拿空表冒充现状。
        let err = engine.load_projects().await.unwrap_err();
        assert!(err.contains("不可用"), "{err}");
        assert!(
            err.contains(&format!("{}", blocked.display())) || err.contains("初始化失败"),
            "错误要点名原因: {err}"
        );
        assert!(engine.add_project("/tmp/p", "p", 1).await.is_err());
        assert!(engine.remove_project("/tmp/p").await.is_err());
        assert!(engine
            .set_project_default_agent("proj-x", "claude")
            .await
            .is_err());
        assert!(engine.save_projects(vec![]).await.is_err());

        // 设置面：完全不受影响。
        engine
            .save_settings(serde_json::json!({}))
            .await
            .expect("settings domain keeps working");
        assert!(engine.load_settings().await.unwrap().is_some());
        let state = engine.load_persisted_state().await;
        assert_eq!(state.mode, sebas_dispatch::provider_state::ProviderMode::Off);
    }
}
