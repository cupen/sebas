//! quarantine-database-reset 集成补测：单元测试（`sebas-db/src/schema.rs`）
//! 之外的两个行为层，对照
//! `openspec/changes/quarantine-database-reset/specs/state-store/spec.md`：
//!
//! 1. **写者 actor 层**：`sebas_db::writer::StateWriter::start` 是生产打开
//!    路径（专用线程里 open_and_sync）。损坏拒启在该层已有用例，但「结构
//!    不兼容 → 隔离后重建、写者照常就绪服务」此前只有单元级（直调
//!    open_and_sync）。场景「Incompatible structure resets the database」。
//! 2. **真域注册表层**：根 crate 域接线（`sebas_state::writer::StateWriter`
//!    + 五表 `REGISTERED_TABLES`）。单测用的是中性夹具表 `alpha`；这里证明
//!    未知 version_format 触发的重置隔离旧库、按**真实 DDL** 重建、真实
//!    ActiveRecord 行照常读写。场景「Unknown version format resets the
//!    database」+「A quarantined database is recoverable」。
//! 3. **隔离命名唯一性**（design Open Question：同一秒多次重置 → 追加
//!    pid/序号）：经公开 API 连续两次重置，先前的隔离产物不得被覆盖。
//!
//! 全部进程内 SQLite + 一次性 tempdir，无网络、无浏览器设施。

use sebas_db::record::Record;
use sebas_db::schema::{SyncOutcome, TableSchema};
use sebas_models::provider::ProviderRow;
use sebas::sebas_state::writer::StateWriter;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

// ---- 夹具：与根注册表 providers 同形的单表 DDL（列来自 struct）----

static PROVIDERS_TABLE: &[TableSchema] = &[TableSchema {
    name: "providers",
    create_ddl: "CREATE TABLE providers (
        id          TEXT PRIMARY KEY,
        config      TEXT NOT NULL,
        deleted     INTEGER NOT NULL DEFAULT 0,
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL
    );
    CREATE INDEX idx_providers_deleted ON providers(deleted);",
    columns: ProviderRow::schema_columns(),
}];

fn provider_row(id: &str) -> ProviderRow {
    ProviderRow {
        id: id.into(),
        config: "{}".into(),
        deleted: 0,
        created_at: 1,
        updated_at: 1,
    }
}

/// 目录下的隔离产物（`*.reset-*`），排序后返回。
fn quarantine_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".reset-")
        })
        .collect();
    files.sort();
    files
}

/// 造一个「结构不可调和」的旧库：providers 多一列 `stale`（model 之外 →
/// 多余列重置），版本键按自描述格式落好（保证触发的是**结构**重置而非未知
/// 格式），并写一行重置前数据。连接以块结束 drop = 关闭 = 提交落盘，
/// 最近一次提交必须在隔离文件中可见。
fn seed_db_with_stale_column(path: &Path, marker_id: &str) {
    let conn = sebas_db::conn::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE providers (
            id          TEXT PRIMARY KEY,
            config      TEXT NOT NULL,
            deleted     INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL,
            stale       TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        INSERT INTO schema_meta (key, value) VALUES
            ('version_format', 'date'),
            ('version', '19990101');",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO providers (id, config, deleted, created_at, updated_at, stale)
         VALUES (?1, '{}', 0, 1, 1, 'pre-reset')",
        rusqlite::params![marker_id],
    )
    .unwrap();
}

// ---- 1. 写者 actor 层：结构不兼容 → 隔离 + 重建 + 写者照常就绪 ----

/// 「Incompatible structure resets the database」×「A quarantined database is
/// recoverable」的 actor 层行为面：生产打开路径（写者线程里的 open_and_sync）
/// 遇多余列——`start` 返回即写者就绪（重置没有卡死启动）、旧库隔离未删除、
/// 隔离文件只读打开可读回重置前的行与结构、新库为空且照常服务 save/find。
#[tokio::test]
async fn writer_actor_resets_incompatible_db_leaving_recoverable_quarantine() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("actor-reset.db");
    seed_db_with_stale_column(&path, "p-old");

    // StateWriter::start 在同步完成前阻塞——能返回就证明「隔离+重建」在
    // 专用线程里走完且写者就绪。
    let writer = sebas_db::writer::StateWriter::start(path.clone(), PROVIDERS_TABLE).unwrap();
    let handle = writer.handle().clone();

    // 隔离产物恰好一份（干净关闭后无 sidecar），是可读的旧库
    let quarantined = quarantine_files(dir.path());
    assert_eq!(
        quarantined.len(),
        1,
        "旧库应被隔离而非删除: {quarantined:?}"
    );
    let qconn = sebas_db::conn::open_readonly(&quarantined[0]).unwrap();
    let (id, stale): (String, String) = qconn
        .query_row("SELECT id, stale FROM providers", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(id, "p-old", "重置前的行应存活在隔离文件中");
    assert_eq!(stale, "pre-reset", "隔离文件保留重置前的结构与数据");

    // 新库: 旧行不迁入（重置 ≠ 迁移，日志明说的语义在数据面成立）
    assert!(
        handle.find::<ProviderRow, _>("p-old").await.unwrap().is_none(),
        "重置后的新库不得包含旧行"
    );

    // 重置后写者照常服务: 真行 round-trip 落在重建后的 schema 上
    let fresh = provider_row("p-new");
    handle.save(&fresh).await.unwrap();
    assert_eq!(
        handle.find::<ProviderRow, _>("p-new").await.unwrap(),
        Some(fresh),
        "隔离重置后写者必须照常读写"
    );
}

