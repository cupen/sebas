//! 领域仓储层: 通过 `StateHandle` 提供类型化表访问。
//!
//! 每个方法接受 `&mut Connection` 并同步执行, 由 `StateHandle::exec` 调度到
//! 写者线程。所有方法都是纯同步的, 不依赖 tokio。

use crate::sebas_state::migration::TableSchema;
use crate::sebas_state::writer::StateHandle;
use rusqlite::{params, Connection};
use sebas_schema_derive::SchemaColumns;

// ---- Provider state ----

/// 从 DB 加载 provider 数据, 构造 `sebas_dispatch::state_store::PersistedState`。
///
/// 读取 providers 表(含软删) + model_aliases 表, 与 `provider_state.rs` 的
/// runtime 段合并。
pub fn load_persisted_state(conn: &mut Connection) -> Result<sebas_dispatch::state_store::PersistedState, String> {
    
    use sebas_dispatch::state_store::PersistedState;
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

        let mut providers: BTreeMap<String, sebas_dispatch::crud::Item> = BTreeMap::new();
        let mut deleted: Vec<String> = Vec::new();

        for row in rows {
            let (id, config, del) = row.map_err(|e| format!("读取 provider 行失败: {e}"))?;
            if del != 0 {
                deleted.push(id);
            } else {
                if let Ok(item) = serde_json::from_str::<sebas_dispatch::crud::Item>(&config) {
                    providers.insert(id, item);
                } else {
                    tracing::warn!("failed to parse provider {id} config JSON, skipping");
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

        let mut aliases: BTreeMap<String, sebas_dispatch::state_store::ModelAliasEntry> =
            BTreeMap::new();
        for row in rows {
            let (alias, provider, upstream_model) =
                row.map_err(|e| format!("读取 model_alias 行失败: {e}"))?;
            aliases.insert(
                alias,
                sebas_dispatch::state_store::ModelAliasEntry {
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
        version: sebas_dispatch::state_store::STATE_VERSION_V2,
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
) -> (sebas_dispatch::provider_state::ProviderMode, Option<sebas_dispatch::state_store::DefaultSelection>) {
    use sebas_dispatch::provider_state::ProviderMode;
    

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
                    tracing::warn!(error = %e, "failed to parse runtime_state, using defaults");
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
pub fn save_persisted_state(conn: &mut Connection, state: &sebas_dispatch::state_store::PersistedState) -> Result<(), String> {
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
    f: impl FnOnce(&mut sebas_dispatch::state_store::PersistedState),
) -> Result<sebas_dispatch::state_store::PersistedState, String> {
    let mut state = load_persisted_state(conn)?;
    f(&mut state);
    save_persisted_state(conn, &state)?;
    Ok(state)
}

// ---- Legacy defaults 一次性导入（make-core-own-provider-data 1.4）----

/// 导入标记是否在场（`settings` 表 `defaults_imported` 行）。在场即不再读
/// legacy defaults.json。
pub fn defaults_import_done(conn: &mut Connection) -> Result<bool, String> {
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'defaults_imported'",
            [],
            |row| row.get(0),
        )
        .ok();
    Ok(done.is_some())
}

/// 标记导入阶段完成（无值可导也落标记——阶段一次性，不每次启动重放）。
pub fn mark_defaults_imported(conn: &mut Connection) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('defaults_imported', '1')
         ON CONFLICT(key) DO UPDATE SET value = '1'",
        [],
    )
    .map_err(|e| format!("写入 defaults_imported 标记失败: {e}"))?;
    Ok(())
}

/// 导入默认值 + 落标记，**同一事务**完成（与 provider 数据同库同事务的
/// D3 语义）。已导入过 → Ok(false)，不覆盖库里的现值（用户后来的选择
/// 优先于 legacy 文件）。
pub fn import_defaults_once(
    conn: &mut Connection,
    selection: sebas_dispatch::state_store::DefaultSelection,
) -> Result<bool, String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("导入事务开始失败: {e}"))?;
    let done: Option<String> = tx
        .query_row(
            "SELECT value FROM settings WHERE key = 'defaults_imported'",
            [],
            |row| row.get(0),
        )
        .ok();
    if done.is_some() {
        return Ok(false);
    }
    // RMW runtime_state：只改 default_selection，mode 原样保留。
    let existing: Option<String> = tx
        .query_row(
            "SELECT value FROM settings WHERE key = 'runtime_state'",
            [],
            |row| row.get(0),
        )
        .ok();
    let mut row: RuntimeStateRow = existing
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    row.default_selection = Some(selection);
    let runtime_json = serde_json::to_string(&row)
        .map_err(|e| format!("序列化 runtime state 失败: {e}"))?;
    tx.execute(
        "INSERT INTO settings (key, value) VALUES ('runtime_state', ?1)
         ON CONFLICT(key) DO UPDATE SET value = ?1",
        params![runtime_json],
    )
    .map_err(|e| format!("写入 runtime state 失败: {e}"))?;
    tx.execute(
        "INSERT INTO settings (key, value) VALUES ('defaults_imported', '1')
         ON CONFLICT(key) DO UPDATE SET value = '1'",
        [],
    )
    .map_err(|e| format!("写入 defaults_imported 标记失败: {e}"))?;
    tx.commit()
        .map_err(|e| format!("导入事务提交失败: {e}"))?;
    Ok(true)
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
///
/// 同时是 projects 表结构的单一事实源 (sqlite-auto-schema-sync): 列由
/// `#[derive(SchemaColumns)]` 提取, 约束 (PRIMARY KEY/UNIQUE) 只在注册
/// 清单的 DDL 里表达。
#[derive(Debug, Clone, SchemaColumns, serde::Serialize, serde::Deserialize)]
pub struct ProjectRow {
    /// 稳定项目 id（`proj-<12hex>`；workbench-agent-wire-fix 2.4）。
    /// 迁移 2 之前的行读取时为 None，由应用层按 path 回填。
    pub id: Option<String>,
    pub path: String,
    pub name: String,
    pub default_agent: Option<String>,
    pub branch: Option<String>,
    #[column(default = "0")]
    pub branch_at: i64,
    pub added_at: i64,
    #[column(default = "0")]
    pub sort_order: i64,
}

/// 加载所有项目。
pub fn load_projects(conn: &mut Connection) -> Result<Vec<ProjectRow>, String> {
    let mut stmt = conn
        .prepare("SELECT id, path, name, default_agent, branch, branch_at, added_at, sort_order FROM projects ORDER BY sort_order, added_at")
        .map_err(|e| format!("准备 projects 查询失败: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(ProjectRow {
                id: row.get(0)?,
                path: row.get(1)?,
                name: row.get(2)?,
                default_agent: row.get(3)?,
                branch: row.get(4)?,
                branch_at: row.get(5)?,
                added_at: row.get(6)?,
                sort_order: row.get(7)?,
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
            "INSERT INTO projects (id, path, name, default_agent, branch, branch_at, added_at, sort_order) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![p.id, p.path, p.name, p.default_agent, p.branch, p.branch_at, p.added_at, p.sort_order],
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

/// 记录项目级默认 agent（workbench-agent-wire-fix 2.6），按稳定 id 定位。
pub fn set_project_default_agent(conn: &mut Connection, id: &str, agent: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE projects SET default_agent = ?2 WHERE id = ?1",
        params![id, agent],
    )
    .map_err(|e| format!("更新项目 {id} 默认 agent 失败: {e}"))?;
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

/// session_map 行 (chat_id, thread_id, session_id, last_active_unix, project_dir)。
/// struct 即表结构事实源 (sqlite-auto-schema-sync); 复合主键
/// `PRIMARY KEY (chat_id, thread_id)` 只在注册清单的 DDL 里表达。
#[derive(Debug, Clone, SchemaColumns)]
pub struct SessionMapRow {
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub session_id: String,
    pub last_active_unix: i64,
    pub project_dir: Option<String>,
}

/// 加载会话映射 (用于恢复)。
pub fn load_session_map(conn: &mut Connection) -> Result<Vec<SessionMapRow>, String> {
    let mut stmt = conn
        .prepare("SELECT chat_id, thread_id, session_id, last_active_unix, project_dir FROM session_map")
        .map_err(|e| format!("准备 session_map 查询失败: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SessionMapRow {
                chat_id: row.get(0)?,
                thread_id: row.get(1)?,
                session_id: row.get(2)?,
                last_active_unix: row.get(3)?,
                project_dir: row.get(4)?,
            })
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
    entries: &[SessionMapRow],
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("保存 session_map 事务开始失败: {e}"))?;

    tx.execute("DELETE FROM session_map", [])
        .map_err(|e| format!("清空 session_map 表失败: {e}"))?;

    for entry in entries {
        tx.execute(
            "INSERT INTO session_map (chat_id, thread_id, session_id, last_active_unix, project_dir) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![entry.chat_id, entry.thread_id, entry.session_id, entry.last_active_unix, entry.project_dir],
        )
        .map_err(|e| format!("写入 session_map 失败: {e}"))?;
    }

    tx.commit()
        .map_err(|e| format!("保存 session_map 事务提交失败: {e}"))?;
    Ok(())
}

// ---- Table schemas (sqlite-auto-schema-sync) ----

/// providers 行, 表结构的单一事实源。PRIMARY KEY 等约束只在下方注册清单的
/// DDL 里表达; `load_persisted_state` 的读取路径只取部分列, 其余字段仅为
/// schema 声明存在, 故整体 allow(dead_code)。
#[allow(dead_code)]
#[derive(Debug, Clone, SchemaColumns)]
pub struct ProviderRow {
    pub id: String,
    pub config: String,
    #[column(default = "0")]
    pub deleted: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// model_aliases 行, 表结构的单一事实源 (同上, 读取路径只取部分列)。
#[allow(dead_code)]
#[derive(Debug, Clone, SchemaColumns)]
pub struct ModelAliasRow {
    pub alias: String,
    pub provider: String,
    pub upstream_model: Option<String>,
    pub created_at: i64,
}

/// settings 行 (key-value), 表结构的单一事实源。
#[allow(dead_code)]
#[derive(Debug, Clone, SchemaColumns)]
pub struct SettingRow {
    pub key: String,
    pub value: String,
}

/// 五表注册清单: (表名, 首建/重建 DDL, 派生列) (sqlite-auto-schema-sync D2)。
///
/// - DDL 只在"建新库/重置"时执行, 与 v2 基线逐字对齐 (列名/类型/默认值/
///   约束/索引); 日常同步只对比派生列 vs `PRAGMA table_info`。
/// - 结构性约束 (PRIMARY KEY / UNIQUE / REFERENCES) 与索引只能表达在 DDL:
///   这类列在 struct 里没有非空默认, 缺列场景由 sync 判为不可原地补列 → 重置。
pub static REGISTERED_TABLES: &[TableSchema] = &[
    TableSchema {
        name: "providers",
        create_ddl: "CREATE TABLE providers (
            id          TEXT PRIMARY KEY,
            config      TEXT NOT NULL,       -- JSON blob
            deleted     INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL
        );
        CREATE INDEX idx_providers_deleted ON providers(deleted);",
        columns: ProviderRow::schema_columns(),
    },
    TableSchema {
        name: "model_aliases",
        create_ddl: "CREATE TABLE model_aliases (
            alias           TEXT PRIMARY KEY,
            provider        TEXT NOT NULL REFERENCES providers(id),
            upstream_model  TEXT,
            created_at      INTEGER NOT NULL
        );
        CREATE INDEX idx_model_aliases_provider ON model_aliases(provider);",
        columns: ModelAliasRow::schema_columns(),
    },
    TableSchema {
        name: "settings",
        create_ddl: "CREATE TABLE settings (
            key     TEXT PRIMARY KEY,
            value   TEXT NOT NULL    -- JSON blob
        );",
        columns: SettingRow::schema_columns(),
    },
    TableSchema {
        name: "projects",
        create_ddl: "CREATE TABLE projects (
            path        TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            branch      TEXT,
            branch_at   INTEGER NOT NULL DEFAULT 0,
            added_at    INTEGER NOT NULL,
            sort_order  INTEGER NOT NULL DEFAULT 0,
            -- workbench-agent-wire-fix 2.4: 迁移 2 追加的列, 放在末尾与 v2 布局一致
            id            TEXT,
            default_agent TEXT
        );
        CREATE UNIQUE INDEX idx_projects_id ON projects(id);",
        columns: ProjectRow::schema_columns(),
    },
    TableSchema {
        name: "session_map",
        create_ddl: "CREATE TABLE session_map (
            chat_id          TEXT NOT NULL,
            thread_id        TEXT,
            session_id       TEXT NOT NULL,
            last_active_unix INTEGER NOT NULL,
            project_dir      TEXT,
            PRIMARY KEY (chat_id, thread_id)
        );",
        columns: SessionMapRow::schema_columns(),
    },
];

// ---- Runtime state wire type ----

/// 运行时状态行 (mode + default_selection) 的 JSON 形状。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct RuntimeStateRow {
    #[serde(default)]
    mode: sebas_dispatch::provider_state::ProviderMode,
    #[serde(default)]
    default_selection: Option<sebas_dispatch::state_store::DefaultSelection>,
}

