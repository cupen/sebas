//! 自动迁移框架: 事务性 DDL 迁移链, VACUUM INTO 备份, 高版本拒绝。
//!
//! # 迁移流程
//!
//! 打开 DB → 读 `PRAGMA user_version` → 比较二进制已知版本:
//! - 版本相等 → 正常服务
//! - 版本低于当前 → VACUUM INTO 备份 → 逐个跑迁移(每个单事务含版本提升)
//! - 版本高于当前 → 拒绝启动, 报错指明版本
//!
//! # 迁移链定义
//!
//! ```ignore
//! const MIGRATIONS: &[fn(&Transaction) -> Result] = &[
//!     migration_1_create_tables,
//! ];
//! ```
//!
//! 每个迁移函数接收一个 `Transaction`, 在其中执行 DDL/DML 并提升版本号。
//! 失败时整个事务回滚, 数据库版本不变。

use rusqlite::{Connection, Transaction, Result as SqlResult};
use std::path::Path;
use tracing::info;

/// 当前二进制已知的最高 schema 版本。
/// 每次新增迁移时 +1。
pub const CURRENT_VERSION: u32 = 1;

/// 迁移链: 顺序数组, 索引 = 版本号 - 1。
/// 迁移 1 = 建表 (无历史数据)。
pub const MIGRATIONS: &[fn(&Transaction<'_>) -> SqlResult<()>] = &[
    migration_1_create_tables,
];

/// 迁移 1: 创建所有初始表。
/// 这是第一个迁移, 不存在历史数据迁移。
fn migration_1_create_tables(tx: &Transaction<'_>) -> SqlResult<()> {
    tx.execute_batch(
        "
        -- providers: 软删 + JSON config
        CREATE TABLE IF NOT EXISTS providers (
            id          TEXT PRIMARY KEY,
            config      TEXT NOT NULL,       -- JSON blob
            deleted     INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL
        );

        -- model_aliases: 绑定到 provider
        CREATE TABLE IF NOT EXISTS model_aliases (
            alias           TEXT PRIMARY KEY,
            provider        TEXT NOT NULL REFERENCES providers(id),
            upstream_model  TEXT,
            created_at      INTEGER NOT NULL
        );

        -- settings: key-value, key = \"card_config\"
        CREATE TABLE IF NOT EXISTS settings (
            key     TEXT PRIMARY KEY,
            value   TEXT NOT NULL    -- JSON blob
        );

        -- projects: 项目注册表
        CREATE TABLE IF NOT EXISTS projects (
            path        TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            branch      TEXT,
            branch_at   INTEGER NOT NULL DEFAULT 0,
            added_at    INTEGER NOT NULL,
            sort_order  INTEGER NOT NULL DEFAULT 0
        );

        -- session_map: 会话映射 (预留, 本 change 不做 session map 迁移)
        CREATE TABLE IF NOT EXISTS session_map (
            chat_id         TEXT NOT NULL,
            thread_id       TEXT,
            session_id      TEXT NOT NULL,
            last_active_unix INTEGER NOT NULL,
            project_dir     TEXT,
            PRIMARY KEY (chat_id, thread_id)
        );

        -- 索引
        CREATE INDEX IF NOT EXISTS idx_providers_deleted ON providers(deleted);
        CREATE INDEX IF NOT EXISTS idx_model_aliases_provider ON model_aliases(provider);
        ",
    )?;

    // 提升版本号 (迁移 1 → version 1)
    tx.pragma_update(None, "user_version", 1i64)?;
    Ok(())
}

/// 迁移结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// 无需迁移 (版本已是最新)
    UpToDate,
    /// 迁移成功, 从旧版本升级到新版本
    Migrated { from: u32, to: u32 },
    /// 数据库版本高于二进制版本, 拒绝启动
    TooNew { db_version: u32, binary_version: u32 },
    /// 备份文件路径 (迁移成功时产生)
    BackupPath(String),
}

/// 执行迁移: 检查版本 → 备份 → 逐个迁移。
///
/// 返回迁移结果。失败时(包括备份失败)返回 `Err`。
pub fn run_migrations(conn: &mut Connection, db_path: &Path) -> Result<MigrationOutcome, String> {
    let db_version = crate::sebas_state::db::user_version(conn)
        .map_err(|e| format!("读取 user_version 失败: {e}"))?;

    if db_version > CURRENT_VERSION {
        return Ok(MigrationOutcome::TooNew {
            db_version,
            binary_version: CURRENT_VERSION,
        });
    }

    if db_version == CURRENT_VERSION {
        return Ok(MigrationOutcome::UpToDate);
    }

    // 需要迁移: 先备份
    let backup_path = backup_before(conn, db_path, db_version, CURRENT_VERSION)?;

    // 逐个迁移
    for version in (db_version + 1)..=CURRENT_VERSION {
        let idx = (version - 1) as usize;
        if idx >= MIGRATIONS.len() {
            return Err(format!(
                "迁移 {} 未在 MIGRATIONS 数组中定义 (最大索引 {})",
                version,
                MIGRATIONS.len()
            ));
        }
        let migration_fn = MIGRATIONS[idx];

        info!(
            from = db_version,
            to = version,
            "正在执行数据库迁移"
        );

        // 每个迁移在一个事务中执行: DDL 是事务性的, 失败时整体回滚。
        // rusqlite 的 Transaction 在 drop 时自动回滚, commit 后持久化。
        let tx = conn
            .transaction()
            .map_err(|e| format!("迁移 {version} 开始事务失败: {e}"))?;

        if let Err(e) = migration_fn(&tx) {
            // Transaction 自动回滚 (drop)
            return Err(format!("迁移 {version} 失败 (已回滚): {e}"));
        }

        tx.commit()
            .map_err(|e| format!("迁移 {version} 提交失败: {e}"))?;
    }

    info!(
        from = db_version,
        to = CURRENT_VERSION,
        backup = %backup_path,
        "数据库迁移完成"
    );

    Ok(MigrationOutcome::Migrated {
        from: db_version,
        to: CURRENT_VERSION,
    })
}

