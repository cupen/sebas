//! extract-sebas-db 集成层补测（review 补齐：单元测试之外的跨 crate 行为）。
//!
//! 对照 `openspec/changes/extract-sebas-db/specs/persistence-runtime/spec.md`
//! 的三个此前只有单元级或纯机械断言、缺少跨 crate 行为级覆盖的场景：
//!
//! - R1「A second database does not re-implement the recipe」的行为面：第二
//!   个库经共享配方打开后，`foreign_keys=ON` 真实生效（不只是 pragma 读数）；
//! - R4「Adding a column updates the mapping, not call sites」：存量库缺列
//!   原地补齐后，**生成的 CRUD**（`find`）把旧行读回、新字段取默认值；
//! - R4「Non-standard queries still return instances」：`aliases_for_provider`
//!   （按非键列条件的域查询）返回表 struct 实例而非散值（迁移前该函数零覆盖）。
//!
//! 域 DDL 事实（哪张表、什么约束）按 design D2 留在域侧——本文件作为集成
//! 测试夹具内联与根 crate 注册表同形的 DDL，仅用于驱动共享层的同步与 CRUD。

use sebas_db::record::Record;
use sebas_db::schema::{SyncOutcome, TableSchema};
use sebas_models::provider::{aliases_for_provider, ModelAliasRow, ProviderRow};
use sebas_models::project::ProjectRow;
use tempfile::tempdir;

// ---- 夹具：与根 crate 注册表同形的域 DDL（约束只在 DDL，列来自 struct）----

static FIXTURE_TABLES: &[TableSchema] = &[
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
            models      TEXT,
            model_map   TEXT,
            deleted     INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL
        );",
        index_ddls: &[],
        columns: ProviderRow::schema_columns(),
    },
    TableSchema {
        name: "model_aliases",
        create_table_ddl: "CREATE TABLE model_aliases (
            alias           TEXT PRIMARY KEY,
            provider        TEXT NOT NULL REFERENCES providers(id),
            upstream_model  TEXT,
            created_at      INTEGER NOT NULL
        );",
        index_ddls: &[],
        columns: ModelAliasRow::schema_columns(),
    },
];

/// 缺列场景专用：model 已是八列、DDL 夹具仍是迁移前五列——差集就是要被
/// 启动同步原地补上的列。
static LEGACY_PROJECTS_TABLE: &[TableSchema] = &[TableSchema {
    name: "projects",
    create_table_ddl: "CREATE TABLE projects (
        path        TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        branch      TEXT,
        branch_at   INTEGER NOT NULL DEFAULT 0,
        added_at    INTEGER NOT NULL
    );",
    index_ddls: &[],
    columns: ProjectRow::schema_columns(),
}];

fn synced_db(name: &str) -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempdir().unwrap();
    let (conn, _) =
        sebas_db::schema::open_and_sync(&dir.path().join(name), FIXTURE_TABLES).unwrap();
    (dir, conn)
}


/// 类型化 providers 行夹具（single-state-dir 3.2 扁平化列形态）。
fn typed_provider_row(id: &str) -> ProviderRow {
    let mut row = ProviderRow::from_item(
        id,
        &serde_json::from_value(serde_json::json!({"preset": "anthropic"})).unwrap(),
    );
    row.created_at = 1;
    row.updated_at = 1;
    row
}

// ---- R1：第二库经共享配方拿到 foreign_keys=ON（行为级，不只 pragma 读数）----

/// 共享配方打开的第二库，外键约束真实生效：引用不存在的 provider 的别名
/// 插入必须被拒（`foreign_keys=ON` 是配方三件套之一，R1 场景的行为面）。
#[test]
fn shared_recipe_enforces_foreign_keys_on_a_second_database() {
    let (_dir, mut conn) = synced_db("fk.db");

    let err = ModelAliasRow {
        alias: "orphan".into(),
        provider: "no-such-provider".into(),
        upstream_model: None,
        created_at: 1,
    }
    .save(&mut conn);

    let err = err.expect_err("FK ON 时引用缺失 provider 的插入必须失败");
    assert!(
        matches!(err, rusqlite::Error::SqliteFailure(..)),
        "应为外键约束错误，实际 {err:?}"
    );

    // 对照组：provider 在场时同一形状插入成功。
    typed_provider_row("anthropic").save(&conn)
    .unwrap();
    ModelAliasRow {
        alias: "my-claude".into(),
        provider: "anthropic".into(),
        upstream_model: Some("claude-sonnet-4".into()),
        created_at: 2,
    }
    .save(&conn)
    .unwrap();
    assert_eq!(ModelAliasRow::find(&conn, "my-claude").unwrap().unwrap().provider, "anthropic");
}

// ---- R4：缺列补齐后，旧行经生成的 CRUD 读回、新字段取默认值 ----

