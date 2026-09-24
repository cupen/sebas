//! 数据库连接配方：open / readonly / WAL / busy_timeout / foreign_keys。
//!
//! 唯一的 SQLite 连接入口（spec「A second database does not re-implement
//! the recipe」）：任何组件打开 SQLite 都从这里拿连接，不再本地拼 pragma。
//! 事务行为两种都提供（design D5），调用方按自身串行模型选择。
//!
//! # 文件权限（retire-legacy-state-json D4）
//!
//! 库文件含 provider `api_key`、card 快照等敏感配置，而 `settings.json` 时代
//! 本来就有 0600 的保证——迁入库不该成为权限回退。`open` 因此在创建/打开后
//! 把库文件与既有 `-wal` / `-shm` 收紧到所有者专用（0600），打开既有库时
//! 发现过宽也一并收紧。**收紧失败只告警，绝不中止启动**（安全加固不该变成
//! 无法启动）。状态**目录**的收紧是 `secure_directory`（0700）——由状态目录
//! 的持有者显式调用；`open` 不擅自改父目录（父目录可能是 `/tmp` 这类共享
//! 目录，改它是一场事故）。非 Unix 由 ACL 语义承担，与 `core.secret` 的既有
//! 处理一致。

use rusqlite::{Connection, OpenFlags, Result as SqlResult, Transaction};
use std::path::Path;

/// 库文件（及其 `-wal` / `-shm` 伴随文件）的所有者专用权限位：`-rw-------`。
pub const OWNER_ONLY_FILE: u32 = 0o600;

/// 状态目录的所有者专用权限位：`drwx------`。目录必须保留 `x` 位才可进入，
/// 所以**不是** 0600（0600 的目录不可遍历，等于把库锁死）。
pub const OWNER_ONLY_DIR: u32 = 0o700;

/// chmod 注入点：生产实现是 `std::fs::set_permissions`，测试注入失败闭包以
/// 覆盖「收紧失败」分支（design D4「告警但不中止启动」）。
pub type ChmodFn = dyn Fn(&Path, u32) -> std::io::Result<()>;

#[cfg(unix)]
fn real_chmod(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// 非 Unix：ACL 语义承担，chmod 是空操作。
#[cfg(not(unix))]
fn real_chmod(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// 权限比 `mode` 宽才收紧；比目标窄（如 0400）不动。路径不存在（如 `-wal`
/// 尚未创建）或收紧失败都只告警，**绝不**让调用方失败。
fn tighten_with(path: &Path, mode: u32, chmod: &ChmodFn) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Ok(md) = std::fs::symlink_metadata(path) else {
            return; // 不存在：没有权限可收紧。
        };
        let current = md.permissions().mode() & 0o777;
        if current & !mode == 0 {
            return; // 不比目标宽。
        }
        if let Err(e) = chmod(path, mode) {
            tracing::warn!(
                path = %path.display(),
                current = format!("{current:o}"),
                target = format!("{mode:o}"),
                error = %e,
                "failed to tighten state file permissions to owner-only (continuing startup)"
            );
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode, chmod);
    }
}

/// `path` 的同级伴随文件（SQLite 的 `-wal` / `-shm` 是全名追加后缀）。
fn sibling(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    std::path::PathBuf::from(os)
}

/// 把状态目录收紧到所有者专用（0700）。由状态目录的持有者在启动期显式调用
/// ——`open` 不碰父目录，避免误改 `/tmp` 这类共享目录。同样只告警不中止。
pub fn secure_directory(dir: &Path) {
    tighten_with(dir, OWNER_ONLY_DIR, &real_chmod);
}

/// 打开 SQLite 数据库，配置 WAL mode 和 busy_timeout。
///
/// - 数据库不存在时自动创建
/// - 启用 WAL journal mode
/// - 设置 busy_timeout = 5000ms (避免 SQLITE_BUSY 在并发测试中误报)
/// - 启用外键约束
/// - 库文件与既有 `-wal` / `-shm` 收紧到 0600（失败只告警，不中止启动）
pub fn open(path: &Path) -> SqlResult<Connection> {
    open_inner(path, &real_chmod)
}

