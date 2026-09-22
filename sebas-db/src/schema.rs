//! 启动 schema 同步：model struct 即事实源，缺列原地补，不兼容即重置
//! （sqlite-auto-schema-sync，extract-sebas-db 下沉）。
//!
//! # 同步流程
//!
//! 1. 先 [`conn::open`]。打不开 = 库损坏 → 走既有损坏
//!    拒启路径，**绝不删除文件**（重置只针对 schema 不兼容，与损坏严格分离）。
//! 2. 全新库 (无任何用户表) → 按注册 DDL 建 schema, 打版本键。
//! 3. 读 `schema_meta` 自描述版本键 (专用小表, 自建自管, 不参与 diff):
//!    `version_format` 缺失或未知 → 重置。旧迁移链产生的库只有
//!    `PRAGMA user_version`、无此键, 升级后首开即走这里 (预期行为)。
//! 4. 逐注册表 diff 派生列 vs `PRAGMA table_info` (按 SQLite 亲和类型归一
//!    比较, 避免 `VARCHAR(255)` vs `TEXT` 误报):
//!    - 缺列且能安全补 (可空, 或非空带常量默认) → `ALTER TABLE ADD COLUMN`;
//!    - 缺列但补不了 / 类型不符 / 多余列 / 整表缺失 → **重置**: 删 DB 文件
//!      (含 `-wal`/`-shm`) 按注册 DDL 重建空 schema, WARN 日志写明触发点。
//! 5. 全部通过 → 写 `version_format` + `version`。版本**值**不同不触发任何
//!    动作, 仅随写 meta 更新; 结构对比是唯一重置触发。
//!
//! 注册表（`&'static [TableSchema]`）由调用方传入——哪些表、什么约束是
//! **域 schema 事实**，留在域侧（根 crate 注册表）；本模块只知道"怎么比、
//! 怎么建、怎么重置"。

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use tracing::warn;

use crate::conn;

/// 当前 schema 版本 (sqlite-auto-schema-sync D4): 日期常量, 改 schema 时
/// bump, 不是 wall-clock。作用是诊断"这库是哪个 schema 日期的", 不作重置触发。
pub const SCHEMA_VERSION: &str = "20260913";

/// 版本键的格式标识。未知格式 → 重置 (为发布后的迁移机制预留格式位)。
pub const VERSION_FORMAT: &str = "date";

/// 同步层自建自管的版本元数据表 (键值对)。不参与 diff。
/// DDL 用小写：这是 runtime 自己的家务表、不是域 schema——域 DDL 的
/// 「只在根注册表」机械门禁（extract-sebas-db 4.6 的大写建表关键词检查）
/// 因此保持可查且干净。
const SCHEMA_META_DDL: &str =
    "create table if not exists schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)";

/// 单列元数据 (由 `#[derive(SchemaColumns)]` 生成, 挂在每个 `*Row` struct 上)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaColumn {
    pub name: &'static str,
    /// SQLite 亲和类型: TEXT / INTEGER / REAL / BLOB / NUMERIC。
    pub affinity: &'static str,
    /// 常量默认值 (逐字作为 SQL `DEFAULT` 表达式), 仅缺列补齐时使用。
    pub default: Option<&'static str>,
    pub not_null: bool,
}

/// 表注册三元组 (sqlite-auto-schema-sync D2): 表名 + 首建/重建 DDL + 派生列清单。
/// DDL 只在"建新库/重置"时执行; 日常同步只依赖 `columns` vs `PRAGMA table_info`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSchema {
    pub name: &'static str,
    pub create_ddl: &'static str,
    pub columns: &'static [SchemaColumn],
}

/// 同步结果 (观测/日志/测试用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// 全新库: 按注册 DDL 建 schema 并打版本键。
    FreshCreated,
    /// 结构一致, 无需变更 (版本值若有差异仅随写 meta)。
    UpToDate,
    /// 缺列已原地补齐。
    Synced { added_columns: usize },
    /// schema 不兼容, 已删库重建 (原因见 reason)。
    Reset { reason: String },
}

/// 同步内部错误: Incompatible 走重置, Fatal 拒启 (不动文件)。
pub enum SyncFail {
    Incompatible(String),
    Fatal(String),
}

