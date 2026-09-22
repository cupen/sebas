//! 专职写者 actor: 专用线程 + mpsc mailbox + oneshot 应答（extract-sebas-db
//! 2.3 下沉，design D4：整体搬迁，线程名 / 通道容量 / 就绪信号 / 错误语义
//! 一律不变）。
//!
//! 所有数据库操作通过 [`StateHandle`] 提交, 由 [`StateWriter`] 在专用线程中
//! 串行执行。命令是 `Box<dyn FnOnce(&mut Connection)…>`，对表结构一无所知
//! ——域无关（spec「Commands serialize through one owner」）。
//!
//! # 模式
//!
//! ```text
//! StateHandle (Clone, Send)
//!     │
//!     ▼  mpsc::Sender<Command>        (容量 128)
//! StateWriter (专用线程 "sebas-state-db")
//!     │
//!     ▼  rusqlite::Connection (同步; 启动时 open + schema sync)
//! ```
//!
//! 在泛型 [`Record`] 之上另有类型化门面
//! （[`StateHandle::save`] / [`find`](StateHandle::find) / [`all`](StateHandle::all)
//! / [`delete`](StateHandle::delete)）——门面仍不认识任何域类型；多表原子性
//! 走 [`StateHandle::exec`] 闭包（unit-of-work）。

use crate::record::Record;
use crate::schema::TableSchema;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use tokio::sync::{mpsc, oneshot};

/// 写者命令: 闭包 `FnOnce(&mut Connection) -> R + Send`, 结果通过 oneshot 返回。
type Cmd = Box<dyn FnOnce(&mut Connection) -> Result<Box<dyn std::any::Any + Send>, String> + Send>;

/// 单条写命令的执行结果（动态擦除，调用方 downcast）。
type CmdOutcome = Result<Box<dyn std::any::Any + Send>, String>;

/// 异步句柄, 克隆后多消费者共享同一写者线程。
#[derive(Clone)]
pub struct StateHandle {
    tx: mpsc::Sender<(Cmd, oneshot::Sender<CmdOutcome>)>,
}

impl StateHandle {
    /// 提交一个命令到写者线程, 等待结果。
    pub async fn exec<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<R, String> + Send + 'static,
    ) -> Result<R, String> {
        let (tx, rx) = oneshot::channel();
        let cmd: Cmd =
            Box::new(move |conn| f(conn).map(|v| Box::new(v) as Box<dyn std::any::Any + Send>));
        self.tx
            .send((cmd, tx))
            .await
            .map_err(|_| "state writer 已关闭".to_string())?;
        let result = rx
            .await
            .map_err(|_| "state writer 响应通道已关闭".to_string())?;
        // 把 Box<dyn Any> 转回 R
        result.map(|any| {
            *any.downcast::<R>()
                .expect("type mismatch in StateHandle::exec")
        })
    }

    /// 提交一个返回 `()` 的命令 (不关心返回值)。
    pub async fn exec_void(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<(), String> + Send + 'static,
    ) -> Result<(), String> {
        self.exec(move |conn| f(conn)).await
    }

    /// 类型化门面：保存一行（按主键 upsert），经单写线程串行执行。
    pub async fn save<R>(&self, record: &R) -> Result<(), String>
    where
        R: Record + Clone + Send + 'static,
    {
        let record = record.clone();
        self.exec(move |conn| crate::record::save(conn, &record).map_err(|e| e.to_string()))
            .await
    }

    /// 类型化门面：按单列主键取一行。
    pub async fn find<R, P>(&self, pk: P) -> Result<Option<R>, String>
    where
        R: Record + Send + 'static,
        P: rusqlite::ToSql + Send + 'static,
    {
        self.exec(move |conn| crate::record::find(conn, pk).map_err(|e| e.to_string()))
            .await
    }

    /// 类型化门面：取全表行。
    pub async fn all<R>(&self) -> Result<Vec<R>, String>
    where
        R: Record + Send + 'static,
    {
        self.exec(move |conn| crate::record::all(conn).map_err(|e| e.to_string()))
            .await
    }

    /// 类型化门面：按单列主键删一行。返回是否确有行被删。
    pub async fn delete<R, P>(&self, pk: P) -> Result<bool, String>
    where
        R: Record + Send + 'static,
        P: rusqlite::ToSql + Send + 'static,
    {
        self.exec(move |conn| {
            crate::record::delete::<R, P>(conn, pk).map_err(|e| e.to_string())
        })
        .await
    }
}

/// 专职写者线程。
///
/// 启动后, 在专用线程中循环接收命令, 串行执行。
/// `drop` 时自动关闭 mpsc 通道, 写者线程退出。
pub struct StateWriter {
    handle: StateHandle,
    #[allow(dead_code)]
    join_handle: Option<std::thread::JoinHandle<()>>,
}

