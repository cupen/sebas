//! SQLite 单写者状态库 (openspec/changes/add-state-store)。
//!
//! - `db.rs` — 连接打开、WAL mode、user_version 读写
//! - `migration.rs` — 自动迁移链: 事务性 DDL、VACUUM INTO 备份、高版本拒绝
//! - `writer.rs` — 专职写者 actor: 专用线程 + mpsc + oneshot
//! - `engine.rs` — StateStoreEngine trait 的 DB 实现 (阶段 3)
//! - `repo.rs` — 领域仓储 (阶段 2)
//!
//! # 迁移纪律
//!
//! - **只加不改**: 常规迁移只允许加表、加可空/带默认值列、加索引。
//!   改名/删列/收紧约束必须走双阶段(先加新列双写 → 下个兼容窗口再删)。
//! - SQL 里 INSERT/UPDATE 一律显式列名, 禁止 `INSERT INTO t VALUES(...)`。
//! - 迁移函数一旦进入 `MIGRATIONS` 数组就不可变, 不许修改已归档的迁移。

pub mod db;
pub mod engine;
pub mod migration;
pub mod repo;
pub mod writer;

// 阶段 2/3 引入:
// pub mod repo;
// pub mod engine;