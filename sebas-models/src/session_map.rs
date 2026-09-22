//! `SessionMapRow` — session_map 表的 ActiveRecord struct（复合主键）+ 域查询。
//!
//! struct 即表结构事实源；复合主键 `PRIMARY KEY (chat_id, thread_id)` 只在
//! 根 crate 注册表的 DDL 里表达，这里用 `#[active_record(pk = …)]` 声明
//! CRUD 的定位键（生成 `find_by` / `delete_by`）。

use sebas_db::rusqlite::Connection;
use sebas_schema_derive::{ActiveRecord, SchemaColumns};

/// session_map 行 (chat_id, thread_id, session_id, last_active_unix, project_dir)。
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "session_map")]
#[active_record(pk = "chat_id")]
#[active_record(pk = "thread_id")]
pub struct SessionMapRow {
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub session_id: String,
    pub last_active_unix: i64,
    pub project_dir: Option<String>,
}

/// 加载会话映射 (用于恢复；无排序语义，与既有行为一致)。
pub fn load_session_map(conn: &mut Connection) -> Result<Vec<SessionMapRow>, String> {
    SessionMapRow::all(conn).map_err(|e| format!("查询 session_map 失败: {e}"))
}

/// 保存会话映射 (全量替换, 单事务)。
pub fn save_session_map(
    conn: &mut Connection,
    entries: &[SessionMapRow],
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("保存 session_map 事务开始失败: {e}"))?;

    tx.execute("DELETE FROM session_map", [])
        .map_err(|e| format!("清空 session_map 表失败: {e}"))?;

    for entry in entries {
        entry
            .save(&tx)
            .map_err(|e| format!("写入 session_map 失败: {e}"))?;
    }

    tx.commit()
        .map_err(|e| format!("保存 session_map 事务提交失败: {e}"))?;
    Ok(())
}
