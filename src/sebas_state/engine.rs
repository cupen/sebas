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
use sebas_models::project::ProjectRow;
use sebas_models::session_map::SessionMapRow;
use serde_json::Value;

/// 基于 SQLite 的状态存储引擎（两库双写者）。
pub struct DbStateEngine {
    /// settings.db 句柄：providers / model_aliases / settings / agents。
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

    async fn load_projects(&self) -> Result<Vec<ProjectRow>, String> {
        self.projects()?.exec(sebas_models::project::load_projects).await
    }

    async fn save_projects(&self, projects: Vec<ProjectRow>) -> Result<(), String> {
        self.projects()?
            .exec(move |conn| sebas_models::project::save_projects(conn, &projects))
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

    // ---- agents 域（add-agent-settings-and-session-titles 1.3/1.4）----
    //
    // 行归 settings.db（agents 表）；删除守卫的「清项目默认」半边落
    // projects.db。两库各一笔事务——跨库单事务按分层纪律不存在，agents_
    // mutation 按先清默认、后删行的顺序串行提交。

    async fn load_agents(&self) -> Result<Vec<sebas_models::agent::AgentRow>, String> {
        self.settings
            .exec(crate::sebas_state::repo::load_agents)
            .await
    }

    async fn put_agent(&self, row: sebas_models::agent::AgentRow) -> Result<(), String> {
        self.settings
            .exec(move |conn| crate::sebas_state::repo::save_agent(conn, row))
            .await?;
        sebas_dispatch::state_store::notify_change("agents");
        Ok(())
    }

    async fn delete_agent(&self, id: &str) -> Result<bool, String> {
        let id = id.to_string();
        let existed = self
            .settings
            .exec(move |conn| crate::sebas_state::repo::delete_agent(conn, &id))
            .await?;
        if existed {
            sebas_dispatch::state_store::notify_change("agents");
        }
        Ok(existed)
    }

    async fn clear_project_default_agent(&self, agent: &str) -> Result<usize, String> {
        let agent = agent.to_string();
        self.projects()?
            .exec(move |conn| sebas_models::project::clear_default_agent_for(conn, &agent))
            .await
    }

    async fn add_project(
        &self,
        node_id: &str,
        path: &str,
        name: &str,
        added_at: i64,
    ) -> Result<(), String> {
        let n = node_id.to_string();
        let p = path.to_string();
        let nm = name.to_string();
        self.projects()?
            .exec(move |conn| sebas_models::project::add_project(conn, &n, &p, &nm, added_at))
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

    // ---- 会话映射（persist-session-map 2.1）：生产读写路径 ----
    //
    // 读经 `sebas_models::session_map::load_session_map`（恢复），写/删经
    // ActiveRecord 生成的 upsert / `delete_by`（按变更落库，无手写 SQL），
    // 全部经 projects 库的单写 actor 串行提交。projects 库不可用时如实
    // 拒绝（typed rejection 点名原因），不拿空表冒充现状。

    async fn load_session_map(&self) -> Result<Vec<SessionMapRow>, String> {
        self.projects()?
            .exec(sebas_models::session_map::load_session_map)
            .await
    }

    async fn save_session_entry(&self, entry: SessionMapRow) -> Result<(), String> {
        self.projects()?
            .exec(move |conn| entry.save(conn).map_err(|e| e.to_string()))
            .await?;
        sebas_dispatch::state_store::notify_change("sessions");
        Ok(())
    }

    async fn delete_session_entry(
        &self,
        chat_id: String,
        thread_id: Option<String>,
    ) -> Result<(), String> {
        self.projects()?
            .exec(move |conn| {
                SessionMapRow::delete_by(conn, &chat_id, &thread_id)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            })
            .await?;
        sebas_dispatch::state_store::notify_change("sessions");
        Ok(())
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
        assert!(engine
            .add_project("local", "/tmp/p", "p", 1)
            .await
            .is_err());
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

    /// persist-session-map 2.1/2.2：会话映射的引擎读写经 projects 库单写
    /// actor 提交——保存即提交（响应返回前库中可见），删除后读回为空；
    /// 提交走 ActiveRecord（无手写 SQL）。不可用的 projects 库如实拒绝。
    #[tokio::test]
    async fn session_map_entries_round_trip_through_the_engine() {
        let dir = tempdir().unwrap();
        let settings = StateWriter::start_settings(dir.path().join("settings.db")).unwrap();
        let projects = StateWriter::start_projects(dir.path().join("projects.db")).unwrap();
        let engine = DbStateEngine::with_projects(settings.handle().clone(), projects.handle().clone());

        let entry = SessionMapRow {
            chat_id: "web".into(),
            thread_id: Some("web-persist".into()),
            session_id: "sess-1".into(),
            last_active_unix: 42,
            project_dir: Some("/tmp/persist".into()),
            acp_session_id: Some("acp-1".into()),
            current_model: None,
            pending_kind: Some("claude".into()),
            pending_model: None,
            pending_mode: None,
            desired_mode: "edit".into(),
            label: Some("映射行".into()),
            prompt_preview: None,
            awaiting_first_prompt: false,
        };
        engine.save_session_entry(entry.clone()).await.unwrap();

        // 同键覆盖（按变更落库 = 反复 upsert）。
        let mut updated = entry.clone();
        updated.session_id = "sess-2".into();
        engine.save_session_entry(updated).await.unwrap();

        let rows = engine.load_session_map().await.unwrap();
        assert_eq!(rows.len(), 1, "同键覆盖不产生第二行");
        assert_eq!(rows[0].session_id, "sess-2");
        assert_eq!(rows[0].desired_mode, "edit");

        engine
            .delete_session_entry("web".into(), Some("web-persist".into()))
            .await
            .unwrap();
        assert!(engine.load_session_map().await.unwrap().is_empty());
    }
}
