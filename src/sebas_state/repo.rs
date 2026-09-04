//! 领域仓储层: 通过 `StateHandle` 提供类型化表访问。
//!
//! 每个方法接受 `&mut Connection` 并同步执行, 由 `StateHandle::exec` 调度到
//! 写者线程。所有方法都是纯同步的, 不依赖 tokio。

use crate::sebas_state::writer::StateHandle;
use rusqlite::{params, Connection};

// ---- Provider state ----

/// 从 DB 加载 provider 数据, 构造 `sebas_router::state_store::PersistedState`。
///
/// 读取 providers 表(含软删) + model_aliases 表, 与 `provider_state.rs` 的
/// runtime 段合并。
pub fn load_persisted_state(conn: &mut Connection) -> Result<sebas_router::state_store::PersistedState, String> {
    
    use sebas_router::state_store::PersistedState;
    use std::collections::BTreeMap;

    // Block scope ensures stmt is dropped before mutable borrows below
    let (providers, deleted) = {
        // 读 providers 表 (含软删)
        let mut stmt = conn
            .prepare("SELECT id, config, deleted FROM providers ORDER BY id")
            .map_err(|e| format!("准备 providers 查询失败: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let config: String = row.get(1)?;
                let deleted: i64 = row.get(2)?;
                Ok((id, config, deleted))
            })
            .map_err(|e| format!("查询 providers 失败: {e}"))?;

        let mut providers: BTreeMap<String, sebas_router::crud::Item> = BTreeMap::new();
        let mut deleted: Vec<String> = Vec::new();

        for row in rows {
            let (id, config, del) = row.map_err(|e| format!("读取 provider 行失败: {e}"))?;
            if del != 0 {
                deleted.push(id);
            } else {
                if let Ok(item) = serde_json::from_str::<sebas_router::crud::Item>(&config) {
                    providers.insert(id, item);
                } else {
                    tracing::warn!("provider {id} 配置 JSON 解析失败, 跳过");
                }
            }
        }

        (providers, deleted)
    };

    // 读 model_aliases (块作用域确保 stmt 及时 drop)
    let model_aliases = {
        let mut stmt = conn
            .prepare("SELECT alias, provider, upstream_model FROM model_aliases ORDER BY alias")
            .map_err(|e| format!("准备 model_aliases 查询失败: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| format!("查询 model_aliases 失败: {e}"))?;

        let mut aliases: BTreeMap<String, sebas_router::state_store::ModelAliasEntry> =
            BTreeMap::new();
        for row in rows {
            let (alias, provider, upstream_model) =
                row.map_err(|e| format!("读取 model_alias 行失败: {e}"))?;
            aliases.insert(
                alias,
                sebas_router::state_store::ModelAliasEntry {
                    provider,
                    upstream_model,
                },
            );
        }
        aliases
    };

    // 读 settings 中的 mode/default_selection (如果存在)
    let (mode, default_selection) = load_runtime_state(conn);

    Ok(PersistedState {
        version: sebas_router::state_store::STATE_VERSION_V2,
        providers,
        deleted,
        mode,
        default_selection,
        model_aliases,
    })
}

/// 从 DB 加载 runtime 状态 (mode + default_selection)。
fn load_runtime_state(
    conn: &mut Connection,
) -> (sebas_router::provider_state::ProviderMode, Option<sebas_router::state_store::DefaultSelection>) {
    use sebas_router::provider_state::ProviderMode;
    

    let json: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'runtime_state'",
            [],
            |row| row.get(0),
        )
        .ok();

    match json {
        Some(raw) => {
            match serde_json::from_str::<RuntimeStateRow>(&raw) {
                Ok(row) => (row.mode, row.default_selection),
                Err(e) => {
                    tracing::warn!(error = %e, "runtime_state 解析失败, 回退默认");
                    (ProviderMode::default(), None)
                }
            }
        }
        None => (ProviderMode::default(), None),
    }
}

/// 保存 PersistedState 到 DB。
///
/// 写入 providers 表 (upsert + 软删) + 运行时状态到 settings 表。
pub fn save_persisted_state(conn: &mut Connection, state: &sebas_router::state_store::PersistedState) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("保存状态事务开始失败: {e}"))?;

    // 清空旧数据（先清 aliases 再清 providers——`model_aliases.provider`
    // 有 REFERENCES providers(id) 外键，FK ON 时顺序不能反）。
    tx.execute("DELETE FROM model_aliases", [])
        .map_err(|e| format!("清空 model_aliases 表失败: {e}"))?;
    tx.execute("DELETE FROM providers", [])
        .map_err(|e| format!("清空 providers 表失败: {e}"))?;

    // 写 providers (非软删)
    let now = crate::sebas_state::db::unix_now();
    for (id, item) in &state.providers {
        let config = serde_json::to_string(item)
            .map_err(|e| format!("序列化 provider {id} 失败: {e}"))?;
        tx.execute(
            "INSERT INTO providers (id, config, deleted, created_at, updated_at) VALUES (?1, ?2, 0, ?3, ?3)",
            params![id, config, now],
        )
        .map_err(|e| format!("写入 provider {id} 失败: {e}"))?;
    }

    // 写 deleted providers (软删)
    for id in &state.deleted {
        tx.execute(
            "INSERT INTO providers (id, config, deleted, created_at, updated_at) VALUES (?1, '{}', 1, ?2, ?2)
             ON CONFLICT(id) DO UPDATE SET deleted = 1, updated_at = ?2",
            params![id, now],
        )
        .map_err(|e| format!("写入 deleted provider {id} 失败: {e}"))?;
    }

    // 写 model_aliases (add-state-store 5.3：随状态库流转)
    for (alias, entry) in &state.model_aliases {
        tx.execute(
            "INSERT INTO model_aliases (alias, provider, upstream_model, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(alias) DO UPDATE SET provider = ?2, upstream_model = ?3",
            params![alias, entry.provider, entry.upstream_model, now],
        )
        .map_err(|e| format!("写入 model alias {alias} 失败: {e}"))?;
    }

    // 写 runtime state
    let runtime = RuntimeStateRow {
        mode: state.mode.clone(),
        default_selection: state.default_selection.clone(),
    };
    let runtime_json = serde_json::to_string(&runtime)
        .map_err(|e| format!("序列化 runtime state 失败: {e}"))?;
    tx.execute(
        "INSERT INTO settings (key, value) VALUES ('runtime_state', ?1)
         ON CONFLICT(key) DO UPDATE SET value = ?1",
        params![runtime_json],
    )
    .map_err(|e| format!("写入 runtime state 失败: {e}"))?;

    tx.commit()
        .map_err(|e| format!("保存状态事务提交失败: {e}"))?;

    Ok(())
}