/// 打开数据库并同步 schema。这是写者线程的启动入口:
/// `open` 失败按损坏拒启且不删除文件; 只有成功打开后跑同步, 才可能触发重置。
pub fn open_and_sync(
    db_path: &Path,
    tables: &'static [TableSchema],
) -> Result<(Connection, SyncOutcome), String> {
    let conn = conn::open(db_path).map_err(|e| {
        format!(
            "打开状态库失败，疑似损坏，拒绝启动且不自动删除文件: {} ({e})",
            db_path.display()
        )
    })?;

    match sync_conn(conn, tables) {
        Ok((conn, outcome)) => Ok((conn, outcome)),
        Err(SyncFail::Fatal(e)) => Err(e),
        Err(SyncFail::Incompatible(reason)) => {
            warn!(
                path = %db_path.display(),
                reason = %reason,
                "schema 不兼容: 删除状态库(含 -wal/-shm)并按当前 model 重建空 schema"
            );
            let conn = reset_and_rebuild(db_path, tables)?;
            Ok((conn, SyncOutcome::Reset { reason }))
        }
    }
}

/// 对已打开的连接执行同步。不兼容时先关闭连接再返回 (调用方删文件才安全)。
pub fn sync_conn(
    conn: Connection,
    tables: &'static [TableSchema],
) -> Result<(Connection, SyncOutcome), SyncFail> {
    // 全新库: 没有任何用户表 → 直接建 schema (不必删文件)。
    let user_tables = list_user_tables(&conn)
        .map_err(|e| SyncFail::Fatal(format!("读取 sqlite_master 失败: {e}")))?;
    if user_tables.is_empty() {
        let mut conn = conn;
        rebuild_schema(&mut conn, tables).map_err(SyncFail::Fatal)?;
        return Ok((conn, SyncOutcome::FreshCreated));
    }

    // 存量库: 先看自描述版本键。缺失/未知格式 → 重置 (覆盖旧迁移链库: 它们
    // 只有 user_version、无 schema_meta; 查询遇"无此表"同样返回 None)。
    let format: Option<String> = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version_format'",
            [],
            |row| row.get(0),
        )
        .ok();
    if format.as_deref() != Some(VERSION_FORMAT) {
        drop(conn);
        return Err(SyncFail::Incompatible(format!(
            "版本元数据缺失或未知格式 (version_format = {format:?}, 期望 {VERSION_FORMAT:?}; \
             旧迁移链产生的库没有这个键)"
        )));
    }

    // 逐注册表 diff。
    let mut alters: Vec<(&'static str, &'static SchemaColumn)> = Vec::new();
    for table in tables {
        let live = live_columns(&conn, table.name)
            .map_err(|e| SyncFail::Fatal(format!("读取 {} 表结构失败: {e}", table.name)))?;

        if live.is_empty() {
            drop(conn);
            return Err(SyncFail::Incompatible(format!("表 {} 缺失", table.name)));
        }

        for col in table.columns {
            match live.iter().find(|l| l.name == col.name) {
                None => alters.push((table.name, col)),
                Some(l) => {
                    let (live_affinity, want_affinity) =
                        (type_affinity(&l.decl), type_affinity(col.affinity));
                    if live_affinity != want_affinity {
                        drop(conn);
                        return Err(SyncFail::Incompatible(format!(
                            "表 {} 列 {} 类型不符: 库内 `{}` (亲和 {live_affinity}), \
                             model 期望亲和 {want_affinity}",
                            table.name, col.name, l.decl
                        )));
                    }
                }
            }
        }

        for l in &live {
            if !table.columns.iter().any(|c| c.name == l.name) {
                drop(conn);
                return Err(SyncFail::Incompatible(format!(
                    "表 {} 有 model 之外的多余列 `{}`",
                    table.name, l.name
                )));
            }
        }
    }

    if alters.is_empty() {
        // 结构一致: 版本值差异不触发任何动作, 仅随写 meta 更新。
        stamp_version(&conn).map_err(SyncFail::Fatal)?;
        return Ok((conn, SyncOutcome::UpToDate));
    }

    // 缺列原地补齐。补不了的 (非空且无常量默认——含 PRIMARY KEY/UNIQUE 成员列,
    // 这类列在 struct 里无非空默认) 不硬来, 走重置: 能加则加, 不能加则重置。
    for (table_name, col) in &alters {
        if col.not_null && col.default.is_none() {
            drop(conn);
            return Err(SyncFail::Incompatible(format!(
                "表 {table_name} 缺列 `{}`: 非空且无常量默认值, 无法 ALTER TABLE 原地补列",
                col.name
            )));
        }
    }

    let mut conn = conn;
    let tx = conn
        .transaction()
        .map_err(|e| SyncFail::Fatal(format!("补列事务开始失败: {e}")))?;
    for (table_name, col) in &alters {
        tx.execute_batch(&add_column_sql(table_name, col))
            .map_err(|e| SyncFail::Fatal(format!("补列 {}.{} 失败: {e}", table_name, col.name)))?;
    }
    stamp_version(&tx).map_err(SyncFail::Fatal)?;
    tx.commit()
        .map_err(|e| SyncFail::Fatal(format!("补列事务提交失败: {e}")))?;

    let added_columns = alters.len();
    Ok((conn, SyncOutcome::Synced { added_columns }))
}

