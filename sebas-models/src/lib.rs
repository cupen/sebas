//! `sebas-models` — core 各表的 ActiveRecord struct（extract-sebas-db D2）。
//!
//! **一个表对应一个 struct、一行对应一个实例**：表与列的映射声明在 struct
//! 上（`#[derive(SchemaColumns, ActiveRecord)]`），标准 CRUD（save / find /
//! all / delete）由 derive 在 struct 定义处生成——一行经对象风格调用落盘，
//! 不再有逐表手写 SQL 或自由函数仓储。
//!
//! # 归属边界
//!
//! 本 crate 收 core 状态库（`sebas.db`）的行 struct；`User`（auth.db）留在
//! `sebas-webui`、`UsageRecord`（usage.db）留在 `sebas-router`——**模式统一
//! （同一个 trait + derive），归属按写入者**（design D2）。`sebas-node` 不
//! 依赖本 crate（节点依赖图里依然没有 SQLite）。
//!
//! # 模块
//!
//! - [`project`]：`ProjectRow`（projects 表）+ 项目域查询；
//! - [`session_map`]：`SessionMapRow`（session_map 表，复合主键）+ 其域查询；
//! - [`provider`]：`ProviderRow` / `ModelAliasRow` + 按非键列的域查询；
//! - [`setting`]：`SettingRow`（settings 表，KV）；
//! - [`runtime_state`]：runtime 状态（mode + default_selection）与 legacy
//!   defaults 一次性导入的域查询。
//!
//! # 边界
//!
//! 域 schema 事实（哪张表、什么约束的手写 DDL）**不在**本 crate——注册表
//! 留在根 crate；本 crate 只有行 struct 与返回 struct 实例的域查询。非标准
//! 查询（排序、聚合、按非键列条件）保留手写 SQL，但一律返回 struct 实例或
//! 标量，不再有 `Map<String, Value>` 式的无类型载体。

pub mod project;
pub mod provider;
pub mod runtime_state;
pub mod session_map;
pub mod setting;
