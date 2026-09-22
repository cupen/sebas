//! SQLite 单写者状态库 (openspec/changes/add-state-store)。
//!
//! extract-sebas-db 之后本模块只保留**域接线**（design D2）：
//!
//! - `repo.rs` — 五表注册清单 (表名, DDL, 派生列) + PersistedState /
//!   CardConfig 域的存储侧胶水（这两类函数引用 dispatch / feishu 的角色
//!   类型，进不了 sebas-models）；
//! - `engine.rs` — StateStoreEngine trait 的 DB 实现（dispatch 端口）；
//! - `writer.rs` — StateWriter 域接线：把域注册表交给 sebas-db 的 actor；
//! - `defaults_import.rs` — legacy defaults.json 的一次性导入。
//!
//! # 下沉共享层（extract-sebas-db）
//!
//! - 连接配方 / 事务入口：`sebas_db::conn`（原 `db.rs` 已删除）；
//! - schema 原语与启动同步算法：`sebas_db::schema`（原 `migration.rs` 已
//!   删除——工作区内不允许第二份迁移机制）；
//! - 单写 actor 与类型化门面：`sebas_db::writer`；
//! - 行 struct 与对象风格 CRUD / 域查询：`sebas_models`。
//!
//! # Schema 纪律 (sqlite-auto-schema-sync)
//!
//! - **struct 即事实源**: 表结构由 `*Row` struct 声明
//!   (`#[derive(SchemaColumns, ActiveRecord)]`, 定义在 `sebas-models`),
//!   启动时与实际库结构对比: 缺列自动 `ALTER TABLE ADD COLUMN`,
//!   其余不兼容(类型不符/多余列/缺表/版本格式未知)重置重建。
//! - **加列规则**: 新列要么可空 (`Option<T>`), 要么带常量默认值
//!   (`#[column(default = "...")]`); 非空无默认的缺列无法原地补, 会触发重置。
//! - SQL 里 INSERT/UPDATE 一律显式列名, 禁止 `INSERT INTO t VALUES(...)`。
//! - 约束 (PRIMARY KEY/UNIQUE/REFERENCES) 与索引只表达在 `REGISTERED_TABLES`
//!   的手写 DDL 里; struct 只描述列名/类型/默认值/可空性。

pub mod defaults_import;
pub mod engine;
pub mod repo;
pub mod writer;
