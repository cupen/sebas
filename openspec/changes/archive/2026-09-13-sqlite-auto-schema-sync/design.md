# 设计：SQLite 自动 schema 同步

## Context

现状：`src/sebas_state/migration.rs` 维护 `MIGRATIONS` 函数数组（已到 v2），打开 DB 时按 `PRAGMA user_version` 逐级推进；迁移前做快照备份；库版本高于二进制时拒启（TooNew）。表结构由各迁移函数手写 DDL，`repo.rs` 的 `*Row` struct 与 DDL 靠人眼保持一致。工作区成员在仓库顶层（`sebas-im`、`sebas-dispatch` 等），状态存储在主 crate `src/sebas_state/`。五张表：providers / model_aliases / settings / projects / session_map。见 proposal.md — Why。

Rust 无运行时反射，"struct 即 schema"需要一个编译期提取列元数据的机制——proc-macro derive。

## Goals / Non-Goals

**Goals:**
- model struct 挂 `#[derive(SchemaColumns)]` 后成为该表列清单的单一事实源（列名、SQLite 亲和类型、常量默认值、可空性）。
- 启动时 diff 同步：缺列 `ALTER TABLE ADD COLUMN`，其余不兼容（类型不符 / 多余列 / 缺表 / 版本格式未知）→ 删库重建空 schema。
- 版本自描述落库（`version_format` + `version`），仅作诊断，不作重置触发。
- SQLite 原地加列做不到的（PRIMARY KEY / UNIQUE 列、无常量默认值的 NOT NULL 列）在**编译期**拒绝。

**Non-Goals:**
- 不做重置前备份；不做 legacy 入轨迁移；不做 drop/rename 列的原地改（走重置）。
- 不动单写者、WAL、打开路径（`db.rs` 的 open 语义保持）。
- 正式发布后的兼容/迁移策略（另立变更重审）。

## Decisions

### D1. 新 workspace 成员 crate `sebas-schema-derive`（proc-macro）

derive 宏挂 `#[derive(SchemaColumns)]`，为 struct 生成 `fn schema_columns() -> &'static [SchemaColumn]`，`SchemaColumn { name, affinity, default: Option<&'static str>, not_null: bool }`。选 proc-macro 而非 `inventory`/运行时解析：编译期完成、零运行时开销、非法字段能在 `cargo build` 就报错。

字段映射：`String`→TEXT、`i64`/`i32`→INTEGER、`f64`→REAL、`bool`→INTEGER、`Vec<u8>`→BLOB、`Option<T>`→可空。带 `#[column(...)]` 辅助属性覆盖列名/默认值。

**编译期拒绝**：字段标 `UNIQUE`/`PRIMARY KEY`（SQLite 无法 ADD COLUMN 加这两类）、`NOT NULL` 且无常量默认值、类型不在映射内。错误信息指向字段名。

### D2. 表注册：`(表名, CREATE TABLE DDL, 派生列清单)` 三元组

`repo.rs` 每张表已有 Row struct；注册点把三者绑在一起（宏产物 + 手写 DDL 首建语句）。DDL 只在"建新库/重置"时使用；日常同步只依赖派生列清单 vs `PRAGMA table_info`。**对比按 SQLite 亲和类型归一**（declared type → affinity 后比较），避免 `VARCHAR(255)` vs `TEXT` 这类误报。

### D3. 同步算法（migration.rs 重写为 sync）

打开 DB 后：
1. 读 `schema_meta` 键值（专用小表，同步层自建自管，不属于五张领域表、不参与 diff）。
2. `version_format` 缺失或非 `date` → **重置**（覆盖旧迁移链产的库：它们只有 `user_version`、无 meta）。
3. 逐注册表：`PRAGMA table_info` 对比派生列——缺列→`ALTER TABLE ADD COLUMN`（带常量默认）；类型不符或有派生清单外的多余列或整表缺失→**重置**。
4. 全部通过→写/更新 meta 键；`version` 值差异本身不触发任何动作，仅随写 meta 更新。
5. 重置 = 删 DB 文件 + `-wal`/`-shm`，按注册 DDL 重建空 schema，`WARN` 日志写明触发原因（哪张表、什么不匹配）。

**与损坏边界的实现衔接**：先 `open()`，打不开→按既有损坏路径拒启（不进入同步、绝不删文件）；同步逻辑只对成功打开的库运行。这保证「重置只针对 schema 不兼容」在代码结构上成立。

### D4. 版本常量

`pub const SCHEMA_VERSION: &str = "20260913";`（本变更落地日的日期），改 schema 时 bump。落库键 `version_format="date"`、`version=SCHEMA_VERSION`。

### D5. 迁移链退役 = 直接删除

`MIGRATIONS` 数组、逐级推进、TooNew 拒启、迁移前备份与备份保留全部删除；`db.rs` 的 `user_version`/`set_user_version` 辅助随之清理（`user_version` PRAGMA 不再承载版本语义）。一次性对齐：任务里先让五表 Row struct 与现有 v2 库结构一一对应，升级后的首次打开走"缺 meta → 重置"，属预期行为（开发库无存量数据），日志如实记录。

## Risks / Trade-offs

- [proc-macro 编译报错信息难懂] → 宏内对每种非法形态给独立 span 指向字段；宏 crate 自带单测覆盖每种拒绝路径。
- [误判重置丢开发数据] → 对比按亲和类型归一；重置日志列明具体不匹配项；开发库可弃是提案前提。
- [ALTER ADD COLUMN 后旧行新列取默认值与 Rust 侧 Option 语义不一致] → 加列场景限常量默认 + 可空字段，映射规则在宏里强校验。
- [未来新表接入忘注册] → 注册表清单集中在 sync 入口，`openspec doctor` 不覆盖此点，靠 code review；风险接受。

## Migration Plan

单提交落地，无灰度。旧开发库首次打开被重置（预期，日志可见）。回滚 = revert 提交（旧二进制读被重置的库会因缺 `user_version` 语义走其 fresh/迁移路径——v2 迁移链对 version 0 库会重建 schema，可自愈）。

## Open Questions

（无——语义边界已在 proposal/spec 定死：损坏拒启、不兼容重置、版本仅诊断。）
