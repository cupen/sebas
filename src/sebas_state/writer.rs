//! 专职写者 actor: 专用线程 + mpsc mailbox + oneshot 应答。
//!
//! 所有数据库操作通过 `StateHandle` 提交, 由 `StateWriter` 在专用线程中
//! 串行执行。这与项目中 `SessionManager` 的 actor 模式同构。
//!
//! # 模式
//!
//! ```ignore
//! StateHandle (Clone, Send)
//!     │
//!     ▼  mpsc::Sender<Command>
//! StateWriter (专用线程)
//!     │
//!     ▼  rusqlite::Connection (同步)
//! ```
//!
//! 每个命令是一个闭包, 通过 `oneshot` 通道返回结果。

use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use tokio::sync::{mpsc, oneshot};

/// 写者命令: 闭包 `FnOnce(&mut Connection) -> R + Send`, 结果通过 oneshot 返回。
type Cmd = Box<dyn FnOnce(&mut Connection) -> Result<Box<dyn std::any::Any + Send>, String> + Send>;

/// 异步句柄, 克隆后多消费者共享同一写者线程。
#[derive(Clone)]
pub struct StateHandle {
    tx: mpsc::Sender<(Cmd, oneshot::Sender<Result<Box<dyn std::any::Any + Send>, String>>)>,
}

impl StateHandle {
    /// 提交一个命令到写者线程, 等待结果。
    pub async fn exec<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<R, String> + Send + 'static,
    ) -> Result<R, String> {
        let (tx, rx) = oneshot::channel();
        let cmd: Cmd = Box::new(move |conn| {
            f(conn).map(|v| Box::new(v) as Box<dyn std::any::Any + Send>)
        });
        self.tx
            .send((cmd, tx))
            .await
            .map_err(|_| "state writer 已关闭".to_string())?;
        let result = rx
            .await
            .map_err(|_| "state writer 响应通道已关闭".to_string())?;
        // 把 Box<dyn Any> 转回 R
        result.map(|any| *any.downcast::<R>().expect("type mismatch in StateHandle::exec"))
    }

    /// 提交一个返回 `()` 的命令 (不关心返回值)。
    pub async fn exec_void(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<(), String> + Send + 'static,
    ) -> Result<(), String> {
        self.exec(move |conn| f(conn)).await
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
    /// 启动写者线程, 使用给定的数据库路径。
    /// 会自动打开/创建数据库并执行迁移。
    /// 在迁移完成前阻塞, 返回后 DB 已就绪。
    pub fn start(db_path: PathBuf) -> Result<Self, String> {
        let (tx, mut rx) = mpsc::channel::<(Cmd, oneshot::Sender<Result<Box<dyn std::any::Any + Send>, String>>)>(128);
        let (ready_tx, ready_rx) = std_mpsc::channel::<Result<(), String>>();

        let join_handle = std::thread::Builder::new()
            .name("sebas-state-db".into())
            .spawn(move || {
                // 在专用线程中打开数据库
                let mut conn = match crate::sebas_state::db::open(&db_path) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(path = %db_path.display(), error = %e, "state writer 打开数据库失败");
                        let _ = ready_tx.send(Err(format!("打开数据库失败: {e}")));
                        return;
                    }
                };

                // 执行迁移
                if let Err(e) = crate::sebas_state::migration::run_migrations(&mut conn, &db_path) {
                    tracing::error!(path = %db_path.display(), error = %e, "state writer 迁移失败");
                    let _ = ready_tx.send(Err(format!("迁移失败: {e}")));
                    return;
                }

                tracing::info!(path = %db_path.display(), "state writer 就绪");
                let _ = ready_tx.send(Ok(()));

                // 命令循环: 串行处理
                while let Some((cmd, tx)) = rx.blocking_recv() {
                    let result = cmd(&mut conn);
                    // 如果接收端已关闭, 忽略
                    let _ = tx.send(result);
                }

                tracing::info!("state writer 已停止");
            })
            .map_err(|e| format!("创建 state writer 线程失败: {e}"))?;

        // 等待迁移完成
        ready_rx.recv()
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

