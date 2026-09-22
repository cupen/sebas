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
//!    - 缺列但补不了 / 类型不符 / 多余列 / 整表缺失 → **重置**: 先把旧库文件
//!      (含 `-wal`/`-shm`) 隔离为 `<path>.reset-<unix>` (改名不删除, 可手工
//!      恢复), 再按注册 DDL 重建空 schema; WARN 日志写明触发点与隔离路径,
//!      并明说旧数据未迁入新库 (quarantine-database-reset)。
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
    /// schema 不兼容, 已隔离旧库并重建空 schema (原因见 reason, 隔离路径见日志)。
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
            // WARN (触发原因 + 隔离路径 + "未迁入") 由 reset_and_rebuild 打。
            let (conn, _quarantined) = reset_and_rebuild(db_path, tables, &reason)?;
            Ok((conn, SyncOutcome::Reset { reason }))
        }
    }
}

/// 对已打开的连接执行同步。不兼容时先关闭连接再返回 (调用方隔离旧库才安全)。
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

/// 重置: 先把旧库文件 (含 `-wal`/`-shm`) 隔离为 `<path>.reset-<unix>`——
/// 改名而非删除, 同一时间戳, 可手工恢复 (quarantine-database-reset D2)——
/// 再按注册 DDL 重建空 schema。返回 (新连接, 实际隔离的文件路径)。
///
/// 隔离文件是**重置前的完整库**: 调用点 (sync_conn) 返回 Incompatible 前已
/// drop 旧连接, 关闭即提交、WAL checkpoint 落盘, 因此最近一次提交在隔离文件
/// 中可见。旧数据**不迁入**新库 (日志明说), 恢复只能手工进行。调用点已保证
/// open 失败的损坏路径进不到这里——损坏拒启, 绝不重置。
pub fn reset_and_rebuild(
    db_path: &Path,
    tables: &'static [TableSchema],
    reason: &str,
) -> Result<(Connection, Vec<PathBuf>), String> {
    let quarantined = quarantine_db_files(db_path)?;
    let quarantined_list = quarantined
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    warn!(
        path = %db_path.display(),
        reason = %reason,
        quarantined = %quarantined_list,
        "schema 不兼容: 旧库(含 -wal/-shm)已隔离未删除, 按当前 model 重建空 schema; \
         旧数据未迁入新库, 如需找回请手工打开隔离文件恢复"
    );

    let mut conn = conn::open(db_path)
        .map_err(|e| format!("重置后重新打开状态库失败: {}: {e}", db_path.display()))?;
    rebuild_schema(&mut conn, tables)?;
    Ok((conn, quarantined))
}

/// 隔离旧库三件套 (主文件 + `-wal`/`-shm`, 同一时间戳), 返回实际改名成功的
/// 路径。先隔离 sidecar、最后隔离主文件——中途被打断时主库也不会带着旧
/// sidecar 复活。
fn quarantine_db_files(db_path: &Path) -> Result<Vec<PathBuf>, String> {
    let base = quarantine_base_suffix(db_path, unix_secs());
    let mut quarantined = Vec::new();
    for suffix in ["-shm", "-wal", ""] {
        let file = db_sidecar_path(db_path, suffix);
        let target = db_sidecar_path(db_path, &format!("{base}{suffix}"));
        match std::fs::rename(&file, &target) {
            Ok(()) => quarantined.push(target),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "重置状态库失败: 隔离 {} 为 {} 出错: {e}",
                    file.display(),
                    target.display()
                ))
            }
        }
    }
    Ok(quarantined)
}

/// 隔离基名后缀: 首选 `.reset-<unix>`; 同一秒内已有重置产物 (任一
/// `<base>` / `<base>-wal` / `<base>-shm` 存在) 时追加 `-{pid}`, 仍占用再
/// 追加序号 (design Open Question: 进程号 + 序号)。
fn quarantine_base_suffix(db_path: &Path, stamp: u64) -> String {
    let pid = std::process::id();
    // 惰性候选流 (不得 extend 进 Vec——开放区间的 size_hint 会按上限预分配)。
    [format!(".reset-{stamp}"), format!(".reset-{stamp}-{pid}")]
        .into_iter()
        .chain((1u32..).map(|n| format!(".reset-{stamp}-{pid}-{n}")))
        .find(|base| !quarantine_base_taken(db_path, base))
        .expect("无限序号候选中必有未占用者")
}

