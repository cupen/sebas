//! 状态库域注册表 + PersistedState / settings 域的存储侧胶水。
//!
//! extract-sebas-db D2：域 schema 事实（哪张表、什么约束的手写 DDL）留在
//! 根 crate；行 struct 与标准 CRUD 已迁 [`sebas_models`]（ActiveRecord，
//! 一表一 struct）；连接/同步/单写 actor 在 [`sebas_db`]。本文件剩下的
//! 自由函数只覆盖**引用角色类型的胶水**——`PersistedState`（sebas-dispatch）
//! 与 `CardConfig`（sebas-feishu）进不了 sebas-models 的依赖清单。
//!
//! single-state-dir D2/D5：core 的库按**增长特征**分成两个——
//! [`SETTINGS_TABLES`]（providers / model_aliases / settings，有界系统配置）
//! 与 [`PROJECTS_TABLES`]（projects / session_map，增长的用户数据）。两库
//! 各开一次连接、各有一份注册表与版本戳；`save_persisted_state` 的三表
//! 事务全部落在 settings.db 内，**没有任何写事务横跨两库**（design D4
//! 枚举表）。项目/会话映射的存储胶水在 `projects_glue`（经
//! `sebas_models::project` 的域查询与 ActiveRecord）。

use rusqlite::Connection;
use sebas_db::schema::TableSchema;
use sebas_models::provider::{ModelAliasRow, ProviderRow};
use sebas_models::setting::SettingRow;
use sebas_models::runtime_state::{self, RuntimeStateRow};

// ---- Table schemas (sqlite-auto-schema-sync) ----

/// settings.db 注册清单（有界系统配置）：providers / model_aliases /
/// settings。DDL 只在"建新库/覆盖式重建"时执行, 与注册列逐一对应; 日常同步
/// 只对比派生列 vs `PRAGMA table_info`。建表段与索引段分开注册
/// （retire-schema-reset D3：覆盖式重建要以临时表名重组建表段）；providers
/// 已扁平化为类型化列（single-state-dir 3.2，列名与既有 provider JSON 键对齐）。
pub static SETTINGS_TABLES: &[TableSchema] = &[
    TableSchema {
        name: "providers",
        create_table_ddl: "CREATE TABLE providers (
            id          TEXT PRIMARY KEY,
            name        TEXT,
            preset      TEXT,
            base_url_anthropic        TEXT,
            base_url_openai_chat      TEXT,
            base_url_openai_responses TEXT,
            api_key     TEXT,
            api_key_env TEXT,
            default_model TEXT,
            protocol    TEXT,
            models      TEXT,            -- JSON 文本（id/tags 条目数组）
            model_map   TEXT,            -- JSON 文本（对象）
            deleted     INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL
        );",
        index_ddls: &["CREATE INDEX idx_providers_deleted ON providers(deleted);"],
        columns: sebas_models::provider::ProviderRow::schema_columns(),
    },
    TableSchema {
        name: "model_aliases",
        create_table_ddl: "CREATE TABLE model_aliases (
            alias           TEXT PRIMARY KEY,
            provider        TEXT NOT NULL REFERENCES providers(id),
            upstream_model  TEXT,
            created_at      INTEGER NOT NULL
        );",
        index_ddls: &["CREATE INDEX idx_model_aliases_provider ON model_aliases(provider);"],
        columns: sebas_models::provider::ModelAliasRow::schema_columns(),
    },
    TableSchema {
        name: "settings",
        create_table_ddl: "CREATE TABLE settings (
            key     TEXT PRIMARY KEY,
            value   TEXT NOT NULL    -- JSON blob
        );",
        index_ddls: &[],
        columns: sebas_models::setting::SettingRow::schema_columns(),
    },
];

/// projects.db 注册清单（增长的用户数据）：projects / session_map。表形状
/// 不变（`migrate-project-registry` 负责目标形状重建；本 change 只换库）。
pub static PROJECTS_TABLES: &[TableSchema] = &[
    TableSchema {
        // migrate-project-registry 1.1：按目标形状重建。节点维度是正式列
        // （`node_id` NOT NULL DEFAULT 'local'）——**旧形状库不走重置**：
        // 有自描述版本键的库缺列由启动同步原地补齐，旧行的 node_id 取默认
        // 'local' 即自动归到本机节点（无迁移脚本；实测见
        // tests/persistence_runtime_integration_test.rs 的
        // added_column_reads_back_at_default_through_generated_crud）。更老的
        // 迁移链库同样由结构 reconcile 吸纳（retire-schema-reset：任何路径都
        // 不删库）。列顺序与 `ProjectRow::COLUMNS` 一致（同步以注册列 diff，
        // 非 DDL 文本）。主键 `path` 与 `id` 唯一索引语义不变（1.3）。
        name: "projects",
        create_table_ddl: "CREATE TABLE projects (
            id          TEXT,
            path        TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            branch_at   INTEGER NOT NULL DEFAULT 0,
            added_at    INTEGER NOT NULL,
            sort_order  INTEGER NOT NULL DEFAULT 0,
            node_id       TEXT NOT NULL DEFAULT 'local',
            default_agent TEXT,
            branch        TEXT
        );",
        index_ddls: &["CREATE UNIQUE INDEX idx_projects_id ON projects(id);"],
        columns: sebas_models::project::ProjectRow::schema_columns(),
    },
    TableSchema {
        // persist-session-map 1.1：按会话映射的完整目标形状重建。键列 =
        // ChannelKey 身份（chat_id = channel，thread_id = reference），主键
        // 仍为 `(chat_id, thread_id)`（1.3：按会话键寻址语义不变）；其余列
        // 承载 Mapping 的持久化字段（desired_mode / awaiting_first_prompt
        // 非空且无常量默认——旧形状库打开时按 fail-closed 拒启动并点名表列，
        // 见 sebas_models::session_map 模块文档）。
        name: "session_map",
        create_table_ddl: "CREATE TABLE session_map (
            chat_id               TEXT NOT NULL,
            thread_id             TEXT,
            session_id            TEXT NOT NULL,
            last_active_unix      INTEGER NOT NULL,
            project_dir           TEXT,
            acp_session_id        TEXT,
            current_model         TEXT,
            pending_kind          TEXT,
            pending_model         TEXT,
            pending_mode          TEXT,
            desired_mode          TEXT NOT NULL,
            label                 TEXT,
            prompt_preview        TEXT,
            awaiting_first_prompt INTEGER NOT NULL,
            PRIMARY KEY (chat_id, thread_id)
        );",
        index_ddls: &[],
        columns: sebas_models::session_map::SessionMapRow::schema_columns(),
    },
];

// ---- PersistedState 域的存储侧胶水（settings.db）----

