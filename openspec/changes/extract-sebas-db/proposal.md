# Proposal: extract-sebas-db

## Why

持久层的 runtime 完全长在根 crate 里，于是**只有根能用**：`src/sebas_state/{db,migration,writer}.rs` 承载连接配方（WAL / `busy_timeout=5s` / `foreign_keys=ON`）、schema 自描述与启动同步（`schema_meta` 日期版本 + 列级 diff）、以及单写线程 actor（`StateWriter` / `StateHandle`，命令是 `Box<dyn FnOnce(&mut Connection)>`，本就与域无关）。而 `sebas-webui` 够不着它们，只能在自己的 `user_store.rs` 里**手抄同一套配方**——模块注释直言「连接配方复用 `sebas_state/db.rs` 的既有组合」——并自建第二套版本机制（`PRAGMA user_version` + `CREATE TABLE IF NOT EXISTS`）。`sebas-schema-derive` 更硬：它生成代码引用 `crate::sebas_state::migration::SchemaColumn`，**物理上只能在根 crate 内使用**，所以任何想用自动迁移的 crate 都无处安放模型。

规模不大但症状典型：全 workspace 只有 2 个 SQLite 文件、只有 1 个文件（`user_store.rs`，约 150 行 SQLite 相关）在 `sebas_state` 之外写 SQL，而它 100% 是复制。这是「抽一层 runtime，让第三处不必再抄」的最小成本时机。

同时，数据访问的**形状**是错的：provider 以 `config TEXT (JSON blob)` 存储，读写靠 `Item = Map<String, Value>` 的无类型映射与约定式键名；repo 层是自由函数（`load_projects` / `save_projects`），表与类型之间没有结构化的对应。本 change 顺势把持久层的访问模式定为 **ActiveRecord**：一个表对应一个 struct、一行对应一个实例、对象风格的 CRUD——由 derive 生成，而不是每个表手写一遍。

## What Changes

- 新增叶子 crate `sebas-db`：连接配方（open / readonly / pragma 组合）、schema 原语（`SchemaColumn` / `TableSchema`）、启动同步算法（列级 diff、`ALTER TABLE ADD COLUMN`、自描述版本戳）、单写 actor（`StateWriter` / `StateHandle`），以及**泛型 `Record` trait**（表名、主键、列参数、行反解——运行时认识 trait，不认识任何域类型）。
- **ActiveRecord 模式**：`sebas-schema-derive` 扩展出 `#[derive(ActiveRecord)]`——在 struct 定义处生成固有 impl（`save` / `find` / `delete` / `all`，以及按 `schema_columns()` 生成的 upsert SQL），一个表一个 struct、一行一个实例。标准 CRUD 零手写 SQL；非标准查询（聚合、按非主键条件）仍允许手写 SQL。
- 新增 crate `sebas-models`：core 各表的 ActiveRecord struct 住在那里（依赖 `sebas-db` + `sebas-domain` + serde），core 与 webui 共用；`User`（auth.db）留在 `sebas-webui`、`UsageRecord`（usage.db）留在 `sebas-router`——**模式统一，归属按写入者**。
- `sebas-schema-derive` 的生成路径从 `crate::sebas_state::migration::SchemaColumn` 改为 `::sebas_db::schema::SchemaColumn`，使 derive 首次可在根 crate 之外使用。
- `sebas-webui::user_store` 的 SQLite 部分改为复用 `sebas-db`：连接配方、单写序列化与 `users` 表的 ActiveRecord CRUD 来自共享层，删除手抄副本。
- 根 crate 的 `sebas_state` 保留**域接线**（两库的注册表引用、`DbStateEngine` 对 dispatch 端口 trait 的实现），行 struct 与 CRUD 迁入 `sebas-models`。
- **行为零变化**：不改变任何 DB 的 pragma 效果、迁移判定、锁行为或 schema 语义（表形状重塑属 `single-state-dir`）。`user_store` 继续用 `TransactionBehavior::Immediate` 与 `user_version`。

## Capabilities

### New Capabilities

- `persistence-runtime`: 持久层 runtime 与访问模式的架构契约——连接配方、schema 自描述与启动同步、单写序列化由唯一共享 crate 提供；**一个表对应一个 struct（derive 生成对象风格 CRUD），一行即一个实例**；任何使用 SQLite 的组件复用该层而非自建配方、第二套版本机制或逐表手写 SQL；schema 模型可在任意 crate 中声明。

### Modified Capabilities

（无。本 change 不改变 `state-store` 任何既有要求的行为，只改变这些能力**住在哪个 crate**、以何种模式访问。迁移词表扩张与版本语义统一**已推迟**——无发布版，没有需要保护的数据，见 `quarantine-database-reset` 的 Non-goals。）

## Impact

- **新增**：`sebas-db`（叶子：rusqlite / serde / thiserror）、`sebas-models`（依赖 `sebas-db` / `sebas-domain` / serde）。
- **改动**：`sebas-schema-derive`（`META_PATH` + ActiveRecord 生成）；根 crate（runtime 下沉、行 struct 迁出、engine 接线保留）；`sebas-webui/src/user_store.rs`（复用配方与 ActiveRecord）；相关 `Cargo.toml`。
- **不变**：两个 DB 的 pragma 效果、schema 形状（providers 仍为 config JSON 列——重塑属 `single-state-dir`）、迁移判定、事务隔离行为、所有对外行为。验收 = 既有 `tests/state_persistence_test.rs` / `state_subscription_test.rs` / `user_store` 测试全绿 + `invoke testsuite-e2e` / `testsuite-acceptance` 全绿。
- **为后续解锁**：`single-state-dir`（分层库 + providers 扁平化重塑）、`persist-session-map`（`entry.save()` 即落盘）、`persist-router-usage`（`UsageRecord` 复用同一模式）、`migrate-project-registry`（规范记录即 ActiveRecord struct）。

## Non-goals

- **不改变「不兼容即重置」语义到「保数据」的程度**——本 change 不改它，只由 `quarantine-database-reset` 把重置从「删除」改为「隔离」；迁移词表的扩张已推迟。
- **不统一两个 DB 的版本机制**（`schema_meta` 日期戳 vs `PRAGMA user_version`）——已推迟。
- **不改变事务行为**（`Immediate` vs 默认 deferred）——共享层提供两种，调用方各自的选择原样保留。
- **不重塑 providers 的 JSON blob 形状**——那是 `single-state-dir` 的表重塑；本 change 的 `ProviderRow` 先按现状搬运。
- **不引入连接池或多连接并发**——单写线程 actor 是既有契约，不在此改变。
- **不动 JSON/JSONL 持久化**（`archive.json` / 节点日志）——ActiveRecord 适用于 SQLite 存储；它们若日后入库，再成为 record。
- **不做跨库或跨记录的自动事务**——多表原子性经 store 上的闭包显式表达（既有 `Cmd` 闭包机制）。