/// 更新 PersistedState (RMW 模式), 与 `state_store::update` 对应。
pub fn update_persisted_state(
    conn: &mut Connection,
    f: impl FnOnce(&mut sebas_router::state_store::PersistedState),
) -> Result<sebas_router::state_store::PersistedState, String> {
    let mut state = load_persisted_state(conn)?;
    f(&mut state);
    save_persisted_state(conn, &state)?;
    Ok(state)
}

// ---- Settings ----

/// 加载 settings (CardConfig), 从 `settings` 表 `key = 'card_config'`。
pub fn load_settings(conn: &mut Connection) -> Result<Option<sebas_feishu::cards::CardConfig>, String> {
    let json: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'card_config'",
            [],
            |row| row.get(0),
        )
        .ok();

    match json {
        Some(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| format!("settings JSON 解析失败: {e}")),
        None => Ok(None),
    }
}

/// 保存 settings (CardConfig)。
pub fn save_settings(conn: &mut Connection, cfg: &sebas_feishu::cards::CardConfig) -> Result<(), String> {
    let json = serde_json::to_string(cfg)
        .map_err(|e| format!("序列化 settings 失败: {e}"))?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('card_config', ?1)
         ON CONFLICT(key) DO UPDATE SET value = ?1",
        params![json],
    )
    .map_err(|e| format!("写入 settings 失败: {e}"))?;
    Ok(())
}

// ---- Projects ----

/// 项目条目 (JSON 兼容形状, 与 `sebas_webui::projects::ProjectEntry` 对应)。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectRow {
    pub path: String,
    pub name: String,
    pub branch: Option<String>,
    pub branch_at: i64,
    pub added_at: i64,
    pub sort_order: i64,
}

/// 加载所有项目。
pub fn load_projects(conn: &mut Connection) -> Result<Vec<ProjectRow>, String> {
    let mut stmt = conn
        .prepare("SELECT path, name, branch, branch_at, added_at, sort_order FROM projects ORDER BY sort_order, added_at")
        .map_err(|e| format!("准备 projects 查询失败: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(ProjectRow {
                path: row.get(0)?,
                name: row.get(1)?,
                branch: row.get(2)?,
                branch_at: row.get(3)?,
                added_at: row.get(4)?,
                sort_order: row.get(5)?,
            })
        })
        .map_err(|e| format!("查询 projects 失败: {e}"))?;

    let mut projects = Vec::new();
    for row in rows {
        projects.push(row.map_err(|e| format!("读取 project 行失败: {e}"))?);
    }
    Ok(projects)
}