/// 从 settings.db 加载 provider 数据, 构造
/// `sebas_dispatch::state_store::PersistedState`。
///
/// 读取 providers 表(含软删) + model_aliases 表 + settings 的 runtime 状态,
/// 经 `sebas_models` 的行 struct 出入。providers 的 wire 形状经
/// `ProviderRow::to_item` 还原（类型化列 ↔ JSON 键一一对应）。
pub fn load_persisted_state(
    conn: &mut Connection,
) -> Result<sebas_dispatch::state_store::PersistedState, String> {
    use sebas_dispatch::state_store::PersistedState;
    use std::collections::BTreeMap;

    // 读 providers 表 (含软删)——行经 ProviderRow，wire 形状经 to_item。
    let mut providers: BTreeMap<String, sebas_dispatch::state_store::Item> = BTreeMap::new();
    let mut deleted: Vec<String> = Vec::new();
    for row in ProviderRow::all(conn).map_err(|e| format!("查询 providers 失败: {e}"))? {
        if row.deleted != 0 {
            deleted.push(row.id);
        } else {
            let item = row.to_item();
            providers.insert(row.id, item);
        }
    }

    // 读 model_aliases——行经 ModelAliasRow（BTreeMap 吸收顺序差异）。
    let mut model_aliases: BTreeMap<String, sebas_dispatch::state_store::ModelAliasEntry> =
        BTreeMap::new();
    for row in
        ModelAliasRow::all(conn).map_err(|e| format!("查询 model_aliases 失败: {e}"))?
    {
        model_aliases.insert(
            row.alias,
            sebas_dispatch::state_store::ModelAliasEntry {
                provider: row.provider,
                upstream_model: row.upstream_model,
            },
        );
    }

    // 读 settings 中的 mode/default_selection (如果存在)
    let (mode, default_selection) = runtime_state::load_runtime_state(conn);

    Ok(PersistedState {
        version: sebas_dispatch::state_store::STATE_VERSION_V2,
        providers,
        deleted,
        mode,
        default_selection,
        model_aliases,
    })
}

/// 保存 PersistedState 到 settings.db。
///
/// 写入 providers 表 (upsert + 软删) + model_aliases + 运行时状态到 settings
/// 表，全部经 ActiveRecord 的 `save()`，**单事务单连接**——providers/
/// aliases/runtime_state 同属有界系统配置，事务完整地落在 settings 库内，
/// 不触达 projects.db（design D3/D4，任务 2.3 有单测钉住）。
pub fn save_persisted_state(
    conn: &mut Connection,
    state: &sebas_dispatch::state_store::PersistedState,
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("保存状态事务开始失败: {e}"))?;

    // 清空旧数据（先清 aliases 再清 providers——`model_aliases.provider`
    // 有 REFERENCES providers(id) 外键，FK ON 时顺序不能反）。
    tx.execute("DELETE FROM model_aliases", [])
        .map_err(|e| format!("清空 model_aliases 表失败: {e}"))?;
    tx.execute("DELETE FROM providers", [])
        .map_err(|e| format!("清空 providers 表失败: {e}"))?;

    let now = sebas_domain::prim::now_unix();

    // 写 providers (非软删)——Item 经 from_item 折进类型化列。
    for (id, item) in &state.providers {
        ProviderRow::from_item(id, item)
            .save(&tx)
            .map_err(|e| format!("写入 provider {id} 失败: {e}"))?;
    }

    // 写 deleted providers (软删)——墓碑行只留 id。
    for id in &state.deleted {
        let mut row = ProviderRow::from_item(id, &serde_json::Map::new());
        row.deleted = 1;
        row.save(&tx)
            .map_err(|e| format!("写入 deleted provider {id} 失败: {e}"))?;
    }

    // 写 model_aliases (add-state-store 5.3：随状态库流转)
    for (alias, entry) in &state.model_aliases {
        ModelAliasRow {
            alias: alias.clone(),
            provider: entry.provider.clone(),
            upstream_model: entry.upstream_model.clone(),
            created_at: now,
        }
        .save(&tx)
        .map_err(|e| format!("写入 model alias {alias} 失败: {e}"))?;
    }

    // 写 runtime state
    let runtime = RuntimeStateRow {
        mode: state.mode.clone(),
        default_selection: state.default_selection.clone(),
    };
    runtime_state::save_runtime_state(&tx, &runtime)?;

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

// ---- Settings (CardConfig 域) ----

/// 加载 settings (CardConfig), 从 `settings` 表 `key = 'card_config'`。
pub fn load_settings(
    conn: &mut Connection,
) -> Result<Option<sebas_feishu::cards::CardConfig>, String> {
    let row = SettingRow::find(conn, "card_config")
        .map_err(|e| format!("读取 card_config 失败: {e}"))?;
    match row {
        Some(row) => serde_json::from_str(&row.value)
            .map(Some)
            .map_err(|e| format!("settings JSON 解析失败: {e}")),
        None => Ok(None),
    }
}

