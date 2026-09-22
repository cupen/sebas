//! 状态库域注册表 + PersistedState / settings 域的存储侧胶水。
//!
//! extract-sebas-db D2：域 schema 事实（哪张表、什么约束的手写 DDL）留在
//! 根 crate；行 struct 与标准 CRUD 已迁 [`sebas_models`]（ActiveRecord，
//! 一表一 struct）；连接/同步/单写 actor 在 [`sebas_db`]。本文件剩下的
//! 自由函数只覆盖**引用角色类型的胶水**——`PersistedState`（sebas-dispatch）
//! 与 `CardConfig`（sebas-feishu）进不了 sebas-models 的依赖清单。

use rusqlite::Connection;
use sebas_db::schema::TableSchema;
use sebas_models::provider::{ModelAliasRow, ProviderRow};
use sebas_models::setting::SettingRow;
use sebas_models::runtime_state::{self, RuntimeStateRow};

// ---- Table schemas (sqlite-auto-schema-sync) ----

/// 五表注册清单: (表名, 首建/重建 DDL, 派生列) (sqlite-auto-schema-sync D2)。
///
/// - DDL 只在"建新库/重置"时执行, 与 v2 基线逐字对齐 (列名/类型/默认值/
///   约束/索引); 日常同步只对比派生列 vs `PRAGMA table_info`。
/// - 结构性约束 (PRIMARY KEY / UNIQUE / REFERENCES) 与索引只能表达在 DDL:
///   这类列在 struct 里没有非空默认, 缺列场景由 sync 判为不可原地补列 → 重置。
/// - 列清单来自 `sebas-models` 的行 struct（extract-sebas-db 4.1——derive
///   首次在根 crate 之外工作）。
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
        columns: sebas_models::provider::ProviderRow::schema_columns(),
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
        columns: sebas_models::provider::ModelAliasRow::schema_columns(),
    },
    TableSchema {
        name: "settings",
        create_ddl: "CREATE TABLE settings (
            key     TEXT PRIMARY KEY,
            value   TEXT NOT NULL    -- JSON blob
        );",
        columns: sebas_models::setting::SettingRow::schema_columns(),
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
        columns: sebas_models::project::ProjectRow::schema_columns(),
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
        columns: sebas_models::session_map::SessionMapRow::schema_columns(),
    },
];

// ---- PersistedState 域的存储侧胶水 ----

