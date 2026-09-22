//! StateWriter 域接线（extract-sebas-db 2.3 / D4）。
//!
//! actor 本体已下沉 [`sebas_db::writer`]（专用线程 `sebas-state-db`、通道
//! 容量 128、启动 open+sync、就绪信号——语义原样）。本模块保留既有公开
//! 路径 `sebas_state::writer::StateWriter`，把**域注册表**（哪几张表）交给
//! actor——域 schema 事实留在域侧，actor 保持域无关。

use crate::sebas_state::repo::REGISTERED_TABLES;

/// 异步句柄（sebas-db 单写 actor 的原样再导出）。
pub use sebas_db::writer::StateHandle;

/// 专职写者（域接线形态）：`start(db_path)` 的既有形状不变，内部以根 crate
/// 的注册表驱动 sebas-db 的 actor。
pub struct StateWriter {
    inner: sebas_db::writer::StateWriter,
}

impl StateWriter {
    /// 启动写者线程, 使用给定的数据库路径。
    /// 会自动打开/创建数据库并同步 schema (sqlite-auto-schema-sync)。
    /// 在同步完成前阻塞, 返回后 DB 已就绪。
    pub fn start(db_path: std::path::PathBuf) -> Result<Self, String> {
        Ok(Self {
            inner: sebas_db::writer::StateWriter::start(db_path, REGISTERED_TABLES)?,
        })
    }

    /// 获取异步句柄。
    pub fn handle(&self) -> &StateHandle {
        self.inner.handle()
    }
}