impl Drop for StateWriter {
    fn drop(&mut self) {
        // 关闭通道, 通知写者线程退出
        // (但 mpsc 的 tx 被 StateHandle 持有, 所以需要等所有 handle 都 drop)
        // 这里我们显式关闭 tx
        // 由于 StateHandle 也持有 tx, 我们只关闭自己的 tx
        // 真正的清理在 StateHandle 全部 drop 后发生
    }
}

/// 重建 StateWriter, 但 handle 已被拿走, 所以 writer 需要新独立通道。
/// 一般用于测试, 生产环境只启动一次。
impl StateWriter {
    /// 测试用: 创建 writer 和 handle, 返回 writer 和独立的 handle。
    #[cfg(test)]
    pub fn start_test(db_path: PathBuf) -> Result<(Self, StateHandle), String> {
        let writer = Self::start(db_path)?;
        let handle = writer.handle.clone();
        Ok((writer, handle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sebas_state::db;
    use tempfile::tempdir;

    #[tokio::test]
    async fn writer_executes_commands() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("writer.db");
        let writer = StateWriter::start(path.clone()).unwrap();
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
        let writer = StateWriter::start(path.clone()).unwrap();
        let handle = writer.handle.clone();

        // 写数据
        handle
            .exec_void(move |conn| {
                conn.execute_batch("CREATE TABLE test (x INTEGER)")
                    .map_err(|e| e.to_string())?;
                conn.execute("INSERT INTO test (x) VALUES (42)", [])
                    .map_err(|e| e.to_string())?;
                Ok(())
            })
            .await
            .unwrap();

        // 读回来
        let val: i64 = handle
            .exec(move |conn| {
                conn.query_row("SELECT x FROM test", [], |row| row.get(0))
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap();

        assert_eq!(val, 42);
    }

    #[tokio::test]
    async fn writer_handles_errors_gracefully() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("error.db");
        let writer = StateWriter::start(path.clone()).unwrap();
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
    async fn writer_migration_creates_tables() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("migrate_writer.db");
        let _writer = StateWriter::start(path.clone()).unwrap();

        // 直接打开数据库验证迁移已执行
        let conn = db::open(&path).unwrap();
        let version = db::user_version(&conn).unwrap();
        assert_eq!(version, crate::sebas_state::migration::CURRENT_VERSION);

        // 验证表存在
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(count >= 5, "expected at least 5 tables, got {count}");
    }

    #[tokio::test]
    async fn writer_serializes_concurrent_commands() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        let dir = tempdir().unwrap();
        let path = dir.path().join("concurrent.db");
        let writer = StateWriter::start(path.clone()).unwrap();
        let handle = writer.handle.clone();

        // 建表
        handle
            .exec_void(move |conn| {
                conn.execute_batch("CREATE TABLE counter (id INTEGER PRIMARY KEY, val INTEGER)")
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap();

        // 并发 50 个递增写入, 验证写者线程串行化
        let counter = Arc::new(AtomicU64::new(0));
        let mut tasks = Vec::new();
        for i in 0..50u64 {
            let h = handle.clone();
            let c = counter.clone();
            tasks.push(tokio::spawn(async move {
                h.exec_void(move |conn| {
                    conn.execute(
                        "INSERT INTO counter (id, val) VALUES (?1, ?2)",
                        rusqlite::params![i as i64, i as i64],
                    )
                    .map_err(|e| e.to_string())?;
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .await
                .unwrap();
            }));
        }

        for t in tasks {
            t.await.unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), 50);

        // 验证所有 50 行都写入
        let count: i64 = handle
            .exec(move |conn| {
                conn.query_row("SELECT COUNT(*) FROM counter", [], |row| row.get(0))
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap();

        assert_eq!(count, 50);
    }
}