impl StateWriter {
    /// 启动写者线程, 使用给定的数据库路径与注册表。
    /// 会自动打开/创建数据库并同步 schema（tables 由调用方传入——域 schema
    /// 事实留在域侧，actor 只管"怎么开、怎么串行"）。
    /// 在同步完成前阻塞, 返回后 DB 已就绪。
    pub fn start(db_path: PathBuf, tables: &'static [TableSchema]) -> Result<Self, String> {
        let (tx, mut rx) = mpsc::channel::<(
            Cmd,
            oneshot::Sender<Result<Box<dyn std::any::Any + Send>, String>>,
        )>(128);
        let (ready_tx, ready_rx) = std_mpsc::channel::<Result<(), String>>();

        let join_handle = std::thread::Builder::new()
            .name("sebas-state-db".into())
            .spawn(move || {
                // 打开 + schema 同步: open 失败按损坏拒启,
                // 不兼容由 sync 层重置重建, 这里只区分"就绪/失败"。
                let mut conn =
                    match crate::schema::open_and_sync(&db_path, tables) {
                        Ok((conn, outcome)) => {
                            tracing::info!(path = %db_path.display(), outcome = ?outcome, "state store schema synced");
                            conn
                        }
                        Err(e) => {
                            tracing::error!(path = %db_path.display(), error = %e, "state writer failed to open/sync database");
                            let _ = ready_tx.send(Err(format!("状态库初始化失败: {e}")));
                            return;
                        }
                    };

                tracing::info!(path = %db_path.display(), "state writer ready");
                let _ = ready_tx.send(Ok(()));

                // 命令循环: 串行处理
                while let Some((cmd, tx)) = rx.blocking_recv() {
                    let result = cmd(&mut conn);
                    // 如果接收端已关闭, 忽略
                    let _ = tx.send(result);
                }

                tracing::info!("state writer stopped");
            })
            .map_err(|e| format!("创建 state writer 线程失败: {e}"))?;

        // 等待打开 + schema 同步完成
        ready_rx
            .recv()
            .map_err(|_| "state writer 启动失败: 通道关闭".to_string())??;

        Ok(Self {
            handle: StateHandle { tx },
            join_handle: Some(join_handle),
        })
    }

    /// 获取异步句柄。
    pub fn handle(&self) -> &StateHandle {
        &self.handle
    }
}

/// 测试用: 创建 writer 和 handle, 返回 writer 和独立的 handle。
/// 生产环境只启动一次。
#[cfg(test)]
impl StateWriter {
    pub fn start_test(
        db_path: PathBuf,
        tables: &'static [TableSchema],
    ) -> Result<(Self, StateHandle), String> {
        let writer = Self::start(db_path, tables)?;
        let handle = writer.handle.clone();
        Ok((writer, handle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, KvRow};
    use tempfile::tempdir;

    #[tokio::test]
    async fn writer_refuses_corrupt_database_at_startup() {
        // state-store spec「Corrupt store is not silently reset」: 损坏库必须
        // 拒启并报出路径, 且绝不自动删除/重建文件 (与 schema 不兼容的重置严格分离)。
        let dir = tempdir().unwrap();
        let path = dir.path().join("corrupt.db");
        let garbage = b"not a sqlite database at all".to_vec();
        std::fs::write(&path, &garbage).unwrap();

        let err = StateWriter::start(path.clone(), fixtures::TEST_TABLES)
            .err()
            .expect("corrupt db must abort startup");
        assert!(
            err.contains("初始化失败") || err.contains("损坏"),
            "unexpected error: {err}"
        );

        // 跨重启同样拒启, 文件始终原样
        for _ in 0..2 {
            let err = StateWriter::start(path.clone(), fixtures::TEST_TABLES)
                .err()
                .expect("corrupt db must abort startup");
            assert!(err.contains("初始化失败") || err.contains("损坏"));
            assert_eq!(std::fs::read(&path).unwrap(), garbage, "损坏文件不得被改动");
        }
    }

    #[tokio::test]
    async fn writer_executes_commands() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("writer.db");
        let writer = StateWriter::start(path.clone(), fixtures::TEST_TABLES).unwrap();
        let handle = writer.handle.clone();

        let result = handle
            .exec(move |conn| {
                let v: i64 = conn
                    .query_row("SELECT 1 + 1", [], |row| row.get(0))
                    .map_err(|e| e.to_string())?;
                Ok(v)
            })
            .await
            .unwrap();

        assert_eq!(result, 2);
    }

    #[tokio::test]
    async fn writer_handles_void_commands() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("void.db");
        let writer = StateWriter::start(path.clone(), fixtures::TEST_TABLES).unwrap();
        let handle = writer.handle.clone();

        // 写数据
        handle
            .exec_void(move |conn| {
                conn.execute("INSERT INTO alpha (id, name) VALUES ('a', 'b')", [])
                    .map_err(|e| e.to_string())?;
                Ok(())
            })
            .await
            .unwrap();

        // 读回来
        let val: String = handle
            .exec(move |conn| {
                conn.query_row("SELECT name FROM alpha WHERE id = 'a'", [], |row| {
                    row.get(0)
                })
                .map_err(|e| e.to_string())
            })
            .await
            .unwrap();

        assert_eq!(val, "b");
    }

