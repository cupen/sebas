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
//! # Schema 纪律 (retire-schema-reset)
//!
//! - **struct 即事实源**: 表结构由 `*Row` struct 声明
//!   (`#[derive(SchemaColumns, ActiveRecord)]`, 定义在 `sebas-models`),
//!   启动时与实际库结构对比，结构差异一律**原位保数据迁移**：缺列
//!   `ALTER TABLE ADD COLUMN`、声明改名 `ALTER TABLE RENAME COLUMN`、类型不符
//!   事务内覆盖式重建、多余列 `ALTER TABLE DROP COLUMN`（受限列走重建）、缺表
//!   按注册 DDL 建表。任何路径都不删除数据库文件。
//! - **加列规则**: 新列要么可空 (`Option<T>`), 要么带常量默认值
//!   (`#[column(default = "...")]`); 非空无默认的缺列既补不了也回填不了，
//!   按 fail-closed 拒启动（不再回退成删库）。
//! - **破坏性步骤先备份**: 重建/删列前 `VACUUM INTO '<db>.pre-sync'`；备份失败
//!   或迁移中途失败 → 事务回滚 + 拒启动，库字节不变。
//! - SQL 里 INSERT/UPDATE 一律显式列名, 禁止 `INSERT INTO t VALUES(...)`。
//! - 约束 (PRIMARY KEY/UNIQUE/REFERENCES) 与索引只表达在 `SETTINGS_TABLES`
//!   / `PROJECTS_TABLES`（single-state-dir 拆库注册表）的手写 DDL 里;
//!   struct 只描述列名/类型/默认值/可空性。

pub mod defaults_import;
pub mod engine;
pub mod repo;
pub mod writer;