// ---- Async 包装 (通过 StateHandle) ----

/// 异步版本的 repo 操作, 通过 `StateHandle` 调度到写者线程。
pub struct Repo;

impl Repo {
    /// 加载 PersistedState。
    pub async fn load_persisted_state(handle: &StateHandle) -> Result<sebas_dispatch::state_store::PersistedState, String> {
        handle
            .exec(load_persisted_state)
            .await
    }

    /// 加载 settings。
    pub async fn load_settings(handle: &StateHandle) -> Result<Option<sebas_feishu::cards::CardConfig>, String> {
        handle
            .exec(load_settings)
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
            .exec(load_projects)
            .await
    }

    /// 保存所有项目。
    pub async fn save_projects(handle: &StateHandle, projects: Vec<ProjectRow>) -> Result<(), String> {
        handle
            .exec(move |conn| save_projects(conn, &projects))
            .await
    }

    /// 添加项目。
    pub async fn set_project_default_agent(handle: &StateHandle, id: String, agent: String) -> Result<(), String> {
        handle
            .exec(move |conn| set_project_default_agent(conn, &id, &agent))
            .await
    }

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
    use crate::sebas_state::migration::open_and_sync;
    use tempfile::tempdir;

    fn setup_db() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open_and_sync(&path).unwrap().0;
        (dir, conn)
    }

    #[test]
    fn load_empty_db_returns_default_state() {
        let (_dir, mut conn) = setup_db();
        let state = load_persisted_state(&mut conn).unwrap();
        assert!(state.providers.is_empty());
        assert!(state.deleted.is_empty());
        assert_eq!(state.mode, sebas_dispatch::provider_state::ProviderMode::Off);
        assert_eq!(state.default_selection, None);
    }

    #[test]
    fn save_and_load_provider_state_round_trips() {
        let (_dir, mut conn) = setup_db();
        use sebas_dispatch::provider_state::ProviderMode;
        use sebas_dispatch::state_store::{DefaultSelection, PersistedState};
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
        use sebas_dispatch::provider_state::ProviderMode;

        update_persisted_state(&mut conn, |s| {
            s.mode = ProviderMode::Router;
        }).unwrap();

        let state = load_persisted_state(&mut conn).unwrap();
        assert_eq!(state.mode, ProviderMode::Router);
    }
}