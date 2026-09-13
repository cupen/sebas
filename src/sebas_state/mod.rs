//! SQLite 单写者状态库 (openspec/changes/add-state-store)。
//!
//! - `db.rs` — 连接打开、WAL mode、busy_timeout
//! - `migration.rs` — 启动 schema 同步: 派生列 diff、缺列 ALTER、不兼容重置
//!   (sqlite-auto-schema-sync)
//! - `writer.rs` — 专职写者 actor: 专用线程 + mpsc + oneshot
//! - `engine.rs` — StateStoreEngine trait 的 DB 实现 (阶段 3)
//! - `repo.rs` — 领域仓储 (阶段 2) + 五表注册清单 (表名, DDL, 派生列)
//!
//! # Schema 纪律 (sqlite-auto-schema-sync)
//!
//! - **struct 即事实源**: 表结构由 `*Row` struct 声明
//!   (`#[derive(SchemaColumns)]` 编译期提取列), 启动时与实际库结构对比:
//!   缺列自动 `ALTER TABLE ADD COLUMN`, 其余不兼容(类型不符/多余列/缺表/
//!   版本格式未知)重置重建。
//! - **加列规则**: 新列要么可空 (`Option<T>`), 要么带常量默认值
//!   (`#[column(default = "...")]`); 非空无默认的缺列无法原地补, 会触发重置。
//! - SQL 里 INSERT/UPDATE 一律显式列名, 禁止 `INSERT INTO t VALUES(...)`。
//! - 约束 (PRIMARY KEY/UNIQUE/REFERENCES) 与索引只表达在 `REGISTERED_TABLES`
//!   的手写 DDL 里; struct 只描述列名/类型/默认值/可空性。

pub mod db;
pub mod defaults_import;
pub mod engine;
pub mod migration;
pub mod repo;
pub mod writer;