/// 迁移前 VACUUM INTO 备份: 生成 `{db_path}.backup-{from}-{to}`。
/// 保留最近一份, 旧的删除。
fn backup_before(
    conn: &Connection,
    db_path: &Path,
    from: u32,
    to: u32,
) -> Result<String, String> {
    let backup_path = format!(
        "{}.backup-{}-{}",
        db_path.display(),
        from,
        to
    );

    // VACUUM INTO 生成引擎背书的干净快照
    conn.execute_batch(&format!(
        "VACUUM INTO '{}'",
        backup_path.replace('\'', "''")
    ))
    .map_err(|e| format!("VACUUM INTO 备份失败: {e}"))?;

    // 删除旧的备份文件 (只保留最近一份)
    if let Some(parent) = db_path.parent() {
        let _prefix = format!("{}.backup-", db_path.display());
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path == Path::new(&backup_path) {
                    continue;
                }
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("sebas.db.backup-") {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }

    Ok(backup_path)
}

/// 检查数据库版本是否不超过当前二进制版本。
/// 如果版本过高, 返回错误信息。
pub fn refuse_if_too_new(conn: &Connection) -> Result<(), String> {
    let db_version = crate::sebas_state::db::user_version(conn)
        .map_err(|e| format!("读取 user_version 失败: {e}"))?;

    if db_version > CURRENT_VERSION {
        return Err(format!(
            "数据库版本 (v{db_version}) 高于当前二进制支持的版本 (v{CURRENT_VERSION})。\
             请使用新版 sebas 或恢复备份后重试。"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sebas_state::db::{open, user_version, set_user_version};
    use tempfile::tempdir;

    #[test]
    fn fresh_db_is_at_version_0() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fresh.db");
        let conn = open(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 0);
    }

    #[test]
    fn run_migrations_on_fresh_db_reaches_current_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("migrate.db");
        let mut conn = open(&path).unwrap();

        let outcome = run_migrations(&mut conn, &path).unwrap();
        assert_eq!(outcome, MigrationOutcome::Migrated { from: 0, to: CURRENT_VERSION });
        assert_eq!(user_version(&conn).unwrap(), CURRENT_VERSION);
    }

    #[test]
    fn already_current_returns_uptodate() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("uptodate.db");
        let mut conn = open(&path).unwrap();
        run_migrations(&mut conn, &path).unwrap(); // first run
        let outcome = run_migrations(&mut conn, &path).unwrap();
        assert_eq!(outcome, MigrationOutcome::UpToDate);
    }

    #[test]
    fn too_new_db_is_refused() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("toonew.db");
        let mut conn = open(&path).unwrap();
        // Manually set version higher than current
        set_user_version(&conn, CURRENT_VERSION + 1).unwrap();

        let outcome = run_migrations(&mut conn, &path).unwrap();
        assert_eq!(
            outcome,
            MigrationOutcome::TooNew {
                db_version: CURRENT_VERSION + 1,
                binary_version: CURRENT_VERSION,
            }
        );
    }

    #[test]
    fn backup_is_created_after_migration() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sebas.db");
        let mut conn = open(&path).unwrap();
        let outcome = run_migrations(&mut conn, &path).unwrap();
        if let MigrationOutcome::Migrated { from, to } = &outcome {
            let backup = dir.path().join(format!("sebas.db.backup-{from}-{to}"));
            assert!(backup.exists(), "backup file should exist: {:?}", backup);
            // Open the backup and verify it's at version 0 (before migration)
            let backup_conn = open(&backup).unwrap();
            assert_eq!(user_version(&backup_conn).unwrap(), 0);
        } else {
            panic!("expected Migrated outcome, got {:?}", outcome);
        }
    }

    #[test]
    fn migration_creates_tables() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("schema.db");
        let mut conn = open(&path).unwrap();
        run_migrations(&mut conn, &path).unwrap();

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        for expected in &["model_aliases", "projects", "providers", "session_map", "settings"] {
            assert!(
                tables.iter().any(|t| t == expected),
                "table {expected} not found in {:?}",
                tables
            );
        }
    }

    #[test]
    fn refuse_if_too_new_errors_on_high_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("refuse.db");
        let conn = open(&path).unwrap();
        set_user_version(&conn, CURRENT_VERSION + 1).unwrap();
        assert!(refuse_if_too_new(&conn).is_err());
    }

    #[test]
    fn refuse_if_too_new_ok_on_current() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("accept.db");
        let mut conn = open(&path).unwrap();
        run_migrations(&mut conn, &path).unwrap();
        assert!(refuse_if_too_new(&conn).is_ok());
    }
}