/// 场景「Adding a column updates the mapping, not call sites」：存量库缺
/// `id` / `default_agent` / `sort_order` / `node_id` 四列（可空 / 可空 /
/// 带默认 / 带默认 `'local'`），启动同步原地补列后，`ProjectRow::find` 读回
/// 旧行——新字段取默认值（`node_id` 回填 `local` 即项目身份的迁移），call
/// site（find 签名与调用形状）零变化。
#[test]
fn added_column_reads_back_at_default_through_generated_crud() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("legacy-projects.db");

    // 旧形状库：projects 只有迁移前五列，含一行业务数据；版本键已按自描述
    // 格式落好（缺列走「原地补齐」，不走重置）。
    {
        let mut conn = sebas_db::conn::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE projects (
                path        TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                branch      TEXT,
                branch_at   INTEGER NOT NULL DEFAULT 0,
                added_at    INTEGER NOT NULL
            );
            CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO schema_meta (key, value) VALUES
                ('version_format', 'date'),
                ('version', '19990101');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO projects (path, name, branch, branch_at, added_at)
             VALUES ('/legacy/p', 'legacy', 'main', 42, 7)",
            [],
        )
        .unwrap();
    }

    let (mut conn, outcome) =
        sebas_db::schema::open_and_sync(&path, LEGACY_PROJECTS_TABLE).unwrap();
    assert_eq!(
        outcome,
        SyncOutcome::Synced {
            added_columns: 4,
            renamed: 0,
            rebuilt: 0,
            dropped: 0,
        },
        "四个缺列都应原地补齐"
    );

    // 生成的 CRUD 读回旧行：既有字段保值，新字段取默认（id/default_agent
    // 可空 → None；sort_order 带 DEFAULT 0 → 0；node_id 带 DEFAULT 'local'
    // → `local`，即旧行自动归到本机节点，不需要迁移脚本）。find 的调用形状
    // 与列扩张前完全一致——call site 未动。
    let row = ProjectRow::find(&conn, "/legacy/p").unwrap().expect("旧行应可读");
    assert_eq!(row.path, "/legacy/p");
    assert_eq!(row.name, "legacy");
    assert_eq!(row.branch.as_deref(), Some("main"));
    assert_eq!(row.branch_at, 42);
    assert_eq!(row.added_at, 7);
    assert_eq!(row.id, None, "补出的可空列读回 None");
    assert_eq!(row.default_agent, None, "补出的可空列读回 None");
    assert_eq!(row.sort_order, 0, "带默认的补列读回 DEFAULT 0");
    assert_eq!(
        row.node_id,
        sebas_models::project::LOCAL_NODE_ID,
        "补出的 node_id 回填本机标识（旧行归本机即迁移）"
    );

    // 补列后的行可整体 save（upsert 全列写）→ find 全等。
    let updated = ProjectRow {
        id: Some("proj-legacy".into()),
        path: "/legacy/p".into(),
        name: "legacy".into(),
        node_id: "local".into(),
        default_agent: Some("claude".into()),
        branch: Some("main".into()),
        branch_at: 42,
        added_at: 7,
        sort_order: 5,
    };
    updated.save(&conn).unwrap();
    assert_eq!(ProjectRow::find(&conn, "/legacy/p").unwrap().unwrap(), updated);
}

// ---- R4：非标准查询返回 struct 实例（aliases_for_provider 原为零覆盖）----

/// 场景「Non-standard queries still return instances」：按 provider（非键列）
/// 查别名的域查询返回 `Vec<ModelAliasRow>`——过滤正确、按 alias 排序、字段
/// 全等；不返回散值或无类型 map。
#[test]
fn alias_domain_query_returns_struct_instances_filtered_and_ordered() {
    let (_dir, mut conn) = synced_db("aliases.db");

    typed_provider_row("p1").save(&conn).unwrap();
    typed_provider_row("p2").save(&conn).unwrap();

    // 故意乱序插入（c, a, b）+ 另一 provider 的干扰行。
    for (alias, provider, upstream, created) in [
        ("zeta", "p1", Some("m-z"), 3),
        ("alpha", "p1", None, 1),
        ("mid", "p1", Some("m-m"), 2),
        ("other", "p2", Some("m-o"), 9),
    ] {
        ModelAliasRow {
            alias: alias.into(),
            provider: provider.into(),
            upstream_model: upstream.map(|s| s.to_string()),
            created_at: created,
        }
        .save(&conn)
        .unwrap();
    }

    let rows = aliases_for_provider(&mut conn, "p1").unwrap();
    assert_eq!(
        rows.iter().map(|r| r.alias.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "mid", "zeta"],
        "只含该 provider 的行且按 alias 排序"
    );
    assert_eq!(rows[0].upstream_model, None);
    assert_eq!(rows[0].created_at, 1);
    assert_eq!(rows[1].upstream_model.as_deref(), Some("m-m"));
    assert_eq!(rows[2].upstream_model.as_deref(), Some("m-z"));

    // 无命中 → 空表（仍是实例序列，不是错误）。
    assert!(aliases_for_provider(&mut conn, "no-such").unwrap().is_empty());

    // 行即实例：经 <ModelAliasRow as Record> 读回全等（同一行的两条读路径）。
    let direct = ModelAliasRow::find(&conn, "mid").unwrap().unwrap();
    assert!(rows.contains(&direct));
}