/// 重置: 删 DB 文件 (含 `-wal`/`-shm`) 后按注册 DDL 重建空 schema。
/// 调用点 (open_and_sync) 已保证旧连接关闭; open 失败的损坏路径进不到这里。
pub fn reset_and_rebuild(
    db_path: &Path,
    tables: &'static [TableSchema],
) -> Result<Connection, String> {
    for suffix in ["", "-wal", "-shm"] {
        let file = db_sidecar_path(db_path, suffix);
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("重置状态库失败: 删除 {} 出错: {e}", file.display())),
        }
    }

    let mut conn = conn::open(db_path)
        .map_err(|e| format!("重置后重新打开状态库失败: {}: {e}", db_path.display()))?;
    rebuild_schema(&mut conn, tables)?;
    Ok(conn)
}

/// 按注册 DDL 重建全部表 + 索引, 建 schema_meta 并打版本键 (单事务)。
fn rebuild_schema(conn: &mut Connection, tables: &'static [TableSchema]) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("重建 schema 事务开始失败: {e}"))?;
    for table in tables {
        tx.execute_batch(table.create_ddl)
            .map_err(|e| format!("重建表 {} 失败: {e}", table.name))?;
    }
    ensure_schema_meta(&tx)?;
    stamp_version(&tx)?;
    tx.commit()
        .map_err(|e| format!("重建 schema 事务提交失败: {e}"))?;
    Ok(())
}

fn ensure_schema_meta(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(SCHEMA_META_DDL)
        .map_err(|e| format!("创建 schema_meta 失败: {e}"))
}

/// 写/更新版本键 (upsert)。结构一致时版本值差异仅经此更新, 不触发重置。
fn stamp_version(conn: &Connection) -> Result<(), String> {
    for (key, value) in [
        ("version_format", VERSION_FORMAT),
        ("version", SCHEMA_VERSION),
    ] {
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )
        .map_err(|e| format!("写版本键 {key} 失败: {e}"))?;
    }
    Ok(())
}

/// 拼缺列补齐语句: `ALTER TABLE t ADD COLUMN col AFFINITY [NOT NULL] [DEFAULT d]`。
/// 列名/默认值都来自编译期派生的常量, 非运行时输入。
pub fn add_column_sql(table: &str, col: &SchemaColumn) -> String {
    let mut sql = format!(
        "ALTER TABLE {table} ADD COLUMN {} {}",
        col.name, col.affinity
    );
    if col.not_null {
        sql.push_str(" NOT NULL");
    }
    if let Some(d) = col.default {
        sql.push_str(" DEFAULT ");
        sql.push_str(d);
    }
    sql
}

struct LiveColumn {
    name: String,
    /// 库内声明的列类型 (可能为空串, 如无类型列)。
    decl: String,
}

/// `PRAGMA table_info` 的 TVF 形式。表不存在时返回空 (调用方以空判定缺表)。
fn live_columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<LiveColumn>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT name, COALESCE(\"type\", '') FROM pragma_table_info('{table}')"
    ))?;
    let rows = stmt.query_map([], |row| {
        Ok(LiveColumn {
            name: row.get(0)?,
            decl: row.get(1)?,
        })
    })?;
    rows.collect()
}

/// SQLite 类型亲和规则 (<https://www.sqlite.org/datatype3.html> §3.1)。
/// 对比两侧都归一到亲和, `VARCHAR(255)` 与 `TEXT` 不误报。
pub fn type_affinity(decl: &str) -> &'static str {
    let d = decl.to_ascii_uppercase();
    if d.contains("INT") {
        "INTEGER"
    } else if d.contains("CHAR") || d.contains("CLOB") || d.contains("TEXT") {
        "TEXT"
    } else if d.contains("BLOB") || d.is_empty() {
        "BLOB"
    } else if d.contains("REAL") || d.contains("FLOA") || d.contains("DOUB") {
        "REAL"
    } else {
        "NUMERIC"
    }
}

fn list_user_tables(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect()
}