    #[tokio::test]
    async fn writer_handles_errors_gracefully() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("error.db");
        let writer = StateWriter::start(path.clone(), fixtures::TEST_TABLES).unwrap();
        let handle = writer.handle.clone();

        let result: Result<i64, String> = handle
            .exec(move |conn| {
                // 查询不存在的表
                let v: i64 = conn
                    .query_row("SELECT x FROM nonexistent", [], |row| row.get(0))
                    .map_err(|e| e.to_string())?;
                Ok(v)
            })
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn writer_sync_creates_schema_and_stamps_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sync_writer.db");
        let _writer = StateWriter::start(path.clone(), fixtures::TEST_TABLES).unwrap();

        // 直接打开数据库验证 schema 同步已执行
        let conn = crate::conn::open(&path).unwrap();
        let version_format: String = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'version_format'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let version: String = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version_format, "date");
        assert_eq!(version, crate::schema::SCHEMA_VERSION);

        // 验证注册表 + schema_meta 都存在
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count >= 2, "expected at least 2 tables, got {count}");
    }

    #[tokio::test]
    async fn writer_serializes_concurrent_commands() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};

        let dir = tempdir().unwrap();
        let path = dir.path().join("concurrent.db");
        let writer = StateWriter::start(path.clone(), fixtures::KV_TABLES).unwrap();
        let handle = writer.handle.clone();

        // 并发 50 个写入, 验证写者线程串行化
        let counter = Arc::new(AtomicU64::new(0));
        let mut tasks = Vec::new();
        for i in 0..50u64 {
            let h = handle.clone();
            let c = counter.clone();
            tasks.push(tokio::spawn(async move {
                h.save(&KvRow {
                    key: format!("k{i}"),
                    value: i.to_string(),
                    flag: false,
                })
                .await
                .unwrap();
                c.fetch_add(1, Ordering::SeqCst);
            }));
        }

        for t in tasks {
            t.await.unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), 50);

        // 验证所有 50 行都写入（门面读取经同一写者线程）
        let rows = handle.all::<KvRow>().await.unwrap();
        assert_eq!(rows.len(), 50);
    }

    /// 类型化门面（3.4 验收）：save / find / all / delete 都经单写线程执行。
    #[tokio::test]
    async fn typed_facade_round_trips_through_writer() {
        let dir = tempdir().unwrap();
        let writer =
            StateWriter::start(dir.path().join("facade.db"), fixtures::KV_TABLES).unwrap();
        let handle = writer.handle().clone();

        let row = KvRow { key: "k".into(), value: "v".into(), flag: true };
        handle.save(&row).await.unwrap();

        let found = handle.find::<KvRow, _>("k").await.unwrap().expect("命中");
        assert_eq!(found, row);

        let deleted = handle.delete::<KvRow, _>("k").await.unwrap();
        assert!(deleted);
        assert!(handle.find::<KvRow, _>("k").await.unwrap().is_none());
        assert!(handle.all::<KvRow>().await.unwrap().is_empty());
    }

    /// 多表/多行原子性 = exec 闭包整体提交或整体回滚（D3b 验收）。
    #[tokio::test]
    async fn exec_closure_transaction_commits_or_rolls_back_whole() {
        let dir = tempdir().unwrap();
        let writer =
            StateWriter::start(dir.path().join("atomic.db"), fixtures::KV_TABLES).unwrap();
        let handle = writer.handle().clone();

        // 一个事务闭包写两行后中途报错 → 两行都不落库。
        let result = handle
            .exec_void(move |conn| {
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                crate::record::save(
                    &tx,
                    &KvRow { key: "a".into(), value: "1".into(), flag: false },
                )
                .map_err(|e| e.to_string())?;
                crate::record::save(
                    &tx,
                    &KvRow { key: "b".into(), value: "2".into(), flag: false },
                )
                .map_err(|e| e.to_string())?;
                let _ = tx; // 有意不 commit，返回错误触发 drop 回滚
                Err("mid-transaction failure".to_string())
            })
            .await;
        assert_eq!(result, Err("mid-transaction failure".to_string()));
        assert!(
            handle.all::<KvRow>().await.unwrap().is_empty(),
            "闭包事务必须整体回滚"
        );

        // 正常提交：两行都在。
        handle
            .exec_void(move |conn| {
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                crate::record::save(
                    &tx,
                    &KvRow { key: "a".into(), value: "1".into(), flag: false },
                )
                .map_err(|e| e.to_string())?;
                crate::record::save(
                    &tx,
                    &KvRow { key: "b".into(), value: "2".into(), flag: false },
                )
                .map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())
            })
            .await
            .unwrap();
        assert_eq!(handle.all::<KvRow>().await.unwrap().len(), 2);
    }
}
