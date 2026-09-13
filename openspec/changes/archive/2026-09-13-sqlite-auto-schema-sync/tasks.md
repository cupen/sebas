# Tasks: sqlite-auto-schema-sync

## 1. Derive 宏 crate

- [x] 1.1 新建 workspace 成员 `sebas-schema-derive`（proc-macro crate，加入根 `Cargo.toml` members），定义 `SchemaColumn` 元数据类型与 `#[derive(SchemaColumns)]`：解析字段生成 `schema_columns()`，含类型映射（String→TEXT、i64/i32→INTEGER、f64→REAL、bool→INTEGER、Vec<u8>→BLOB、Option→可空）与 `#[column(...)]` 覆盖属性
- [x] 1.2 编译期拒绝路径：UNIQUE/PRIMARY KEY 字段、NOT NULL 无常量默认、未支持类型——各自带字段定位的清晰报错；宏 crate 单测覆盖每条拒绝路径与正常生成

## 2. 五表接入与对齐

- [x] 2.1 `repo.rs` 五张表的 Row struct 挂 derive，逆向对齐现有 v2 schema（列名/亲和类型/默认值/可空一一对应，对不上的以现有 DDL 为准改 struct 或补 `#[column]`）；建立 `(表名, CREATE TABLE DDL, 派生列)` 注册清单

## 3. 启动同步与重置

- [x] 3.1 `migration.rs` 重写为 sync：`schema_meta` 小表自建自管；先 open（失败走既有损坏拒启路径，不进同步）；`version_format` 缺失/未知 → 重置
- [x] 3.2 逐表 diff：缺列 `ALTER TABLE ADD COLUMN`（常量默认）、亲和类型归一比较、类型不符/多余列/缺表 → 删库（含 `-wal`/`-shm`）按注册 DDL 重建 + WARN 日志写明触发点；通过后写 `version_format=date` 与 `version=SCHEMA_VERSION`
- [x] 3.3 删除迁移链：`MIGRATIONS` 数组、逐级推进、TooNew 拒启、迁移前备份与保留逻辑；清理 `db.rs` 的 `user_version` 辅助；`SCHEMA_VERSION` 日期常量落地

## 4. 验证

- [x] 4.1 状态模块单测：fresh 建库打版本键、缺列原地加列（旧行可读）、多余列/类型不符/缺表各自触发重置、旧 `user_version` 库（无 meta）首开重置、版本值不同但结构一致不重置、损坏库仍拒启不删文件
- [x] 4.2 全工作区 `cargo build` + `cargo test` 绿；沙箱起 core 验证 `/api/summary` 与会话往返正常（schema 同步不破坏既有功能）