// ---- 2. 真域注册表层：未知版本格式 → 隔离 + 真实 DDL 重建 ----

/// 「Unknown version format resets the database」的域层行为面：真实五表
/// 注册表（域接线 `StateWriter::start` → `REGISTERED_TABLES`）打开一个
/// `version_format` 被改成未知值的库——隔离而非删除、按真实 DDL 重建、
/// provider 行在重建后的 schema 上照常 save/find。
#[tokio::test]
async fn domain_writer_quarantines_unknown_version_format_and_serves_real_registry() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("domain-reset.db");

    // 第一段: 用真注册表建一次新库（fresh create）并写一行数据,
    // 再把它改造成「version_format 未知」的存量库。
    {
        let writer = StateWriter::start(path.clone()).unwrap();
        writer.handle().save(&provider_row("p-old")).await.unwrap();
    } // drop = 写者线程退出、连接关闭
    {
        let conn = sebas_db::conn::open(&path).unwrap();
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('version_format', 'semver')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .unwrap();
    }

    // 第二段: 再次启动写者——未知格式 → 隔离 + 按真 DDL 重建。
    let writer = StateWriter::start(path.clone()).unwrap();
    let handle = writer.handle().clone();

    let quarantined = quarantine_files(dir.path());
    assert_eq!(
        quarantined.len(),
        1,
        "未知格式也应隔离而非删除: {quarantined:?}"
    );
    let qconn = sebas_db::conn::open_readonly(&quarantined[0]).unwrap();
    let id: String = qconn
        .query_row("SELECT id FROM providers WHERE id = 'p-old'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id, "p-old", "隔离文件可读回重置前行（可手工恢复）");

    // 新库: 旧行不迁入; 真注册表 round-trip 照常。
    assert!(handle.find::<ProviderRow, _>("p-old").await.unwrap().is_none());
    let fresh = provider_row("p-new");
    handle.save(&fresh).await.unwrap();
    assert_eq!(
        handle.find::<ProviderRow, _>("p-new").await.unwrap(),
        Some(fresh),
        "按真实 DDL 重建后域行必须照常读写"
    );
}

// ---- 3. 隔离命名唯一性：连续两次重置互不覆盖 ----

/// design Open Question（同一秒内多次重置的命名唯一性，tasks 1.1「唯一性
/// 不足时追加序号或 pid」）：经公开 API 连续两次触发重置——第二次即便拿到
/// 与第一次相同的时间戳，也不得覆盖第一次的隔离产物。两份产物各自可读、
/// 各自保有自己的重置前行。（若两次重置跨秒，时间戳天然不同，断言同样成立。）
#[test]
fn consecutive_resets_keep_every_quarantine_copy_distinct_and_readable() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("collide.db");

    // 第一次重置: 结构不兼容（多余列 stale, marker r1）
    seed_db_with_stale_column(&path, "r1");
    let (conn, outcome) = sebas_db::schema::open_and_sync(&path, PROVIDERS_TABLE).unwrap();
    assert!(
        matches!(outcome, SyncOutcome::Reset { .. }),
        "前置: 多余列应触发重置, 实际 {outcome:?}"
    );
    drop(conn);

    // 第二次重置: 在重建后的新库上再造不兼容（再加一列 stale2, marker r2）
    {
        let conn = sebas_db::conn::open(&path).unwrap();
        conn.execute_batch("ALTER TABLE providers ADD COLUMN stale2 TEXT NOT NULL DEFAULT '';")
            .unwrap();
        conn.execute(
            "INSERT INTO providers (id, config, deleted, created_at, updated_at)
             VALUES ('r2', '{}', 0, 1, 1)",
            [],
        )
        .unwrap();
    }
    let (_conn, outcome) = sebas_db::schema::open_and_sync(&path, PROVIDERS_TABLE).unwrap();
    assert!(
        matches!(outcome, SyncOutcome::Reset { .. }),
        "前置: 第二次多余列也应触发重置, 实际 {outcome:?}"
    );

    let quarantined = quarantine_files(dir.path());
    assert_eq!(
        quarantined.len(),
        2,
        "两次重置各留一份隔离产物, 后一次不得覆盖前一次: {quarantined:?}"
    );

    // 两份都是可读旧库, 各自保有自己的 marker 行
    let mut seen = Vec::new();
    for q in &quarantined {
        let qconn = sebas_db::conn::open_readonly(q).unwrap();
        let id: String = qconn
            .query_row("SELECT id FROM providers", [], |r| r.get(0))
            .unwrap();
        seen.push(id);
    }
    seen.sort();
    assert_eq!(
        seen,
        vec!["r1".to_string(), "r2".to_string()],
        "两份隔离产物各保留各自的重置前行"
    );
}