/// 保存所有项目 (全量替换)。
pub fn save_projects(conn: &mut Connection, projects: &[ProjectRow]) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("保存 projects 事务开始失败: {e}"))?;

    tx.execute("DELETE FROM projects", [])
        .map_err(|e| format!("清空 projects 表失败: {e}"))?;

    for p in projects {
        tx.execute(
            "INSERT INTO projects (path, name, branch, branch_at, added_at, sort_order) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![p.path, p.name, p.branch, p.branch_at, p.added_at, p.sort_order],
        )
        .map_err(|e| format!("写入 project {} 失败: {e}", p.path))?;
    }

    tx.commit()
        .map_err(|e| format!("保存 projects 事务提交失败: {e}"))?;
    Ok(())
}

/// 添加一个项目。
pub fn add_project(conn: &mut Connection, path: &str, name: &str, added_at: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO projects (path, name, branch, branch_at, added_at, sort_order) VALUES (?1, ?2, NULL, 0, ?3, 0)
         ON CONFLICT(path) DO NOTHING",
        params![path, name, added_at],
    )
    .map_err(|e| format!("添加项目 {path} 失败: {e}"))?;
    Ok(())
}

/// 删除项目。
pub fn remove_project(conn: &mut Connection, path: &str) -> Result<bool, String> {
    let affected = conn
        .execute("DELETE FROM projects WHERE path = ?1", params![path])
        .map_err(|e| format!("删除项目 {path} 失败: {e}"))?;
    Ok(affected > 0)
}

/// 更新项目分支信息。
pub fn update_project_branch(conn: &mut Connection, path: &str, branch: Option<&str>, branch_at: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE projects SET branch = ?1, branch_at = ?2 WHERE path = ?3",
        params![branch, branch_at, path],
    )
    .map_err(|e| format!("更新项目分支 {path} 失败: {e}"))?;
    Ok(())
}

// ---- Session map ----

/// 加载会话映射 (用于恢复)。
pub fn load_session_map(conn: &mut Connection) -> Result<Vec<(String, Option<String>, String, i64, Option<String>)>, String> {
    let mut stmt = conn
        .prepare("SELECT chat_id, thread_id, session_id, last_active_unix, project_dir FROM session_map")
        .map_err(|e| format!("准备 session_map 查询失败: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| format!("查询 session_map 失败: {e}"))?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(|e| format!("读取 session_map 行失败: {e}"))?);
    }
    Ok(entries)
}

/// 保存会话映射 (全量替换)。
pub fn save_session_map(
    conn: &mut Connection,
    entries: &[(String, Option<String>, String, i64, Option<String>)],
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("保存 session_map 事务开始失败: {e}"))?;

    tx.execute("DELETE FROM session_map", [])
        .map_err(|e| format!("清空 session_map 表失败: {e}"))?;

    for (chat_id, thread_id, session_id, last_active_unix, project_dir) in entries {
        tx.execute(
            "INSERT INTO session_map (chat_id, thread_id, session_id, last_active_unix, project_dir) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![chat_id, thread_id, session_id, last_active_unix, project_dir],
        )
        .map_err(|e| format!("写入 session_map 失败: {e}"))?;
    }

    tx.commit()
        .map_err(|e| format!("保存 session_map 事务提交失败: {e}"))?;
    Ok(())
}

// ---- Runtime state wire type ----

/// 运行时状态行 (mode + default_selection) 的 JSON 形状。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RuntimeStateRow {
    #[serde(default)]
    mode: sebas_router::provider_state::ProviderMode,
    #[serde(default)]
    default_selection: Option<sebas_router::state_store::DefaultSelection>,
}

// ---- Async 包装 (通过 StateHandle) ----

/// 异步版本的 repo 操作, 通过 `StateHandle` 调度到写者线程。
pub struct Repo;

impl Repo {
    /// 加载 PersistedState。
    pub async fn load_persisted_state(handle: &StateHandle) -> Result<sebas_router::state_store::PersistedState, String> {
        handle
            .exec(|conn| load_persisted_state(conn))
            .await
    }

    /// 加载 settings。
    pub async fn load_settings(handle: &StateHandle) -> Result<Option<sebas_feishu::cards::CardConfig>, String> {
        handle
            .exec(|conn| load_settings(conn))
            .await
    }

    /// 保存 settings。
    pub async fn save_settings(handle: &StateHandle, cfg: &sebas_feishu::cards::CardConfig) -> Result<(), String> {
        let cfg = cfg.clone();
        handle
            .exec(move |conn| save_settings(conn, &cfg))
            .await
    }

    /// 加载所有项目。
    pub async fn load_projects(handle: &StateHandle) -> Result<Vec<ProjectRow>, String> {
        handle
            .exec(|conn| load_projects(conn))
            .await
    }

    /// 保存所有项目。
    pub async fn save_projects(handle: &StateHandle, projects: Vec<ProjectRow>) -> Result<(), String> {
        handle
            .exec(move |conn| save_projects(conn, &projects))
            .await
    }

