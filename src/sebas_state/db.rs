//! 数据库连接管理: 打开、WAL mode、busy_timeout、user_version。
//!
//! 所有操作是同步的(rusqlite 本身同步), 包裹在 `Connection` 上。
//! 异步调用者通过 `writer.rs` 的 actor 间接访问, 不直接调用这里的函数。

use rusqlite::{Connection, OpenFlags, Result as SqlResult};
use std::path::Path;

/// 打开 SQLite 数据库, 配置 WAL mode 和 busy_timeout。
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
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
}

/// 读取当前 schema version (`PRAGMA user_version`)。
pub fn user_version(conn: &Connection) -> SqlResult<u32> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version as u32)
}

/// 写入 schema version (`PRAGMA user_version = n`)。
/// 这是一个廉价操作, 不与事务冲突。
pub fn set_user_version(conn: &Connection, version: u32) -> SqlResult<()> {
    conn.pragma_update(None, "user_version", version as i64)
}

/// 检查数据库是否已打开且可用 (简单 ping)。
pub fn ping(conn: &Connection) -> bool {
    conn.query_row("SELECT 1", [], |_| Ok(()))
        .is_ok()
}

/// 当前 Unix 时间戳 (秒)。
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn opens_new_db_in_wal_mode() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = open(&path).expect("open");
        let journal: String = conn.pragma_query_value(None, "journal_mode", |row| row.get(0)).unwrap();
        assert_eq!(journal, "wal", "expected WAL mode, got {journal}");
        assert_eq!(user_version(&conn).unwrap(), 0, "fresh DB version 0");
    }

    #[test]
    fn busy_timeout_is_set() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("busy.db");
        let conn = open(&path).expect("open");
        // Check that busy_timeout is roughly 5000ms
        let timeout: i64 = conn.pragma_query_value(None, "busy_timeout", |row| row.get(0)).unwrap();
        assert!(timeout >= 4000, "busy_timeout too low: {timeout}");
    }

    #[test]
    fn user_version_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("version.db");
        let conn = open(&path).expect("open");
        set_user_version(&conn, 42).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 42);
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
    fn foreign_keys_enabled() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fk.db");
        let conn = open(&path).expect("open");
        let fk: i64 = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0)).unwrap();
        assert_eq!(fk, 1, "foreign_keys should be ON");
    }
}