/// 保存 settings (CardConfig)。
pub fn save_settings(
    conn: &mut Connection,
    cfg: &sebas_feishu::cards::CardConfig,
) -> Result<(), String> {
    let value = serde_json::to_string(cfg).map_err(|e| format!("序列化 settings 失败: {e}"))?;
    SettingRow {
        key: "card_config".to_string(),
        value,
    }
    .save(conn)
    .map_err(|e| format!("写入 settings 失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use sebas_db::schema::open_and_sync;
    use sebas_dispatch::provider_state::ProviderMode;
    use sebas_dispatch::state_store::{DefaultSelection, PersistedState};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    /// settings.db（三表注册表）连接。
    fn settings_db() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let conn = open_and_sync(&dir.path().join("settings.db"), SETTINGS_TABLES)
            .unwrap()
            .0;
        (dir, conn)
    }

    /// projects.db（两表注册表）连接。
    fn projects_db() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let conn = open_and_sync(&dir.path().join("projects.db"), PROJECTS_TABLES)
            .unwrap()
            .0;
        (dir, conn)
    }

    /// `migrate-project-registry` 1.3：`projects` 表的物理形状与约束钉——
    /// 既有 8 列一列不少、主键仍是 `path`、`id` 唯一、新增的 `node_id` 是正式
    /// 列（`NOT NULL DEFAULT 'local'`，旧行取默认即自动归本机）。
    ///
    /// 走**行为**断言而不是内省 SQLite 的列元数据表（表结构 diff 原语）：那是
    /// 持久层 runtime 的能力，只允许出现在 `sebas-db`（机械守卫见
    /// `tests/persistence_runtime_test.rs::table_diffing_exists_only_in_sebas_db`）。
    /// 列清单与顺序由 [`registered_tables_match_baseline_columns`]（注册基线）
    /// 与 `generated_upsert_sql_matches_golden_samples`（生成 SQL 的逐字列序）
    /// 钉住；这里钉「SQLite 真建出来的表怎么表现」。
    #[test]
    fn projects_table_shape_and_primary_key_are_pinned() {
        let (_dir, conn) = projects_db();
        let count = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
                .unwrap()
        };

        // 既有 8 列 + node_id 全部可按名写入：缺任何一列这条 INSERT 就会报
        // 「no such column」——「一列不少」的行为等价断言。
        conn.execute(
            "INSERT INTO projects \
             (id, path, name, branch_at, added_at, sort_order, node_id, default_agent, branch) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                "proj-legacy0001",
                "/w/legacy",
                "legacy",
                7i64,
                11i64,
                3i64,
                "local",
                "claude",
                "main"
            ],
        )
        .expect("既有 8 列 + node_id 必须都建出来了");
        assert_eq!(count(&conn), 1);
        // 写进去的 8 个既有列可按名原样读回（列名拼写不变）。
        let (name, branch, sort_order, agent): (String, String, i64, String) = conn
            .query_row(
                "SELECT name, branch, sort_order, default_agent FROM projects WHERE path = '/w/legacy'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), branch.as_str(), sort_order, agent.as_str()),
            ("legacy", "main", 3, "claude"),
            "既有列必须原样往返"
        );

        // 主键是 path：同 path 再插一行（不同 id）必须冲突。
        let dup_path = conn.execute(
            "INSERT INTO projects (id, path, name, added_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["proj-other0001", "/w/legacy", "again", 1i64],
        );
        assert!(dup_path.is_err(), "path 必须是主键（同 path 不得并存）");

        // id 唯一：同 id 不同 path 必须冲突（idx_projects_id 仍是唯一索引）。
        let dup_id = conn.execute(
            "INSERT INTO projects (id, path, name, added_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["proj-legacy0001", "/w/other", "other", 1i64],
        );
        assert!(dup_id.is_err(), "id 必须唯一（idx_projects_id 唯一索引）");

        // node_id 有默认 'local'：省略该列的插入成功，读回本机节点。
        conn.execute(
            "INSERT INTO projects (id, path, name, added_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["proj-default0001", "/w/defaulted", "defaulted", 2i64],
        )
        .expect("省略 node_id 的插入应成功（该列有默认值）");
        let node: String = conn
            .query_row(
                "SELECT node_id FROM projects WHERE path = '/w/defaulted'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(node, "local", "node_id 默认值必须是 local（旧行自动归本机）");

        // node_id NOT NULL：显式 NULL 必须被拒。
        let null_node = conn.execute(
            "INSERT INTO projects (id, path, name, added_at, node_id) \
             VALUES (?1, ?2, ?3, ?4, NULL)",
            rusqlite::params!["proj-null0001", "/w/nullnode", "nullnode", 3i64],
        );
        assert!(null_node.is_err(), "node_id 必须 NOT NULL");

        // 其余必填列语义不变：name 显式 NULL 被拒。
        assert!(
            conn.execute(
                "INSERT INTO projects (id, path, name, added_at) VALUES (?1, ?2, NULL, ?3)",
                rusqlite::params!["proj-name0001", "/w/noname", 4i64],
            )
            .is_err(),
            "name 必须 NOT NULL"
        );
    }

    #[test]
    fn load_empty_db_returns_default_state() {
        let (_dir, mut conn) = settings_db();
        let state = load_persisted_state(&mut conn).unwrap();
        assert!(state.providers.is_empty());
        assert!(state.deleted.is_empty());
        assert_eq!(state.mode, ProviderMode::Off);
        assert_eq!(state.default_selection, None);
    }

    /// 3.2 验收：providers 的 wire 形状（Item）经 save/load 往返与写入时
    /// 一致——类型化列对 channel 形状透明。
    #[test]
    fn save_and_load_provider_state_round_trips_wire_shape() {
        let (_dir, mut conn) = settings_db();

        let item = serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(
            serde_json::json!({
                "name": "deepseek",
                "preset": "deepseek",
                "base_url_openai_chat": "https://api.deepseek.com",
                "api_key": "sk-x",
                "models": [{"id": "deepseek-chat", "tags": []}],
            }),
        )
        .unwrap();

        let original = PersistedState {
            version: 2,
            providers: BTreeMap::from([("deepseek".into(), item)]),
            deleted: vec!["openai".into()],
            mode: ProviderMode::Direct {
                provider: "deepseek".into(),
            },
            default_selection: Some(DefaultSelection::with_model("deepseek", "deepseek-chat")),
            model_aliases: BTreeMap::new(),
        };

        save_persisted_state(&mut conn, &original).unwrap();
        let loaded = load_persisted_state(&mut conn).unwrap();

        assert_eq!(loaded.providers.len(), 1);
        assert_eq!(
            loaded.providers.get("deepseek"),
            original.providers.get("deepseek"),
            "provider wire 形状经类型化列往返逐字段一致"
        );
        assert!(loaded.deleted.contains(&"openai".to_string()));
        assert_eq!(
            loaded.mode,
            ProviderMode::Direct {
                provider: "deepseek".into()
            }
        );
        assert_eq!(
            loaded.default_selection,
            Some(DefaultSelection::with_model("deepseek", "deepseek-chat"))
        );
    }

    #[test]
    fn load_settings_round_trips() {
        let (_dir, mut conn) = settings_db();
        let cfg = sebas_feishu::cards::CardConfig::default();
        save_settings(&mut conn, &cfg).unwrap();
        let loaded = load_settings(&mut conn).unwrap();
        assert!(loaded.is_some());
    }

    #[test]
    fn load_settings_absent_returns_none() {
        let (_dir, mut conn) = settings_db();
        let loaded = load_settings(&mut conn).unwrap();
        assert!(loaded.is_none());
    }

    /// 3.1 验收：projects / session_map 在 projects.db（PROJECTS_TABLES）
    /// 上照常工作——两库拆分后项目域的存储胶水不依赖 settings 库。
    #[test]
    fn projects_crud_on_projects_db() {
        let (_dir, mut conn) = projects_db();
        let now = 1000;

        // 添加
        sebas_models::project::add_project(&mut conn, "local", "/tmp/p1", "p1", now).unwrap();
        sebas_models::project::add_project(&mut conn, "local", "/tmp/p2", "p2", now + 1).unwrap();

        let projects = sebas_models::project::load_projects(&mut conn).unwrap();
        assert_eq!(projects.len(), 2);

        // 删除
        assert!(sebas_models::project::remove_project(&mut conn, "/tmp/p1").unwrap());
        let projects = sebas_models::project::load_projects(&mut conn).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "p2");

        // 更新分支
        sebas_models::project::update_project_branch(&mut conn, "/tmp/p2", Some("main"), now + 10)
            .unwrap();
        let projects = sebas_models::project::load_projects(&mut conn).unwrap();
        assert_eq!(projects[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn save_projects_replaces_all() {
        let (_dir, mut conn) = projects_db();
        let now = 1000;

        sebas_models::project::add_project(&mut conn, "local", "/tmp/p1", "p1", now).unwrap();
        sebas_models::project::add_project(&mut conn, "local", "/tmp/p2", "p2", now + 1).unwrap();

        // 全量替换
        sebas_models::project::save_projects(&mut conn, &[]).unwrap();
        let projects = sebas_models::project::load_projects(&mut conn).unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn update_persisted_state_rmw() {
        let (_dir, mut conn) = settings_db();

        update_persisted_state(&mut conn, |s| {
            s.mode = ProviderMode::Router;
        })
        .unwrap();

        let state = load_persisted_state(&mut conn).unwrap();
        assert_eq!(state.mode, ProviderMode::Router);
    }

    // ---- 任务 2.3 验收：save_persisted_state 只触达 settings 库的连接 ----

    /// 该事务运行在**只有三张 settings 表**的连接上：若它引用 projects /
    /// session_map，语句会因缺表当场失败——「事务只触达 settings 库的连接」
    /// 由这个构造直接钉住（design D4：providers + model_aliases +
    /// runtime_state 同落 settings.db，不跨库）。
    #[test]
    fn save_persisted_state_touches_only_the_settings_connection() {
        let (dir, mut conn) = settings_db();

        let state = PersistedState {
            version: 2,
            providers: BTreeMap::from([(
                "deepseek".into(),
                serde_json::from_value(serde_json::json!({"preset": "deepseek"})).unwrap(),
            )]),
            deleted: vec!["openai".into()],
            mode: ProviderMode::Router,
            default_selection: Some(DefaultSelection::new("deepseek")),
            model_aliases: BTreeMap::from([(
                "my-claude".into(),
                sebas_dispatch::state_store::ModelAliasEntry {
                    provider: "deepseek".into(),
                    upstream_model: Some("claude-sonnet-4".into()),
                },
            )]),
        };
        save_persisted_state(&mut conn, &state).unwrap();

        // 三张表都有数据；同连接上不存在任何 projects 域表。
        // providers = 1 行在用 + 1 行墓碑（deleted provider）。
        let providers: i64 = conn
            .query_row("SELECT COUNT(*) FROM providers", [], |r| r.get(0))
            .unwrap();
        let aliases: i64 = conn
            .query_row("SELECT COUNT(*) FROM model_aliases", [], |r| r.get(0))
            .unwrap();
        assert_eq!(providers, 2, "1 行在用 + 1 行墓碑");
        assert_eq!(aliases, 1);
        let runtime = runtime_state::load_runtime_state(&mut conn);
        assert_eq!(runtime.0, ProviderMode::Router);
        let project_tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' \
                 AND name IN ('projects','session_map')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(project_tables, 0, "settings 库里没有 projects 域表");
        let _ = dir;
    }

    // ---- 3.3 验收（跨库半边）：projects.db 的结构漂移不影响 settings.db ----

    /// projects.db 因 schema 漂移走原位迁移（多余列 DROP）后，settings.db 的值
    /// 原样健在（spec「resetting user data leaves configuration intact」的
    /// retire-schema-reset 形态：漂移不再删库，另一库更不受牵连）。
    #[test]
    fn drifting_projects_db_leaves_settings_db_intact() {
        let sdir = tempdir().unwrap();
        let pdir = tempdir().unwrap();
        let settings_path = sdir.path().join("settings.db");
        let projects_path = pdir.path().join("projects.db");

        // 两库各开一次并各写一份数据。
        {
            let (mut conn, _) = open_and_sync(&settings_path, SETTINGS_TABLES).unwrap();
            save_settings(&mut conn, &sebas_feishu::cards::CardConfig::default()).unwrap();
        }
        {
            let (mut conn, _) = open_and_sync(&projects_path, PROJECTS_TABLES).unwrap();
            sebas_models::project::add_project(&mut conn, "local", "/tmp/p", "p", 1).unwrap();
        }

        // 模拟 projects.db 的结构漂移（多余列 → 下次 open 原地 DROP，不删库）。
        {
            let conn = sebas_db::conn::open(&projects_path).unwrap();
            conn.execute_batch("ALTER TABLE projects ADD COLUMN stale TEXT;")
                .unwrap();
        }

        // 重开 projects.db → 原位迁移：删掉多余列，数据保住，库文件不删。
        let (conn, outcome) = open_and_sync(&projects_path, PROJECTS_TABLES).unwrap();
        assert_eq!(
            outcome,
            sebas_db::schema::SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 0,
                dropped: 1,
            },
            "结构漂移应原位删列（不重置），实际 {outcome:?}"
        );
        let projects: i64 = conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(projects, 1, "projects.db 的存量行随原位迁移存活");
        drop(conn);
        assert!(projects_path.exists(), "库文件绝不被删除");
        let backups: Vec<_> = std::fs::read_dir(pdir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().ends_with(sebas_db::schema::BACKUP_SUFFIX))
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(backups.len(), 1, "破坏性迁移前留有 .pre-sync 备份: {backups:?}");

        // settings.db 完全不受影响：重开后配置仍在。
        let (mut conn, outcome) = open_and_sync(&settings_path, SETTINGS_TABLES).unwrap();
        assert_eq!(
            outcome,
            sebas_db::schema::SyncOutcome::UpToDate,
            "settings.db 未被牵连"
        );
        assert!(
            load_settings(&mut conn).unwrap().is_some(),
            "settings 在 projects.db 原位迁移后存活"
        );
    }

    // ---- registered_tables_match_v2_baseline_columns（4.5 基线的拆库版）----

    #[test]
    fn registered_tables_match_baseline_columns() {
        // 派生列与基线对齐的护栏: 列名 + 亲和逐一核对。
        // providers 基线是 single-state-dir 3.2 的扁平化列（与 provider
        // JSON 键对齐）；其余四表与迁移前逐字一致。
        let settings: &[(&str, &[(&str, &str)])] = &[
            (
                "providers",
                &[
                    ("id", "TEXT"),
                    ("name", "TEXT"),
                    ("preset", "TEXT"),
                    ("base_url_anthropic", "TEXT"),
                    ("base_url_openai_chat", "TEXT"),
                    ("base_url_openai_responses", "TEXT"),
                    ("api_key", "TEXT"),
                    ("api_key_env", "TEXT"),
                    ("default_model", "TEXT"),
                    ("protocol", "TEXT"),
                    ("models", "TEXT"),
                    ("model_map", "TEXT"),
                    ("deleted", "INTEGER"),
                    ("created_at", "INTEGER"),
                    ("updated_at", "INTEGER"),
                ],
            ),
            (
                "model_aliases",
                &[
                    ("alias", "TEXT"),
                    ("provider", "TEXT"),
                    ("upstream_model", "TEXT"),
                    ("created_at", "INTEGER"),
                ],
            ),
            ("settings", &[("key", "TEXT"), ("value", "TEXT")]),
        ];
        let projects: &[(&str, &[(&str, &str)])] = &[
            (
                "projects",
                &[
                    ("id", "TEXT"),
                    ("path", "TEXT"),
                    ("name", "TEXT"),
                    ("branch_at", "INTEGER"),
                    ("added_at", "INTEGER"),
                    ("sort_order", "INTEGER"),
                    ("node_id", "TEXT"),
                    ("default_agent", "TEXT"),
                    ("branch", "TEXT"),
                ],
            ),
            (
                "session_map",
                &[
                    ("chat_id", "TEXT"),
                    ("thread_id", "TEXT"),
                    ("session_id", "TEXT"),
                    ("last_active_unix", "INTEGER"),
                    ("project_dir", "TEXT"),
                    ("acp_session_id", "TEXT"),
                    ("current_model", "TEXT"),
                    ("pending_kind", "TEXT"),
                    ("pending_model", "TEXT"),
                    ("pending_mode", "TEXT"),
                    ("desired_mode", "TEXT"),
                    ("label", "TEXT"),
                    ("prompt_preview", "TEXT"),
                    ("awaiting_first_prompt", "INTEGER"),
                ],
            ),
        ];
        for (registry, tables) in [(SETTINGS_TABLES, settings), (PROJECTS_TABLES, projects)] {
            for (table, cols) in tables {
                let reg = registry.iter().find(|t| t.name == *table).unwrap();
                assert_eq!(reg.columns.len(), cols.len(), "表 {table} 列数不一致");
                for (col, (name, affinity)) in reg.columns.iter().zip(*cols) {
                    assert_eq!(col.name, *name, "表 {table} 列名不一致");
                    assert_eq!(col.affinity, *affinity, "表 {table} 列 {name} 亲和不一致");
                }
            }
        }
    }

    /// 分层不变量：两份注册表各管各的表，无交叠、合并后恰好六表
    /// （settings 三表 + projects 两表，schema_meta 由 runtime 自管）。
    #[test]
    fn registries_are_disjoint_and_complete() {
        let mut seen = Vec::new();
        for t in SETTINGS_TABLES.iter().chain(PROJECTS_TABLES.iter()) {
            assert!(!seen.contains(&t.name), "表 {} 被注册了两次", t.name);
            seen.push(t.name);
        }
        for expected in ["providers", "model_aliases", "settings"] {
            assert!(SETTINGS_TABLES.iter().any(|t| t.name == expected));
        }
        for expected in ["projects", "session_map"] {
            assert!(PROJECTS_TABLES.iter().any(|t| t.name == expected));
        }
    }
    // ---- retire-schema-reset 2.2：拆段后首建布局逐字不变 ----

    /// 新库 `sqlite_master` 的每条注册 DDL（建表段 + 索引段）与注册字面量
    /// **逐字一致**——拆分前抓取的黄金就是这些字面量（建表段 = 原 `create_ddl`
    /// 的表语句逐字，索引段 = 原尾部索引语句逐字），且语句集合里除 runtime 自建
    /// 的 `schema_meta` 外无任何多余记录。防注册重构悄悄变形。
    #[test]
    fn fresh_db_sqlite_master_layout_matches_registered_ddl_verbatim() {
        use std::collections::BTreeSet;

        // SQLite 存语句原文：去掉首尾空白与结尾分号后应与注册字面量逐字相等。
        fn normalize(ddl: &str) -> String {
            ddl.trim().trim_end_matches(';').trim_end().to_string()
        }

        let dir = tempdir().unwrap();
        for (name, tables) in [
            ("settings.db", SETTINGS_TABLES),
            ("projects.db", PROJECTS_TABLES),
        ] {
            let (conn, _) = open_and_sync(&dir.path().join(name), tables).unwrap();
            let mut stmt = conn
                .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL")
                .unwrap();
            let stored: BTreeSet<String> = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .flatten()
                .map(|s| normalize(&s))
                .collect();

            let mut expected: BTreeSet<String> = BTreeSet::new();
            for table in tables {
                assert!(
                    stored.contains(&normalize(table.create_table_ddl)),
                    "{name}: 建表段被组装变形: {}",
                    table.name
                );
                expected.insert(normalize(table.create_table_ddl));
                for index_ddl in table.index_ddls {
                    assert!(
                        stored.contains(&normalize(index_ddl)),
                        "{name}: 索引段被组装变形: {index_ddl}"
                    );
                    expected.insert(normalize(index_ddl));
                }
            }
            // 只多出一条 runtime 自建的 schema_meta（小写 DDL，不进域注册表）。
            assert_eq!(
                stored.len(),
                expected.len() + 1,
                "{name}: 首建语句集合与注册清单不一致: {stored:?}"
            );
        }
    }

    // ---- retire-schema-reset 5.4：两个分层库各自独立适用迁移 ----

    /// 以真实注册表为底派生一份「把某列声明为 `rename_from` 旧列名」的注册表
    /// （DDL/索引段原样复用——改名不改类型，注册 DDL 仍自洽）。测试专用：
    /// `Box::leak` 换 `'static` 是 test-only 的便利。
    fn registry_with_renamed_column(
        base: &[TableSchema],
        table: &str,
        column: &str,
        old: &str,
    ) -> &'static [TableSchema] {
        use sebas_db::schema::SchemaColumn;

        let mut tables: Vec<TableSchema> = base.to_vec();
        for t in &mut tables {
            if t.name != table {
                continue;
            }
            let mut cols: Vec<SchemaColumn> = t.columns.to_vec();
            for c in &mut cols {
                if c.name == column {
                    c.rename_from = Some(Box::leak(old.to_string().into_boxed_str()));
                }
            }
            t.columns = Box::leak(cols.into_boxed_slice());
        }
        Box::leak(tables.into_boxed_slice())
    }

    fn snapshot(path: &std::path::Path) -> (Vec<u8>, std::time::SystemTime) {
        (
            std::fs::read(path).unwrap(),
            std::fs::metadata(path).unwrap().modified().unwrap(),
        )
    }

    /// 列名清单（经语句元数据读，不用表结构 diff 原语——那些原语只允许在
    /// `sebas-db` 出现，见 tests/persistence_runtime_test.rs 的机械门禁）。
    fn sqlite_columns(conn: &Connection, table: &str) -> Vec<String> {
        let stmt = conn
            .prepare(&format!("SELECT * FROM {table} LIMIT 0"))
            .unwrap();
        stmt.column_names().into_iter().map(String::from).collect()
    }

    fn temp_file_count(dir: &std::path::Path, suffix: &str) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .ends_with(suffix)
            })
            .count()
    }

    /// settings.db 与 projects.db 各独立走一轮「改名 + 类型变更 + 删列」：
    /// 每个库自己的结构差异都在**本库内**原位迁移，另一个库的文件逐字节、
    /// mtime 逐项未变（retire-schema-reset：分层库各自适用、互不触碰）。
    #[test]
    fn layered_databases_migrate_independently_and_never_touch_each_other() {
        let dir = tempdir().unwrap();
        let settings_path = dir.path().join("settings.db");
        let projects_path = dir.path().join("projects.db");

        // 两库 fresh 建好并各写真实域数据。
        {
            let (conn, _) = open_and_sync(&settings_path, SETTINGS_TABLES).unwrap();
            conn.execute(
                "INSERT INTO providers (id, name, deleted, created_at, updated_at)
                 VALUES ('p1', 'keep', 0, 1, 1)",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO settings (key, value) VALUES ('k', 'v')", [])
                .unwrap();
        }
        {
            let (conn, _) = open_and_sync(&projects_path, PROJECTS_TABLES).unwrap();
            conn.execute(
                "INSERT INTO projects
                 (id, path, name, branch_at, added_at, sort_order, node_id)
                 VALUES ('id1', '/p', 'proj', 0, 1, 0, 'local')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO session_map
                 (chat_id, thread_id, session_id, last_active_unix, project_dir,
                  acp_session_id, current_model, pending_kind, pending_model, pending_mode,
                  desired_mode, label, prompt_preview, awaiting_first_prompt)
                 VALUES ('web', NULL, 's1', 5, NULL, NULL, NULL, NULL, NULL, NULL,
                         'auto', NULL, NULL, 0)",
                [],
            )
            .unwrap();
        }

        let settings_tables =
            registry_with_renamed_column(SETTINGS_TABLES, "providers", "name", "title");
        let projects_tables =
            registry_with_renamed_column(PROJECTS_TABLES, "projects", "name", "title");

        // ---- settings.db：改名（providers.name ← title）----
        {
            let conn = sebas_db::conn::open(&settings_path).unwrap();
            conn.execute_batch("ALTER TABLE providers RENAME COLUMN name TO title;")
                .unwrap();
        }
        let projects_before = snapshot(&projects_path);
        let (conn, outcome) = open_and_sync(&settings_path, settings_tables).unwrap();
        assert_eq!(
            outcome,
            sebas_db::schema::SyncOutcome::Synced {
                added_columns: 0,
                renamed: 1,
                rebuilt: 0,
                dropped: 0,
            },
            "settings.db 的声明改名应原位执行"
        );
        let name: String = conn
            .query_row("SELECT name FROM providers WHERE id = 'p1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(name, "keep", "改名后 settings.db 存量行保值");
        assert_eq!(snapshot(&projects_path), projects_before, "settings.db 迁移不得触碰 projects.db");
        drop(conn);

        // ---- settings.db：类型变更（settings.value TEXT vs live INTEGER）----
        {
            let conn = sebas_db::conn::open(&settings_path).unwrap();
            conn.execute_batch(
                "DROP TABLE settings;
                 CREATE TABLE settings (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
                 INSERT INTO settings (key, value) VALUES ('k', 42);",
            )
            .unwrap();
        }
        let projects_before = snapshot(&projects_path);
        let (conn, outcome) = open_and_sync(&settings_path, SETTINGS_TABLES).unwrap();
        assert_eq!(
            outcome,
            sebas_db::schema::SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 1,
                dropped: 0,
            },
            "settings.db 的类型不符应事务内重建"
        );
        let value: String = conn
            .query_row("SELECT value FROM settings WHERE key = 'k'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "42", "重建按列名拷贝（INTEGER 42 经 TEXT 亲和转 '42'）");
        assert_eq!(snapshot(&projects_path), projects_before, "settings.db 迁移不得触碰 projects.db");
        drop(conn);

        // ---- settings.db：删列（providers.stale 无引用）----
        {
            let conn = sebas_db::conn::open(&settings_path).unwrap();
            conn.execute_batch("ALTER TABLE providers ADD COLUMN stale TEXT;")
                .unwrap();
        }
        let (conn, outcome) = open_and_sync(&settings_path, SETTINGS_TABLES).unwrap();
        assert_eq!(
            outcome,
            sebas_db::schema::SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 0,
                dropped: 1,
            },
            "settings.db 的多余列应原位 DROP"
        );
        assert!(!sqlite_columns(&conn, "providers").iter().any(|c| c == "stale"));
        drop(conn);

        // ---- projects.db：改名（projects.name ← title）----
        {
            let conn = sebas_db::conn::open(&projects_path).unwrap();
            conn.execute_batch("ALTER TABLE projects RENAME COLUMN name TO title;")
                .unwrap();
        }
        let settings_before = snapshot(&settings_path);
        let (conn, outcome) = open_and_sync(&projects_path, projects_tables).unwrap();
        assert_eq!(
            outcome,
            sebas_db::schema::SyncOutcome::Synced {
                added_columns: 0,
                renamed: 1,
                rebuilt: 0,
                dropped: 0,
            },
            "projects.db 的声明改名应原位执行"
        );
        let name: String = conn
            .query_row("SELECT name FROM projects WHERE id = 'id1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "proj", "改名后 projects.db 存量行保值");
        assert_eq!(snapshot(&settings_path), settings_before, "projects.db 迁移不得触碰 settings.db");
        drop(conn);

        // ---- projects.db：类型变更（projects.name TEXT vs live INTEGER）----
        {
            let conn = sebas_db::conn::open(&projects_path).unwrap();
            conn.execute_batch(
                "DROP TABLE projects;
                 CREATE TABLE projects (
                     id          TEXT,
                     path        TEXT PRIMARY KEY,
                     name        INTEGER NOT NULL,
                     branch_at   INTEGER NOT NULL DEFAULT 0,
                     added_at    INTEGER NOT NULL,
                     sort_order  INTEGER NOT NULL DEFAULT 0,
                     node_id     TEXT NOT NULL DEFAULT 'local',
                     default_agent TEXT,
                     branch      TEXT
                 );
                 CREATE UNIQUE INDEX idx_projects_id ON projects(id);
                 INSERT INTO projects (id, path, name, branch_at, added_at, sort_order, node_id)
                     VALUES ('id1', '/p', 42, 0, 1, 0, 'local');",
            )
            .unwrap();
        }
        let settings_before = snapshot(&settings_path);
        let (conn, outcome) = open_and_sync(&projects_path, PROJECTS_TABLES).unwrap();
        assert_eq!(
            outcome,
            sebas_db::schema::SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 1,
                dropped: 0,
            },
            "projects.db 的类型不符应事务内重建"
        );
        let name: String = conn
            .query_row("SELECT name FROM projects WHERE path = '/p'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(name, "42", "重建按列名拷贝并保留唯一索引");
        assert_eq!(snapshot(&settings_path), settings_before, "projects.db 迁移不得触碰 settings.db");
        // 再开一次必须 UpToDate：重建后的实表布局与注册 DDL 逐列自洽
        // （`name` 已是 TEXT）——比读 pragma 元数据更强，且不引入表结构
        // diff 原语（机械门禁只允许 sebas-db 一份）。
        let (conn, outcome) = open_and_sync(&projects_path, PROJECTS_TABLES).unwrap();
        assert_eq!(
            outcome,
            sebas_db::schema::SyncOutcome::UpToDate,
            "重建后的 projects 表与注册 DDL 自洽（name 已回到 TEXT）"
        );
        drop(conn);

        // ---- projects.db：删列（session_map.stale 无引用）----
        {
            let conn = sebas_db::conn::open(&projects_path).unwrap();
            conn.execute_batch("ALTER TABLE session_map ADD COLUMN stale TEXT;")
                .unwrap();
        }
        let (conn, outcome) = open_and_sync(&projects_path, PROJECTS_TABLES).unwrap();
        assert_eq!(
            outcome,
            sebas_db::schema::SyncOutcome::Synced {
                added_columns: 0,
                renamed: 0,
                rebuilt: 0,
                dropped: 1,
            },
            "projects.db 的多余列应原位 DROP"
        );
        assert!(!sqlite_columns(&conn, "session_map").iter().any(|c| c == "stale"));
        let sid: String = conn
            .query_row(
                "SELECT session_id FROM session_map WHERE chat_id = 'web'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sid, "s1", "删列不牵连同表的其它行");

        // 两库各自留下一份破坏性迁移前的备份，互不覆盖（各自库旁一份）。
        assert_eq!(
            temp_file_count(dir.path(), &format!("settings.db{}", sebas_db::schema::BACKUP_SUFFIX)),
            1
        );
        assert_eq!(
            temp_file_count(dir.path(), &format!("projects.db{}", sebas_db::schema::BACKUP_SUFFIX)),
            1
        );
    }
}

// ---- extract-sebas-db 3.3/3.4：ActiveRecord 黄金样本 + 逐表往返 ----

#[cfg(test)]
mod active_record_tests {
    use super::{PROJECTS_TABLES, SETTINGS_TABLES};
    use rusqlite::Connection;
    use sebas_db::record::{delete_sql, select_sql, upsert_sql};
    use sebas_db::schema::open_and_sync;
    use sebas_models::provider::{ModelAliasRow, ProviderRow};
    use sebas_models::project::ProjectRow;
    use sebas_models::session_map::{self, SessionMapRow};
    use sebas_models::setting::SettingRow;
    use tempfile::tempdir;

    fn settings_db() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let conn = open_and_sync(&dir.path().join("ar-settings.db"), SETTINGS_TABLES)
            .unwrap()
            .0;
        (dir, conn)
    }

    fn projects_db() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let conn = open_and_sync(&dir.path().join("ar-projects.db"), PROJECTS_TABLES)
            .unwrap()
            .0;
        (dir, conn)
    }

    /// 黄金样本（3.3）：生成的 upsert SQL 与既有仓储语义的规范化形态逐字
    /// 一致——`INSERT INTO t (全列) VALUES (?1..?n) ON CONFLICT(主键)
    /// DO UPDATE SET 非键列 = excluded.非键列`。逐表钉死，SQL 形状漂移即红。
    /// providers 的列清单是 single-state-dir 3.2 的扁平化形状。
    #[test]
    fn generated_upsert_sql_matches_golden_samples() {
        assert_eq!(
            upsert_sql::<ProviderRow>(),
            "INSERT INTO providers (id, name, preset, base_url_anthropic, \
             base_url_openai_chat, base_url_openai_responses, api_key, api_key_env, \
             default_model, protocol, models, model_map, deleted, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, preset = excluded.preset, \
             base_url_anthropic = excluded.base_url_anthropic, \
             base_url_openai_chat = excluded.base_url_openai_chat, \
             base_url_openai_responses = excluded.base_url_openai_responses, \
             api_key = excluded.api_key, api_key_env = excluded.api_key_env, \
             default_model = excluded.default_model, protocol = excluded.protocol, \
             models = excluded.models, model_map = excluded.model_map, \
             deleted = excluded.deleted, created_at = excluded.created_at, \
             updated_at = excluded.updated_at"
        );
        assert_eq!(
            upsert_sql::<ModelAliasRow>(),
            "INSERT INTO model_aliases (alias, provider, upstream_model, created_at) \
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT(alias) DO UPDATE SET \
             provider = excluded.provider, upstream_model = excluded.upstream_model, \
             created_at = excluded.created_at"
        );
        assert_eq!(
            upsert_sql::<SettingRow>(),
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) \
             DO UPDATE SET value = excluded.value"
        );
        assert_eq!(
            upsert_sql::<ProjectRow>(),
            "INSERT INTO projects (id, path, name, branch_at, added_at, sort_order, \
             node_id, default_agent, branch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(path) DO UPDATE SET id = excluded.id, name = excluded.name, \
             branch_at = excluded.branch_at, added_at = excluded.added_at, \
             sort_order = excluded.sort_order, node_id = excluded.node_id, \
             default_agent = excluded.default_agent, branch = excluded.branch"
        );
        assert_eq!(
            upsert_sql::<SessionMapRow>(),
            "INSERT INTO session_map (chat_id, thread_id, session_id, last_active_unix, \
             project_dir, acp_session_id, current_model, pending_kind, pending_model, \
             pending_mode, desired_mode, label, prompt_preview, awaiting_first_prompt) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
             ON CONFLICT(chat_id, thread_id) \
             DO UPDATE SET session_id = excluded.session_id, \
             last_active_unix = excluded.last_active_unix, project_dir = excluded.project_dir, \
             acp_session_id = excluded.acp_session_id, current_model = excluded.current_model, \
             pending_kind = excluded.pending_kind, pending_model = excluded.pending_model, \
             pending_mode = excluded.pending_mode, desired_mode = excluded.desired_mode, \
             label = excluded.label, prompt_preview = excluded.prompt_preview, \
             awaiting_first_prompt = excluded.awaiting_first_prompt"
        );

        // find / delete 的 SQL 形状同样来自生成器（单列主键形态）。
        assert_eq!(
            select_sql::<SettingRow>(),
            "SELECT key, value FROM settings"
        );
        assert_eq!(
            delete_sql::<SettingRow>(),
            "DELETE FROM settings WHERE key = ?1"
        );
    }

    /// 往返全等（3.3）：每张表一个 save → find/all → 字段全等 → 删除用例。
    #[test]
    fn provider_row_save_find_round_trip() {
        let (_dir, conn) = settings_db();
        let row = ProviderRow {
            id: "p1".into(),
            name: Some("p1".into()),
            preset: Some("deepseek".into()),
            base_url_anthropic: None,
            base_url_openai_chat: Some("https://x".into()),
            base_url_openai_responses: None,
            api_key: Some("sk-x".into()),
            api_key_env: None,
            default_model: None,
            protocol: None,
            models: Some(r#"[{"id":"m","tags":[]}]"#.into()),
            model_map: None,
            deleted: 0,
            created_at: 100,
            updated_at: 200,
        };
        row.save(&conn).unwrap();
        assert_eq!(ProviderRow::find(&conn, "p1").unwrap().unwrap(), row);
        // upsert 更新分支：同主键覆盖。
        let mut updated = row.clone();
        updated.updated_at = 300;
        updated.deleted = 1;
        updated.save(&conn).unwrap();
        assert_eq!(ProviderRow::find(&conn, "p1").unwrap().unwrap(), updated);
        assert!(ProviderRow::delete(&conn, "p1").unwrap());
        assert!(ProviderRow::find(&conn, "p1").unwrap().is_none());
    }

    #[test]
    fn model_alias_row_save_find_round_trip() {
        let (_dir, conn) = settings_db();
        // model_aliases.provider 有 REFERENCES providers(id) 外键：先落 provider。
        let provider = ProviderRow::from_item(
            "anthropic",
            &serde_json::from_value(serde_json::json!({"preset": "anthropic"})).unwrap(),
        );
        provider.save(&conn).unwrap();
        let row = ModelAliasRow {
            alias: "my-claude".into(),
            provider: "anthropic".into(),
            upstream_model: Some("claude-sonnet-4".into()),
            created_at: 42,
        };
        row.save(&conn).unwrap();
        assert_eq!(ModelAliasRow::find(&conn, "my-claude").unwrap().unwrap(), row);
        assert!(ModelAliasRow::delete(&conn, "my-claude").unwrap());
        assert!(ModelAliasRow::find(&conn, "my-claude").unwrap().is_none());
    }

    #[test]
    fn setting_row_save_find_round_trip() {
        let (_dir, conn) = settings_db();
        let row = SettingRow { key: "k".into(), value: "v".into() };
        row.save(&conn).unwrap();
        assert_eq!(SettingRow::find(&conn, "k").unwrap().unwrap(), row);
        assert_eq!(SettingRow::all(&conn).unwrap(), vec![row.clone()]);
        assert!(SettingRow::delete(&conn, "k").unwrap());
    }

    #[test]
    fn project_row_save_find_round_trip() {
        let (_dir, mut conn) = projects_db();
        let row = ProjectRow {
            id: Some("proj-1".into()),
            path: "/tmp/p".into(),
            name: "p".into(),
            node_id: sebas_models::project::LOCAL_NODE_ID.into(),
            default_agent: Some("claude".into()),
            branch: Some("main".into()),
            branch_at: 10,
            added_at: 7,
            sort_order: 3,
        };
        row.save(&conn).unwrap();
        assert_eq!(ProjectRow::find(&conn, "/tmp/p").unwrap().unwrap(), row);
        // 域查询（非标准排序）读回同实例。
        let listed = sebas_models::project::load_projects(&mut conn).unwrap();
        assert_eq!(listed, vec![row.clone()]);
        assert!(ProjectRow::delete(&conn, "/tmp/p").unwrap());
        assert!(ProjectRow::find(&conn, "/tmp/p").unwrap().is_none());
    }

    #[test]
    fn session_map_row_save_find_by_round_trip() {
        let (_dir, mut conn) = projects_db();
        let row = SessionMapRow {
            chat_id: "web".into(),
            thread_id: Some("web-1".into()),
            session_id: "s1".into(),
            last_active_unix: 99,
            project_dir: Some("/tmp/p".into()),
            acp_session_id: Some("acp-1".into()),
            current_model: Some("m1".into()),
            pending_kind: Some("claude".into()),
            pending_model: None,
            pending_mode: Some("edit".into()),
            desired_mode: "edit".into(),
            label: Some("重构计划".into()),
            prompt_preview: None,
            awaiting_first_prompt: false,
        };
        row.save(&conn).unwrap();
        // 复合主键：find_by / delete_by（按全部键列）。
        assert_eq!(
            SessionMapRow::find_by(&conn, "web", Some("web-1")).unwrap().unwrap(),
            row
        );
        assert_eq!(
            session_map::load_session_map(&mut conn).unwrap(),
            vec![row.clone()]
        );
        // upsert 更新分支：同键覆盖（按变更落盘就是同一键的反复 upsert）。
        let mut updated = row.clone();
        updated.session_id = "s2".into();
        updated.desired_mode = "auto".into();
        updated.awaiting_first_prompt = true;
        updated.save(&conn).unwrap();
        assert_eq!(
            session_map::load_session_map(&mut conn).unwrap(),
            vec![updated]
        );
        assert!(SessionMapRow::delete_by(&conn, "web", Some("web-1")).unwrap());
        assert!(SessionMapRow::find_by(&conn, "web", Some("web-1")).unwrap().is_none());
        assert!(session_map::load_session_map(&mut conn).unwrap().is_empty());
    }

    /// retire-schema-reset：改造前的 5 列 session_map 库在新注册表下打开——
    /// `desired_mode` / `awaiting_first_prompt` 是「非空且无常量默认」的缺列，
    /// 无法原地补列、也无法重建回填 → fail-closed 拒启动；库原样不动，
    /// 既不删文件也不迁移（取代旧的「隔离重置」语义）。
    #[test]
    fn old_shape_session_map_db_refuses_startup_and_keeps_file_untouched() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("projects.db");
        {
            // 手工搭出「改造前的库」：旧 5 列 session_map + 现行 projects 表
            // + 规范的 schema_meta 版本键（与旧代码打点一致）。
            let conn = sebas_db::conn::open(&db_path).unwrap();
            conn.execute_batch(&format!(
                "CREATE TABLE projects (
                    path        TEXT PRIMARY KEY,
                    name        TEXT NOT NULL,
                    branch      TEXT,
                    branch_at   INTEGER NOT NULL DEFAULT 0,
                    added_at    INTEGER NOT NULL,
                    sort_order  INTEGER NOT NULL DEFAULT 0,
                    id          TEXT,
                    default_agent TEXT
                );
                CREATE UNIQUE INDEX idx_projects_id ON projects(id);
                CREATE TABLE session_map (
                    chat_id          TEXT NOT NULL,
                    thread_id        TEXT,
                    session_id       TEXT NOT NULL,
                    last_active_unix INTEGER NOT NULL,
                    project_dir      TEXT,
                    PRIMARY KEY (chat_id, thread_id)
                );
                CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO schema_meta (key, value) VALUES
                    ('version_format', 'date'),
                    ('version', '{}');
                INSERT INTO session_map (chat_id, thread_id, session_id, last_active_unix, project_dir)
                    VALUES ('oc_old', NULL, 's-old', 100, '/tmp/p');",
                sebas_db::schema::SCHEMA_VERSION
            ))
            .unwrap();
        }
        let before_bytes = std::fs::read(&db_path).unwrap();
        let before_mtime = std::fs::metadata(&db_path).unwrap().modified().unwrap();

        let err = open_and_sync(&db_path, PROJECTS_TABLES)
            .err()
            .expect("非空无默认的缺列必须拒启动");
        assert!(
            err.contains("desired_mode") && err.contains("非空"),
            "诊断要点名不可迁移的缺列: {err}"
        );
        assert_eq!(
            std::fs::read(&db_path).unwrap(),
            before_bytes,
            "拒启动时库字节一个都不能变"
        );
        assert_eq!(
            std::fs::metadata(&db_path).unwrap().modified().unwrap(),
            before_mtime,
            "拒启动时库 mtime 也不变"
        );

        // 旧行仍在旧结构里可读；库旁没有任何重置/备份产物。
        let old_conn = sebas_db::conn::open_readonly(&db_path).unwrap();
        let old_sid: String = old_conn
            .query_row(
                "SELECT session_id FROM session_map WHERE chat_id = 'oc_old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_sid, "s-old", "旧数据原样保留（未被删库清空）");
        drop(old_conn);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .map(|n| {
                        let n = n.to_string_lossy();
                        n.contains(".reset-") || n.ends_with(sebas_db::schema::BACKUP_SUFFIX)
                    })
                    .unwrap_or(false)
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "拒启动路径不得留下重置或备份产物: {leftovers:?}"
        );
    }

}