    /// 添加项目。
    pub async fn add_project(handle: &StateHandle, path: String, name: String, added_at: i64) -> Result<(), String> {
        handle
            .exec(move |conn| add_project(conn, &path, &name, added_at))
            .await
    }

    /// 删除项目。
    pub async fn remove_project(handle: &StateHandle, path: String) -> Result<bool, String> {
        handle
            .exec(move |conn| remove_project(conn, &path))
            .await
    }

    /// 更新项目分支。
    pub async fn update_project_branch(handle: &StateHandle, path: String, branch: Option<String>, branch_at: i64) -> Result<(), String> {
        handle
            .exec(move |conn| update_project_branch(conn, &path, branch.as_deref(), branch_at))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sebas_state::db;
    use crate::sebas_state::migration::run_migrations;
    use tempfile::tempdir;

    fn setup_db() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let mut conn = db::open(&path).unwrap();
        run_migrations(&mut conn, &path).unwrap();
        (dir, conn)
    }

    #[test]
    fn load_empty_db_returns_default_state() {
        let (_dir, mut conn) = setup_db();
        let state = load_persisted_state(&mut conn).unwrap();
        assert!(state.providers.is_empty());
        assert!(state.deleted.is_empty());
        assert_eq!(state.mode, sebas_router::provider_state::ProviderMode::Off);
        assert_eq!(state.default_selection, None);
    }

    #[test]
    fn save_and_load_provider_state_round_trips() {
        let (_dir, mut conn) = setup_db();
        use sebas_router::provider_state::ProviderMode;
        use sebas_router::state_store::{DefaultSelection, PersistedState};
        use std::collections::BTreeMap;

        let mut item = serde_json::Map::new();
        item.insert("name".into(), serde_json::Value::String("deepseek".into()));
        item.insert("preset".into(), serde_json::Value::String("deepseek".into()));

        let original = PersistedState {
            version: 2,
            providers: BTreeMap::from([("deepseek".into(), item)]),
            deleted: vec!["openai".into()],
            mode: ProviderMode::Direct { provider: "deepseek".into() },
            default_selection: Some(DefaultSelection::with_model("deepseek", "deepseek-chat")),
            model_aliases: BTreeMap::new(),
        };

        save_persisted_state(&mut conn, &original).unwrap();
        let loaded = load_persisted_state(&mut conn).unwrap();

        assert_eq!(loaded.providers.len(), 1);
        assert!(loaded.providers.contains_key("deepseek"));
        assert!(loaded.deleted.contains(&"openai".to_string()));
        assert_eq!(loaded.mode, ProviderMode::Direct { provider: "deepseek".into() });
        assert_eq!(loaded.default_selection, Some(DefaultSelection::with_model("deepseek", "deepseek-chat")));
    }

    #[test]
    fn load_settings_round_trips() {
        let (_dir, mut conn) = setup_db();
        let cfg = sebas_feishu::cards::CardConfig::default();
        save_settings(&mut conn, &cfg).unwrap();
        let loaded = load_settings(&mut conn).unwrap();
        assert!(loaded.is_some());
    }

    #[test]
    fn load_settings_absent_returns_none() {
        let (_dir, mut conn) = setup_db();
        let loaded = load_settings(&mut conn).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn projects_crud() {
        let (_dir, mut conn) = setup_db();
        let now = 1000;

        // 添加
        add_project(&mut conn, "/tmp/p1", "p1", now).unwrap();
        add_project(&mut conn, "/tmp/p2", "p2", now + 1).unwrap();

        let projects = load_projects(&mut conn).unwrap();
        assert_eq!(projects.len(), 2);

        // 删除
        assert!(remove_project(&mut conn, "/tmp/p1").unwrap());
        let projects = load_projects(&mut conn).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "p2");

        // 更新分支
        update_project_branch(&mut conn, "/tmp/p2", Some("main"), now + 10).unwrap();
        let projects = load_projects(&mut conn).unwrap();
        assert_eq!(projects[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn save_projects_replaces_all() {
        let (_dir, mut conn) = setup_db();
        let now = 1000;

        add_project(&mut conn, "/tmp/p1", "p1", now).unwrap();
        add_project(&mut conn, "/tmp/p2", "p2", now + 1).unwrap();

        // 全量替换
        save_projects(&mut conn, &[]).unwrap();
        let projects = load_projects(&mut conn).unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn update_persisted_state_rmw() {
        let (_dir, mut conn) = setup_db();
        use sebas_router::provider_state::ProviderMode;

        update_persisted_state(&mut conn, |s| {
            s.mode = ProviderMode::Gateway;
        }).unwrap();

        let state = load_persisted_state(&mut conn).unwrap();
        assert_eq!(state.mode, ProviderMode::Gateway);
    }
}