fn open_inner(path: &Path, chmod: &ChmodFn) -> SqlResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;

    // 先收紧库文件：`-wal` / `-shm` 由 SQLite 按库文件的模式创建，库先收到
    // 0600 可让随后新建的伴随文件自动继承（不必事后追着 chmod）。
    tighten_with(path, OWNER_ONLY_FILE, chmod);

    // WAL mode: 读不阻塞写, 写不阻塞读。
    conn.pragma_update(None, "journal_mode", "wal")?;

    // 5s busy timeout: 避免 SQLITE_BUSY
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    // 外键约束 (默认关闭, 但我们的 schema 可能用到)
    conn.pragma_update(None, "foreign_keys", "ON")?;

    // 既有库上过宽的 `-wal` / `-shm`（老机器可能是 0644）一并收紧。
    tighten_with(path, OWNER_ONLY_FILE, chmod);
    for suffix in ["-wal", "-shm"] {
        tighten_with(&sibling(path, suffix), OWNER_ONLY_FILE, chmod);
    }

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

    // ---- 文件权限（retire-legacy-state-json 1.1 / 1.2 / 1.3）----

    /// 测试日志捕获的共享缓冲（本模块自带一份，避免跨模块测试可见性）。
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

    /// 权限位读取（Unix 专属；非 Unix 平台下这些用例不编译）。
    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// 1.1：新建库的权限是 0600（不是 umask 给的 0644）。
    #[cfg(unix)]
    #[test]
    fn new_db_file_is_owner_only_0600() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fresh.db");
        let _conn = open(&path).expect("open");
        assert_eq!(
            mode_of(&path),
            OWNER_ONLY_FILE,
            "新建库必须是 0600（库里含 provider api_key 与 card 快照）"
        );
    }

    /// 1.1：打开一个 **0644 的既有库** → 当场收紧为 0600。
    #[cfg(unix)]
    #[test]
    fn existing_loose_db_file_is_tightened_on_open() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        // 先造一个合法库，再把它改宽（模拟「老机器上本来就 0644」）。
        drop(open(&path).expect("create"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode_of(&path), 0o644, "前置条件：既有库是 0644");

        let _conn = open(&path).expect("reopen");
        assert_eq!(mode_of(&path), OWNER_ONLY_FILE, "打开既有库时应收紧到 0600");
    }

    /// 1.2：WAL 模式下 `-wal` 的权限不宽于库文件；已存在的过宽 `-wal` 也在
    /// open 时收紧。
    #[cfg(unix)]
    #[test]
    fn wal_sidecar_is_never_wider_than_the_db_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("wal.db");
        let wal = sibling(&path, "-wal");

        // 先起一个库并写一笔，逼 WAL 伴随文件出现（库 0600 → -wal 继承 0600）。
        // 连接保持存活：-wal 在最后一个连接关闭时被 checkpoint 删除，关了就拿
        // 不到它了。
        let conn = open(&path).expect("open");
        conn.execute_batch("create table t (x INTEGER)").unwrap();
        assert!(wal.exists(), "WAL 模式下写一笔应产生 -wal 文件");
        assert_eq!(
            mode_of(&wal) & !OWNER_ONLY_FILE,
            0,
            "-wal 不得比库文件宽：{:o}",
            mode_of(&wal)
        );

        // 老机器上 -wal 可能是 0644：再次 open 时一并收紧。
        std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _conn2 = open(&path).expect("reopen");
        assert_eq!(mode_of(&wal), OWNER_ONLY_FILE, "既有 -wal 应收紧到 0600");
        drop(conn);
    }

    /// 1.2：状态目录收紧到 0700（目录保留 x 位，不是 0600）；已经更窄的不动。
    #[cfg(unix)]
    #[test]
    fn state_directory_is_tightened_to_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let nested = dir.path().join("state");
        std::fs::create_dir(&nested).unwrap();
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o755)).unwrap();

        secure_directory(&nested);
        assert_eq!(mode_of(&nested), OWNER_ONLY_DIR, "状态目录应为 0700");

        // 收紧后目录仍可进入（0700 保留 x 位——0600 的目录会锁死库）。
        assert!(
            std::fs::write(nested.join("probe"), b"x").is_ok(),
            "0700 目录必须仍可写"
        );

        // 已更窄（0500）不动：收紧只对「更宽」生效。
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o500)).unwrap();
        secure_directory(&nested);
        assert_eq!(mode_of(&nested), 0o500, "更窄的权限不被放宽");
    }

    /// 1.3：权限收紧失败 → **告警但不中止启动**（open 照样返回可用连接）。
    /// 用注入的失败 chmod 模拟不可 chmod 的场景（如非属主 / 只读 fs）。
    #[cfg(unix)]
    #[test]
    fn tighten_failure_warns_but_startup_continues() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nochmod.db");
        // 先造出 0644 的库与 0644 的 -wal，让收紧路径一定会被走到。
        drop(open(&path).expect("create"));
        std::fs::write(sibling(&path, "-wal"), b"").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::set_permissions(
                &sibling(&path, "-wal"),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }

        let buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>> = Default::default();
        let captured = SharedBuf(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || captured.clone())
            .finish();

        let outcome = tracing::subscriber::with_default(subscriber, || {
            open_inner(&path, &|_p, _m| {
                Err(std::io::Error::other("simulated EPERM (not owner)"))
            })
        });

        let conn = outcome.expect("收紧失败绝不能让启动失败");
        assert!(ping(&conn), "连接仍然可用");
        let logs = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            logs.contains("failed to tighten state file permissions"),
            "收紧失败必须留下 warn，实际日志：{logs:?}"
        );
    }
}