fn db_sidecar_path(db_path: &Path, suffix: &str) -> PathBuf {
    let mut s = db_path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::TEST_TABLES;
    use tempfile::tempdir;

    fn temp_db(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let path = dir.path().join(name);
        (dir, path)
    }

    fn meta_get(conn: &Connection, key: &str) -> Option<String> {
        conn.query_row(
            "SELECT value FROM schema_meta WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok()
    }

    /// 测试用: 直接写 schema_meta 键 (模拟旧值/坏格式)。
    fn meta_set(conn: &Connection, key: &str, value: &str) {
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            [key, value],
        )
        .unwrap();
    }

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect()
    }

    fn column_names(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect()
    }

    /// 手工建一张"旧结构" alpha (缺 score 列) —— 模拟"上一个 schema 日期的库"。
    fn seed_legacy_db_without_score(path: &Path) {
        let conn = conn::open(path).unwrap();
        conn.execute_batch(
            "create table alpha (
                id    TEXT PRIMARY KEY,
                name  TEXT NOT NULL,
                note  TEXT
            );",
        )
        .unwrap();
        conn.execute_batch(SCHEMA_META_DDL).unwrap();
        stamp_version(&conn).unwrap();
    }

    #[test]
    fn fresh_db_creates_schema_and_stamps_version_keys() {
        let (_dir, path) = temp_db("fresh.db");
        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();

        assert_eq!(outcome, SyncOutcome::FreshCreated);
        let tables = table_names(&conn);
        assert!(tables.iter().any(|t| t == "alpha"), "缺表 alpha: {tables:?}");
        assert!(tables.iter().any(|t| t == "schema_meta"));
        assert_eq!(meta_get(&conn, "version_format").as_deref(), Some("date"));
        assert_eq!(meta_get(&conn, "version").as_deref(), Some(SCHEMA_VERSION));
    }

    #[test]
    fn second_open_with_matching_structure_is_up_to_date() {
        let (_dir, path) = temp_db("uptodate.db");
        let (_conn, first) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(first, SyncOutcome::FreshCreated);
        let (_conn, second) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(second, SyncOutcome::UpToDate);
    }

    #[test]
    fn missing_column_is_added_in_place_and_old_rows_stay_readable() {
        let (_dir, path) = temp_db("addcol.db");
        seed_legacy_db_without_score(&path);

        // 旧行: 没有(score) 列的时代写入
        {
            let conn = conn::open(&path).unwrap();
            conn.execute(
                "INSERT INTO alpha (id, name, note) VALUES ('a1', 'old', NULL)",
                [],
            )
            .unwrap();
        }

        let (mut conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert!(
            matches!(outcome, SyncOutcome::Synced { added_columns: 1 }),
            "应原地补一列, 实际 {outcome:?}"
        );

        // 旧行可读: 新列取常量默认值
        let score: i64 = conn
            .query_row("SELECT score FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(score, 0, "旧行的新列应取 DEFAULT 0");
    }

    #[test]
    fn extra_column_resets_database() {
        let (_dir, path) = temp_db("extracol.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            conn.execute_batch("ALTER TABLE alpha ADD COLUMN stale TEXT;")
                .unwrap();
            // 放一行数据证明重置是"删库重建", 不是保留数据
            conn.execute(
                "INSERT INTO alpha (id, name) VALUES ('a1', 'keepme')",
                [],
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        let reason = match outcome {
            SyncOutcome::Reset { reason } => reason,
            other => panic!("多余列应触发重置, 实际 {other:?}"),
        };
        assert!(
            reason.contains("stale") && reason.contains("alpha"),
            "日志要点名触发点: {reason}"
        );

        assert!(!column_names(&conn, "alpha").iter().any(|c| c == "stale"));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM alpha", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "重置后是空 schema, marker 行不应幸存");
        assert_eq!(meta_get(&conn, "version").as_deref(), Some(SCHEMA_VERSION));
    }

    #[test]
    fn type_mismatch_resets_database() {
        let (_dir, path) = temp_db("typemismatch.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            conn.execute_batch(
                "DROP TABLE alpha;
                 create table alpha (
                    id    TEXT PRIMARY KEY,
                    name  INTEGER NOT NULL,
                    note  TEXT,
                    score INTEGER NOT NULL DEFAULT 0
                 );",
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        let reason = match outcome {
            SyncOutcome::Reset { reason } => reason,
            other => panic!("类型不符应触发重置, 实际 {other:?}"),
        };
        assert!(reason.contains("name"), "类型不符要点名列: {reason}");

        // 按注册 DDL 重建: name 回到 TEXT 亲和
        let decl: String = conn
            .query_row(
                "SELECT \"type\" FROM pragma_table_info('alpha') WHERE name = 'name'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(type_affinity(&decl), "TEXT");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM alpha", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn missing_table_resets_database() {
        let (_dir, path) = temp_db("missingtable.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            conn.execute_batch("DROP TABLE alpha;").unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        let reason = match outcome {
            SyncOutcome::Reset { reason } => reason,
            other => panic!("缺表应触发重置, 实际 {other:?}"),
        };
        assert!(reason.contains("alpha"), "缺表要点名表: {reason}");
        assert!(table_names(&conn).iter().any(|t| t == "alpha"));
    }

    #[test]
    fn legacy_user_version_db_without_meta_resets_on_first_open() {
        let (_dir, path) = temp_db("legacy.db");
        {
            // 旧迁移链的库: user_version, 无 schema_meta
            let conn = conn::open(&path).unwrap();
            conn.execute_batch(
                "create table alpha (
                    id    TEXT PRIMARY KEY,
                    name  TEXT NOT NULL,
                    note  TEXT,
                    score INTEGER NOT NULL DEFAULT 0
                );
                create table legacy_junk (x TEXT);",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 2i64).unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert!(
            matches!(outcome, SyncOutcome::Reset { .. }),
            "旧 user_version 库首开应重置, 实际 {outcome:?}"
        );
        assert_eq!(meta_get(&conn, "version_format").as_deref(), Some("date"));
        assert_eq!(meta_get(&conn, "version").as_deref(), Some(SCHEMA_VERSION));
        // 重置后是当前 model 的空 schema, 旧垃圾表不再存在
        assert!(!table_names(&conn).iter().any(|t| t == "legacy_junk"));
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 0, "user_version 不再承载版本语义");
    }

    #[test]
    fn unknown_version_format_resets_database() {
        let (_dir, path) = temp_db("badformat.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            meta_set(&conn, "version_format", "semver");
        }
        let (_conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert!(
            matches!(outcome, SyncOutcome::Reset { .. }),
            "未知格式应重置, 实际 {outcome:?}"
        );
    }

    #[test]
    fn version_value_differs_but_structure_matches_never_resets() {
        let (_dir, path) = temp_db("oldvalue.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            // 只改版本值 + 写一行业务数据
            meta_set(&conn, "version", "19990101");
            conn.execute(
                "INSERT INTO alpha (id, name) VALUES ('a1', 'keepme')",
                [],
            )
            .unwrap();
        }

        let (conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert_eq!(
            outcome,
            SyncOutcome::UpToDate,
            "结构一致时版本值不同绝不重置"
        );

        let value: String = conn
            .query_row("SELECT name FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "keepme", "数据应原样保留");
        assert_eq!(
            meta_get(&conn, "version").as_deref(),
            Some(SCHEMA_VERSION),
            "版本值随写更新"
        );
    }

    #[test]
    fn corrupt_db_refuses_to_open_and_file_is_untouched() {
        let (_dir, path) = temp_db("corrupt.db");
        let garbage = b"this is definitely not a sqlite database".to_vec();
        std::fs::write(&path, &garbage).unwrap();

        let err = open_and_sync(&path, TEST_TABLES).err().expect("损坏库必须拒启");
        assert!(
            err.contains("损坏") || err.contains("打开状态库失败"),
            "报错要说明损坏: {err}"
        );
        assert!(
            err.contains(path.file_name().unwrap().to_str().unwrap()),
            "报错要含路径: {err}"
        );

        let after = std::fs::read(&path).unwrap();
        assert_eq!(
            after, garbage,
            "损坏库文件一个字节都不能动 (重置不适用于损坏)"
        );
    }

    #[test]
    fn type_affinity_follows_sqlite_rules() {
        assert_eq!(type_affinity("INTEGER"), "INTEGER");
        assert_eq!(type_affinity("BIGINT"), "INTEGER");
        assert_eq!(type_affinity("VARCHAR(255)"), "TEXT");
        assert_eq!(type_affinity("TEXT"), "TEXT");
        assert_eq!(type_affinity(""), "BLOB");
        assert_eq!(type_affinity("BLOB"), "BLOB");
        assert_eq!(type_affinity("DOUBLE"), "REAL");
        assert_eq!(type_affinity("DECIMAL(10,5)"), "NUMERIC");
    }

    #[test]
    fn add_column_sql_includes_not_null_and_default() {
        let col = SchemaColumn {
            name: "flag",
            affinity: "INTEGER",
            default: Some("0"),
            not_null: true,
        };
        assert_eq!(
            add_column_sql("alpha", &col),
            "ALTER TABLE alpha ADD COLUMN flag INTEGER NOT NULL DEFAULT 0"
        );
        let nullable = SchemaColumn {
            name: "note",
            affinity: "TEXT",
            default: None,
            not_null: false,
        };
        assert_eq!(
            add_column_sql("alpha", &nullable),
            "ALTER TABLE alpha ADD COLUMN note TEXT"
        );
    }
}