fn quarantine_base_taken(db_path: &Path, base: &str) -> bool {
    ["", "-wal", "-shm"]
        .iter()
        .any(|sfx| db_sidecar_path(db_path, &format!("{base}{sfx}")).exists())
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

    /// 目录下的隔离产物 (`*.reset-*`)。
    fn quarantine_files_in_dir(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().contains(".reset-"))
            .collect();
        files.sort();
        files
    }

    /// 测试日志捕获的共享缓冲 (fmt subscriber 的 writer)。
    #[derive(Clone)]
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
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

    /// 2.1: 多余列重置 → 原行在隔离文件中存活, 新库为空。
    #[test]
    fn extra_column_resets_database_with_rows_surviving_in_quarantine() {
        let (dir, path) = temp_db("extracol.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            conn.execute_batch("ALTER TABLE alpha ADD COLUMN stale TEXT;")
                .unwrap();
            // 放一行数据证明隔离文件保存的是重置前的完整库
            conn.execute(
                "INSERT INTO alpha (id, name) VALUES ('a1', 'keepme')",
                [],
            )
            .unwrap();
            // drop = 关闭 = 提交落盘 (1.2): 此后触发重置, 最近一次提交必须
            // 在隔离文件中可见。
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

        // 新库: 空 schema, marker 行不迁入, 版本键已打
        assert!(!column_names(&conn, "alpha").iter().any(|c| c == "stale"));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM alpha", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "重置后是空 schema, marker 行不在新库");
        assert_eq!(meta_get(&conn, "version").as_deref(), Some(SCHEMA_VERSION));

        // 隔离文件存在且是重置前的完整库
        let quarantined = quarantine_files_in_dir(dir.path());
        assert_eq!(quarantined.len(), 1, "只应隔离主文件: {quarantined:?}");
        let qconn = conn::open_readonly(&quarantined[0]).unwrap();
        let name: String = qconn
            .query_row("SELECT name FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "keepme", "重置前的行应存活在隔离文件中");
        let qcount: i64 = qconn
            .query_row("SELECT COUNT(*) FROM alpha", [], |r| r.get(0))
            .unwrap();
        assert_eq!(qcount, 1);
        assert!(
            column_names(&qconn, "alpha").iter().any(|c| c == "stale"),
            "隔离文件保留重置前的结构"
        );
    }

    /// 1.1: 重置隔离三件套 (主文件 + `-wal`/`-shm`) 且用同一时间戳, 新库为
    /// 空 schema。
    #[test]
    fn reset_quarantines_db_file_with_wal_and_shm_under_one_timestamp() {
        let (dir, path) = temp_db("three.db");
        // 旧三件套手工摆放 (内容不重要——只被改名, 不会被打开)
        std::fs::write(&path, b"old db bytes, never opened by reset").unwrap();
        std::fs::write(db_sidecar_path(&path, "-wal"), b"old wal").unwrap();
        std::fs::write(db_sidecar_path(&path, "-shm"), b"old shm").unwrap();

        let (conn, quarantined) =
            reset_and_rebuild(&path, TEST_TABLES, "测试: 三件套隔离").unwrap();
        assert_eq!(quarantined.len(), 3, "主文件与两个 sidecar 都要隔离: {quarantined:?}");

        // 同一时间戳: 两个 sidecar 的隔离路径 = 主隔离路径 + 同后缀
        let qmain = quarantined
            .iter()
            .find(|p| {
                let n = p.file_name().unwrap().to_string_lossy();
                !n.ends_with("-wal") && !n.ends_with("-shm")
            })
            .expect("主文件隔离路径");
        let qname = qmain.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            qname.contains(".reset-"),
            "隔离名形如 <path>.reset-<unix>: {qname}"
        );
        for sfx in ["-wal", "-shm"] {
            let sidecar = db_sidecar_path(qmain, sfx);
            assert!(
                quarantined.contains(&sidecar) && sidecar.exists(),
                "sidecar {sfx} 应随主文件同一时间戳隔离: {quarantined:?}"
            );
        }
        assert!(qmain.exists(), "主隔离文件存在: {qmain:?}");
        // 新库为空 schema
        assert_eq!(meta_get(&conn, "version").as_deref(), Some(SCHEMA_VERSION));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM alpha", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "重建的新库是空 schema");
        // 隔离产物只在 <db> 同目录, 不碰别处
        assert_eq!(
            quarantine_files_in_dir(dir.path()).len(),
            3,
            "目录里恰好三个隔离文件"
        );
    }

    /// 1.3: 重置日志同时给出触发原因与隔离路径, 并明说数据未迁入新库。
    #[test]
    fn reset_log_names_reason_quarantine_path_and_non_migration() {
        let (dir, path) = temp_db("logcap.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            conn.execute_batch("ALTER TABLE alpha ADD COLUMN stale TEXT;")
                .unwrap();
        }

        let buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>> = Default::default();
        {
            let captured = SharedBuf(buf.clone());
            let subscriber = tracing_subscriber::fmt()
                .with_ansi(false)
                .with_max_level(tracing::Level::WARN)
                .with_writer(move || captured.clone())
                .finish();
            // 必须全局安装（进程内只此一处）：scoped dispatcher（with_default）
            // 不参与 callsite interest 的全局缓存——并行的重置测试线程会抢先把
            // 产品 warn! callsite 注册成 never，scoped subscriber 因此永远收不到
            // 事件（捕获为空，测试顺序依赖）。全局 subscriber 注册时重建全部
            // callsite 的 interest，此后首次注册的 callsite 也能看到它，捕获
            // 因而是确定的。并行的重置测试可能同样写进缓冲，但断言只做
            // contains，互不干扰。
            tracing::subscriber::set_global_default(subscriber)
                .expect("进程内只允许这一处全局 subscriber");
            let (_conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
            assert!(
                matches!(outcome, SyncOutcome::Reset { .. }),
                "前置: 该场景应触发重置, 实际 {outcome:?}"
            );
        }
        let logs = String::from_utf8(buf.lock().unwrap().clone()).unwrap();

        assert!(logs.contains("stale"), "日志要点名触发列: {logs}");
        let quarantined = quarantine_files_in_dir(dir.path());
        assert_eq!(quarantined.len(), 1);
        let qname = quarantined[0].file_name().unwrap().to_string_lossy();
        assert!(
            logs.contains(&*qname),
            "日志要给出隔离路径 {qname}: {logs}"
        );
        assert!(logs.contains("未迁入"), "日志要明说数据未迁入新库: {logs}");
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

    /// 2.2: 未知版本格式 → 隔离而非删除, 隔离文件是可读的旧库。
    #[test]
    fn unknown_version_format_quarantines_file_instead_of_deleting() {
        let (dir, path) = temp_db("badformat2.db");
        {
            let (conn, _) = open_and_sync(&path, TEST_TABLES).unwrap();
            conn.execute(
                "INSERT INTO alpha (id, name) VALUES ('a1', 'keepme')",
                [],
            )
            .unwrap();
            meta_set(&conn, "version_format", "semver");
        }
        let (_conn, outcome) = open_and_sync(&path, TEST_TABLES).unwrap();
        assert!(
            matches!(outcome, SyncOutcome::Reset { .. }),
            "未知格式应重置, 实际 {outcome:?}"
        );

        let quarantined = quarantine_files_in_dir(dir.path());
        assert_eq!(quarantined.len(), 1, "未知格式也应隔离而非删除: {quarantined:?}");
        let qconn = conn::open_readonly(&quarantined[0]).unwrap();
        let name: String = qconn
            .query_row("SELECT name FROM alpha WHERE id = 'a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "keepme", "隔离文件应能读到重置前的行");
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

    /// 2.3: 损坏路径绝不产生隔离文件 (重置只属于结构不兼容)。
    #[test]
    fn corrupt_db_refusal_produces_no_quarantine_files() {
        let (dir, path) = temp_db("corrupt2.db");
        std::fs::write(&path, b"this is definitely not a sqlite database").unwrap();

        assert!(
            open_and_sync(&path, TEST_TABLES).is_err(),
            "损坏库必须拒启"
        );
        assert!(
            quarantine_files_in_dir(dir.path()).is_empty(),
            "损坏路径不得产生任何隔离文件"
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
