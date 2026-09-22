//! 数据库连接配方：open / readonly / WAL / busy_timeout / foreign_keys。
//!
//! 唯一的 SQLite 连接入口（spec「A second database does not re-implement
//! the recipe」）：任何组件打开 SQLite 都从这里拿连接，不再本地拼 pragma。
//! 事务行为两种都提供（design D5），调用方按自身串行模型选择。

use rusqlite::{Connection, OpenFlags, Result as SqlResult, Transaction};
use std::path::Path;

/// 打开 SQLite 数据库，配置 WAL mode 和 busy_timeout。
///
/// - 数据库不存在时自动创建
/// - 启用 WAL journal mode
/// - 设置 busy_timeout = 5000ms (避免 SQLITE_BUSY 在并发测试中误报)
/// - 启用外键约束
pub fn open(path: &Path) -> SqlResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;

    // WAL mode: 读不阻塞写, 写不阻塞读。
    conn.pragma_update(None, "journal_mode", "wal")?;

    // 5s busy timeout: 避免 SQLITE_BUSY
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    // 外键约束 (默认关闭, 但我们的 schema 可能用到)
    conn.pragma_update(None, "foreign_keys", "ON")?;

    Ok(conn)
}

/// 只读打开 (不创建, 非 WAL 模式, 用于存在性检查/诊断)。
pub fn open_readonly(path: &Path) -> SqlResult<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

/// 默认（deferred）事务入口：读事务到首个写才升级锁。配单写线程 actor
/// 使用（`sebas.db` 的既有形态）。
pub fn transaction(conn: &mut Connection) -> SqlResult<Transaction<'_>> {
    conn.transaction()
}

/// `TransactionBehavior::Immediate` 事务入口：开启即取写锁，避免
/// 读-升级-写之间的锁竞争窗口。配调用方自持 `Mutex` 的串行模型使用
/// （`auth.db` 的既有形态，design D5——共享层不改任何调用方的选择）。
pub fn transaction_immediate(conn: &mut Connection) -> SqlResult<Transaction<'_>> {
    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
}

/// 检查数据库是否已打开且可用 (简单 ping)。
pub fn ping(conn: &Connection) -> bool {
    conn.query_row("SELECT 1", [], |_| Ok(())).is_ok()
}

/// 读 `PRAGMA user_version`（连接机制原语——版本**语义**归调用方：
/// `auth.db` 以它做兼容性拒绝，`sebas.db` 的版本自描述在 `schema_meta`）。
pub fn user_version(conn: &Connection) -> SqlResult<i64> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
}

/// 写 `PRAGMA user_version`。语义同上，由调用方决定是否/何时写。
pub fn set_user_version(conn: &Connection, version: i64) -> SqlResult<()> {
    conn.pragma_update(None, "user_version", version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// 配方三件套可直接观测（design「Risks」）：不靠代码比对，打开后读回。
    #[test]
    fn recipe_pragmas_are_wal_busy5s_foreign_keys() {
        let dir = tempdir().unwrap();
        let conn = open(&dir.path().join("recipe.db")).expect("open");

        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "wal", "expected WAL mode, got {journal}");

        let timeout: i64 = conn
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .unwrap();
        assert_eq!(timeout, 5000, "busy_timeout must be exactly 5000ms");

        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1, "foreign_keys should be ON");
    }

    #[test]
    fn opens_new_db_in_wal_mode() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open(&path).expect("open");
        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "wal", "expected WAL mode, got {journal}");
    }

    #[test]
    fn readonly_open_fails_on_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.db");
        assert!(open_readonly(&path).is_err());
    }

    #[test]
    fn ping_returns_true_for_open_db() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ping.db");
        let conn = open(&path).expect("open");
        assert!(ping(&conn));
    }

    #[test]
    fn user_version_round_trips() {
        let dir = tempdir().unwrap();
        let conn = open(&dir.path().join("uv.db")).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 0);
        set_user_version(&conn, 7).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 7);
    }

    /// 两种事务入口都能开、都能提交（行为差异不在此断言——那是调用方的
    /// 既有并发测试的职责，design D5）。
    #[test]
    fn both_transaction_entry_points_work() {
        let dir = tempdir().unwrap();
        let mut conn = open(&dir.path().join("tx.db")).unwrap();
        conn.execute_batch("create table t (x INTEGER)").unwrap();

        let tx = transaction(&mut conn).unwrap();
        tx.execute("INSERT INTO t (x) VALUES (1)", []).unwrap();
        tx.commit().unwrap();

        let tx = transaction_immediate(&mut conn).unwrap();
        tx.execute("INSERT INTO t (x) VALUES (2)", []).unwrap();
        tx.commit().unwrap();

        let n: i64 = conn.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
    }
}
