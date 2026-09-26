//! StateWriter 域接线（extract-sebas-db 2.3 / D4；single-state-dir D5）。
//!
//! actor 本体在 [`sebas_db::writer`]（专用线程、通道容量 128、启动
//! open+sync、就绪信号——语义原样）。本模块把**域注册表**（哪几张表）交给
//! actor——域 schema 事实留在域侧，actor 保持域无关。core 的两库各走一次
//! open：[`StateWriter::start_settings`]（providers / model_aliases /
//! settings / agents）与 [`StateWriter::start_projects`]（projects / session_map），
//! 各自独立的 WAL / `busy_timeout` / 注册表与版本戳（design D5）。

use crate::sebas_state::repo::{PROJECTS_TABLES, SETTINGS_TABLES};

/// 异步句柄（sebas-db 单写 actor 的原样再导出）。
pub use sebas_db::writer::StateHandle;

/// 专职写者（域接线形态）：内部以根 crate 的注册表驱动 sebas-db 的 actor。
pub struct StateWriter {
    inner: sebas_db::writer::StateWriter,
}

impl StateWriter {
    /// 启动 **settings 库**写者线程（最常用形态的别名；`PersistedState` /
    /// CardConfig / defaults 导入都走它）。projects 库一律走
    /// [`StateWriter::start_projects`]——两库各开一次（single-state-dir D5）。
    /// 会自动打开/创建数据库并同步 schema (sqlite-auto-schema-sync)。
    /// 在同步完成前阻塞, 返回后 DB 已就绪。
    pub fn start(db_path: std::path::PathBuf) -> Result<Self, String> {
        Self::start_settings(db_path)
    }

    /// 启动 settings.db 写者线程（providers / model_aliases / settings /
    /// agents）。
    /// 会自动打开/创建数据库并同步 schema (retire-schema-reset：原位保数据迁移)。
    /// 在同步完成前阻塞, 返回后 DB 已就绪。
    pub fn start_settings(db_path: std::path::PathBuf) -> Result<Self, String> {
        Ok(Self {
            inner: sebas_db::writer::StateWriter::start(db_path, SETTINGS_TABLES)?,
        })
    }

    /// 启动 projects.db 写者线程（projects / session_map）。语义同上——
    /// 两库互不感知，一个库的迁移不影响另一个（各自的备份/事务落在本库）。
    pub fn start_projects(db_path: std::path::PathBuf) -> Result<Self, String> {
        Ok(Self {
            inner: sebas_db::writer::StateWriter::start(db_path, PROJECTS_TABLES)?,
        })
    }

    /// 获取异步句柄。
    pub fn handle(&self) -> &StateHandle {
        self.inner.handle()
    }
}