// 下沉说明：StateWriter 的启动接入点在 src/run.rs（经本模块的域接线包装）；
// 以下断言钉住「writer 走根注册表 + sebas-db actor」的组合形态，以及
// single-state-dir 3.1 的两库落点（映射表派生路径 + 各自注册表）。
#[cfg(test)]
mod writer_wiring_tests {
    use crate::sebas_state::writer::StateWriter;
    use sebas_dispatch::state_store::StateStoreEngine;
    use tempfile::tempdir;

    /// 两库各自 open：settings.db 拿到三张表 + schema_meta，projects.db
    /// 拿到两张表 + schema_meta；文件名按映射表落点。
    #[tokio::test]
    async fn domain_writers_boot_two_databases_with_their_own_registries() {
        let dir = tempdir().unwrap();
        let settings_path = dir.path().join("settings.db");
        let projects_path = dir.path().join("projects.db");

        let settings =
            StateWriter::start_settings(settings_path.clone()).expect("settings writer");
        let projects =
            StateWriter::start_projects(projects_path.clone()).expect("projects writer");

        let settings_tables: Vec<String> = settings
            .handle()
            .exec(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))
                    .map_err(|e| e.to_string())?;
                rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
            })
            .await
            .unwrap();
        for expected in ["providers", "model_aliases", "settings", "schema_meta"] {
            assert!(
                settings_tables.iter().any(|t| t == expected),
                "settings.db 缺表 {expected}: {settings_tables:?}"
            );
        }
        assert!(
            !settings_tables.iter().any(|t| t == "projects"),
            "settings.db 不得再装 projects 表"
        );

        let projects_tables: Vec<String> = projects
            .handle()
            .exec(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))
                    .map_err(|e| e.to_string())?;
                rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
            })
            .await
            .unwrap();
        for expected in ["projects", "session_map", "schema_meta"] {
            assert!(
                projects_tables.iter().any(|t| t == expected),
                "projects.db 缺表 {expected}: {projects_tables:?}"
            );
        }
        assert!(
            !projects_tables.iter().any(|t| t == "providers"),
            "projects.db 不得装 settings 域表"
        );

        // 引擎接两库：项目域经 projects.db、设置域经 settings.db。
        let engine = crate::sebas_state::engine::DbStateEngine::with_projects(
            settings.handle().clone(),
            projects.handle().clone(),
        );
        engine
            .add_project(sebas_models::project::LOCAL_NODE_ID, "/tmp/two-db", "two-db", 1)
            .await
            .unwrap();
        engine
            .save_settings(serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(engine.load_projects().await.unwrap().len(), 1);
        assert!(engine.load_settings().await.unwrap().is_some());
    }

    /// 两库的版本戳各自独立（3.3）：schema_meta 各自打各自的键，任一库
    /// 迁移只重写自己的 meta。
    #[tokio::test]
    async fn each_database_stamps_its_own_version_keys() {
        let dir = tempdir().unwrap();
        let settings = StateWriter::start_settings(dir.path().join("settings.db")).unwrap();
        let projects = StateWriter::start_projects(dir.path().join("projects.db")).unwrap();
        for (handle, name) in [
            (settings.handle(), "settings.db"),
            (projects.handle(), "projects.db"),
        ] {
            let (format, version): (String, String) = handle
                .exec(|conn| {
                    let format: String = conn
                        .query_row(
                            "SELECT value FROM schema_meta WHERE key='version_format'",
                            [],
                            |r| r.get(0),
                        )
                        .map_err(|e| e.to_string())?;
                    let version: String = conn
                        .query_row(
                            "SELECT value FROM schema_meta WHERE key='version'",
                            [],
                            |r| r.get(0),
                        )
                        .map_err(|e| e.to_string())?;
                    Ok((format, version))
                })
                .await
                .unwrap();
            assert_eq!(format, "date", "{name} 的 version_format");
            assert_eq!(version, sebas_db::schema::SCHEMA_VERSION, "{name} 的版本");
        }
    }
}
