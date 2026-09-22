//! `sebas-db` — 持久层 runtime（extract-sebas-db）。
//!
//! 连接配方、schema 自描述与启动同步、单写 actor、泛型 `Record` trait 的
//! **唯一**共享提供者。第三处需要 SQLite 的组件从这里取连接与执行模型，
//! 而不是再手抄一份配方或长出第二套版本机制。
//!
//! # 域无关纪律
//!
//! 本 crate 的公开面**不得出现任何域表名、域行类型或角色实现**
//! （spec「The execution model carries no domain knowledge」）：表名、主键、
//! 列与行映射全部由使用方提供——领域 DDL 注册表留在根 crate，行 struct 与
//! 其 ActiveRecord impl 住在 `sebas-models`（以及按写入者归属的
//! `sebas-webui` / `sebas-router`）。依赖图上机械可核对：
//! `cargo tree -p sebas-db` 不得出现任何 sebas-* crate。
//!
//! # 模块
//!
//! - [`conn`]：连接配方（WAL + busy_timeout=5s + foreign_keys=ON）、两种
//!   事务入口、`user_version` 原语；
//! - [`schema`]：schema 原语（`SchemaColumn` / `TableSchema`）与启动同步
//!   算法（列级 diff、`ALTER TABLE ADD COLUMN`、自描述版本戳、不兼容重置）；
//! - [`record`]：泛型 `Record` trait 与标准 CRUD 的 SQL 构建器——运行时
//!   只认识 trait，不认识任何域类型；
//! - [`writer`]：单写 actor（`StateWriter` / `StateHandle`）与 `Record`
//!   类型化门面。
//!
//! # 事务行为
//!
//! 两种行为都提供、不替调用方选（design D5）：`sebas.db` 走单写线程 +
//! 默认 deferred 事务；`auth.db` 靠自己的 `Mutex` 串行并用
//! `TransactionBehavior::Immediate`——调用方各自的选择原样保留。

/// rusqlite 再导出：让 `#[derive(ActiveRecord)]` 生成的代码只需依赖
/// `sebas-db` 一个名字（生成路径统一走 `::sebas_db::rusqlite::…`）。
pub use rusqlite;

pub mod conn;
pub mod record;
pub mod schema;
pub mod writer;

/// 测试专用中性夹具（仅 `#[cfg(test)]` 编译，不进公开面）。
#[cfg(test)]
mod fixtures;