/// 从 DB 加载 provider 数据, 构造 `sebas_dispatch::state_store::PersistedState`。
///
/// 读取 providers 表(含软删) + model_aliases 表 + settings 的 runtime 状态,
/// 经 `sebas_models` 的行 struct 出入（extract-sebas-db D3b：无类型载体出局）。
pub fn load_persisted_state(
    conn: &mut Connection,
) -> Result<sebas_dispatch::state_store::PersistedState, String> {
    use sebas_dispatch::state_store::PersistedState;
    use std::collections::BTreeMap;

    // 读 providers 表 (含软删)——行经 ProviderRow。
    let mut providers: BTreeMap<String, sebas_dispatch::crud::Item> = BTreeMap::new();
    let mut deleted: Vec<String> = Vec::new();
    for row in ProviderRow::all(conn).map_err(|e| format!("查询 providers 失败: {e}"))? {
        if row.deleted != 0 {
            deleted.push(row.id);
        } else {
            match serde_json::from_str::<sebas_dispatch::crud::Item>(&row.config) {
                Ok(item) => {
                    providers.insert(row.id, item);
                }
                Err(e) => {
                    tracing::warn!(provider = %row.id, error = %e, "failed to parse provider config JSON, skipping");
                }
            }
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

/// 保存 PersistedState 到 DB。
///
/// 写入 providers 表 (upsert + 软删) + model_aliases + 运行时状态到 settings
/// 表，全部经 ActiveRecord 的 `save()`，单事务提交。
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

    // 写 providers (非软删)
    for (id, item) in &state.providers {
        let config =
            serde_json::to_string(item).map_err(|e| format!("序列化 provider {id} 失败: {e}"))?;
        ProviderRow {
            id: id.clone(),
            config,
            deleted: 0,
            created_at: now,
            updated_at: now,
        }
        .save(&tx)
        .map_err(|e| format!("写入 provider {id} 失败: {e}"))?;
    }

    // 写 deleted providers (软删)
    for id in &state.deleted {
        ProviderRow {
            id: id.clone(),
            config: "{}".to_string(),
            deleted: 1,
            created_at: now,
            updated_at: now,
        }
        .save(&tx)
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
    use sebas_models::project;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn setup_db() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open_and_sync(&path, REGISTERED_TABLES).unwrap().0;
        (dir, conn)
    }

    #[test]
    fn load_empty_db_returns_default_state() {
        let (_dir, mut conn) = setup_db();
        let state = load_persisted_state(&mut conn).unwrap();
        assert!(state.providers.is_empty());
        assert!(state.deleted.is_empty());
        assert_eq!(state.mode, ProviderMode::Off);
        assert_eq!(state.default_selection, None);
    }

    #[test]
    fn save_and_load_provider_state_round_trips() {
        let (_dir, mut conn) = setup_db();

        let mut item = serde_json::Map::new();
        item.insert("name".into(), serde_json::Value::String("deepseek".into()));
        item.insert(
            "preset".into(),
            serde_json::Value::String("deepseek".into()),
        );

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
        assert!(loaded.providers.contains_key("deepseek"));
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
        project::add_project(&mut conn, "/tmp/p1", "p1", now).unwrap();
        project::add_project(&mut conn, "/tmp/p2", "p2", now + 1).unwrap();

        let projects = project::load_projects(&mut conn).unwrap();
        assert_eq!(projects.len(), 2);

        // 删除
        assert!(project::remove_project(&mut conn, "/tmp/p1").unwrap());
        let projects = project::load_projects(&mut conn).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "p2");

        // 更新分支
        project::update_project_branch(&mut conn, "/tmp/p2", Some("main"), now + 10).unwrap();
        let projects = project::load_projects(&mut conn).unwrap();
        assert_eq!(projects[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn save_projects_replaces_all() {
        let (_dir, mut conn) = setup_db();
        let now = 1000;

        project::add_project(&mut conn, "/tmp/p1", "p1", now).unwrap();
        project::add_project(&mut conn, "/tmp/p2", "p2", now + 1).unwrap();

        // 全量替换
        project::save_projects(&mut conn, &[]).unwrap();
        let projects = project::load_projects(&mut conn).unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn update_persisted_state_rmw() {
        let (_dir, mut conn) = setup_db();

        update_persisted_state(&mut conn, |s| {
            s.mode = ProviderMode::Router;
        })
        .unwrap();

        let state = load_persisted_state(&mut conn).unwrap();
        assert_eq!(state.mode, ProviderMode::Router);
    }

    // ---- registered_tables_match_v2_baseline_columns（4.5 基线，迁移前既有）----

    #[test]
    fn registered_tables_match_v2_baseline_columns() {
        // 派生列与 v2 基线对齐的一次性护栏: 列名 + 亲和逐一核对
        // （4.5：迁移前后基线一致——行 struct 迁 sebas-models 后列元数据不变）
        let want: &[(&str, &[(&str, &str)])] = &[
            (
                "providers",
                &[
                    ("id", "TEXT"),
                    ("config", "TEXT"),
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
            (
                "projects",
                &[
                    ("id", "TEXT"),
                    ("path", "TEXT"),
                    ("name", "TEXT"),
                    ("default_agent", "TEXT"),
                    ("branch", "TEXT"),
                    ("branch_at", "INTEGER"),
                    ("added_at", "INTEGER"),
                    ("sort_order", "INTEGER"),
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
                ],
            ),
        ];
        for (table, cols) in want {
            let reg = REGISTERED_TABLES
                .iter()
                .find(|t| t.name == *table)
                .unwrap();
            assert_eq!(reg.columns.len(), cols.len(), "表 {table} 列数不一致");
            for (col, (name, affinity)) in reg.columns.iter().zip(*cols) {
                assert_eq!(col.name, *name, "表 {table} 列名不一致");
                assert_eq!(col.affinity, *affinity, "表 {table} 列 {name} 亲和不一致");
            }
        }
    }
}

// ---- extract-sebas-db 3.3/3.4：ActiveRecord 黄金样本 + 逐表往返 ----

#[cfg(test)]
mod active_record_tests {
    use super::REGISTERED_TABLES;
    use rusqlite::Connection;
    use sebas_db::record::{delete_sql, select_sql, upsert_sql};
    use sebas_db::schema::open_and_sync;
    use sebas_models::provider::{ModelAliasRow, ProviderRow};
    use sebas_models::project::ProjectRow;
    use sebas_models::session_map::{self, SessionMapRow};
    use sebas_models::setting::SettingRow;
    use tempfile::tempdir;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempdir().unwrap();
        let conn = open_and_sync(&dir.path().join("ar.db"), REGISTERED_TABLES)
            .unwrap()
            .0;
        (dir, conn)
    }

    /// 黄金样本（3.3）：生成的 upsert SQL 与既有仓储语义的规范化形态逐字
    /// 一致——`INSERT INTO t (全列) VALUES (?1..?n) ON CONFLICT(主键)
    /// DO UPDATE SET 非键列 = excluded.非键列`。逐表钉死，SQL 形状漂移即红。
    #[test]
    fn generated_upsert_sql_matches_golden_samples() {
        assert_eq!(
            upsert_sql::<ProviderRow>(),
            "INSERT INTO providers (id, config, deleted, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(id) DO UPDATE SET \
             config = excluded.config, deleted = excluded.deleted, \
             created_at = excluded.created_at, updated_at = excluded.updated_at"
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
            "INSERT INTO projects (id, path, name, default_agent, branch, branch_at, \
             added_at, sort_order) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(path) DO UPDATE SET id = excluded.id, name = excluded.name, \
             default_agent = excluded.default_agent, branch = excluded.branch, \
             branch_at = excluded.branch_at, added_at = excluded.added_at, \
             sort_order = excluded.sort_order"
        );
        assert_eq!(
            upsert_sql::<SessionMapRow>(),
            "INSERT INTO session_map (chat_id, thread_id, session_id, last_active_unix, \
             project_dir) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(chat_id, thread_id) \
             DO UPDATE SET session_id = excluded.session_id, \
             last_active_unix = excluded.last_active_unix, project_dir = excluded.project_dir"
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
        let (_dir, conn) = db();
        let row = ProviderRow {
            id: "p1".into(),
            config: r#"{"preset":"deepseek"}"#.into(),
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
        let (_dir, conn) = db();
        // model_aliases.provider 有 REFERENCES providers(id) 外键：先落 provider。
        let provider = ProviderRow {
            id: "anthropic".into(),
            config: "{}".into(),
            deleted: 0,
            created_at: 1,
            updated_at: 1,
        };
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
        let (_dir, conn) = db();
        let row = SettingRow { key: "k".into(), value: "v".into() };
        row.save(&conn).unwrap();
        assert_eq!(SettingRow::find(&conn, "k").unwrap().unwrap(), row);
        assert_eq!(SettingRow::all(&conn).unwrap(), vec![row.clone()]);
        assert!(SettingRow::delete(&conn, "k").unwrap());
    }

    #[test]
    fn project_row_save_find_round_trip() {
        let (_dir, mut conn) = db();
        let row = ProjectRow {
            id: Some("proj-1".into()),
            path: "/tmp/p".into(),
            name: "p".into(),
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
        let (_dir, mut conn) = db();
        let row = SessionMapRow {
            chat_id: "ch1".into(),
            thread_id: Some("th1".into()),
            session_id: "s1".into(),
            last_active_unix: 99,
            project_dir: Some("/tmp/p".into()),
        };
        row.save(&conn).unwrap();
        // 复合主键：find_by / delete_by（按全部键列）。
        assert_eq!(
            SessionMapRow::find_by(&conn, "ch1", Some("th1")).unwrap().unwrap(),
            row
        );
        assert_eq!(
            session_map::load_session_map(&mut conn).unwrap(),
            vec![row.clone()]
        );
        assert!(SessionMapRow::delete_by(&conn, "ch1", Some("th1")).unwrap());
        assert!(SessionMapRow::find_by(&conn, "ch1", Some("th1")).unwrap().is_none());
    }
}

// 下沉说明：StateWriter 的启动接入点在 src/run.rs（经本模块的域接线包装）；
// 以下断言钉住「writer 走根注册表 + sebas-db actor」的组合形态。
#[cfg(test)]
mod writer_wiring_tests {
    use crate::sebas_state::writer::StateWriter;
    use tempfile::tempdir;

    #[tokio::test]
    async fn domain_writer_boots_actor_and_syncs_domain_schema() {
        let dir = tempdir().unwrap();
        let writer = StateWriter::start(dir.path().join("wiring.db")).unwrap();
        let count: i64 = writer
            .handle()
            .exec(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN \
                     ('providers','model_aliases','settings','projects','session_map','schema_meta')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())
            })
            .await
            .unwrap();
        assert_eq!(count, 6, "五张域表 + schema_meta 都应被同步创建");
    }
}
