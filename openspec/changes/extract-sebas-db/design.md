## Context

动机见 `proposal.md` — Why。设计相关的当前状态：

1. **runtime 已基本域无关**：`StateWriter` 的命令类型是 `Box<dyn FnOnce(&mut Connection) -> Result<Box<dyn Any + Send>, String>>`（`writer.rs:25`），对表结构一无所知；`migration.rs` 的 `SchemaColumn` / `TableSchema` / diff 算法也只吃元数据。**唯一咬住根 crate 的地方是 derive 的生成路径**（`sebas-schema-derive/src/lib.rs:36-38` 明说只能在根内用）。该 API 的形状本就是「一次 open = 一个库文件 + 一张注册表」，因此**天然支持分层**：`single-state-dir` 会让 core 开两个库（`settings.db` / `projects.db`），本 change 不实施该分层，但必须保证 open、注册表与版本戳都是**每库独立**的（见 D2 的分界）。
2. **域 schema 事实在 `repo.rs`**：`REGISTERED_TABLES`（`repo.rs:556-616`）是 5 张表的手写 DDL（含 `PRIMARY KEY` / `UNIQUE` / `REFERENCES` / 索引），derive 只提供列元数据。
3. **两个 DB 的隔离行为不同且有理由**：`sebas.db` 走单写线程 + 默认 deferred 事务；`auth.db` 走 `Mutex<Connection>` + `TransactionBehavior::Immediate`（`user_store.rs:396,448,505,525,545`）。
4. **两个 DB 的连接配方当前一致**：都是 WAL + `busy_timeout=5000ms` + `foreign_keys=ON`（`db.rs:18-34` 与 `user_store.rs:278-311`），后者注释自认是复制。
5. **`state-store` spec 已钉住的行为**（不得改变）：单写序列化、「缺失列原位 ADD COLUMN」、「不兼容结构重置」、「损坏不重置」、「版本值本身从不触发重置」。

## Goals / Non-Goals

**Goals:**

- 让「第三处需要 SQLite」时**不必再抄一遍**——这是本 change 唯一的收益主张。
- 让 schema 模型**可以在任意 crate 声明**，为后续持久层演进解开物理限制。
- 全程零行为变化：pragma 效果、迁移判定、锁行为、schema 语义一字不改。

**Non-Goals:**

- 不改进迁移能力、不统一版本机制、不动「不兼容即重置」的判定——迁移词表扩张与版本语义统一**已推迟**（无发布版），把重置由删除改为隔离属 `quarantine-database-reset`。
- 不重塑表形状（providers 的 JSON blob 列、settings 的 KV 形状原样搬运）——那是 `single-state-dir` 的表重塑。
- 不让 `sebas-db` 知道任何域概念；不为跨记录做自动事务。

## Decisions

### D1 独立 crate `sebas-db`，不并进 `sebas-domain`

- **理由（关键）**：`sebas-node` 要依赖 `sebas-domain`（change 1），但节点**没有任何 SQLite**。若把 rusqlite 拖进 domain，节点产物就会凭空多出 SQLite 与 bundled C 库——直接违背 `execution-node`「节点产物只含运行自己会话所需」的既有要求。
- **备选否决**：并进 `sebas-domain`（少一个 crate，但污染节点依赖图）；并进 `sebas-schema-derive`（proc-macro crate 不能导出普通类型，物理不可能）。

### D2 下沉 runtime，留下域接线——行 struct 迁入 `sebas-models`

| 下沉 `sebas-db` | 新 crate `sebas-models` | 留在根 `sebas_state` |
|---|---|---|
| 连接配方（open / readonly / pragma） | core 各表的 ActiveRecord struct（`ProjectRow` / `SessionMapRow` / `ProviderRow` / `ModelAliasRow` / `SettingRow`） | 两库的注册表引用与 DDL |
| `SchemaColumn` / `TableSchema` / `type_affinity` | 它们 `#[derive(ActiveRecord)]` 生成的 CRUD | `DbStateEngine` 与 `notify_change` 接线 |
| `sync_conn` / `SyncOutcome` / `SyncFail` / `reset_and_rebuild` | 域级查询函数（非标准 CRUD 的部分，返回 struct 实例） | dispatch 端口 trait（`StateStoreEngine`）的实现 |
| `StateWriter` / `StateHandle` / `Cmd` / `CmdOutcome` + 泛型 `Record` trait | — | — |
| `unix_now` 一类原语（若 change 1 未收） | — | — |

**分界理由**：注册表与 DDL 是**域 schema 事实**（哪张表、什么约束），runtime 是**怎么连、怎么比、怎么串行**。前者留在域侧，才让 `sebas-db` 保持域无关（spec「The execution model carries no domain knowledge」）。

**行 struct 为何进 `sebas-models` 而非 `sebas-db` 或根 crate**：ActiveRecord 的固有方法（`row.save(&mut conn)`）要求 impl 与 struct 同 crate，而 impl 依赖 rusqlite——放进 `sebas-db` 会让 runtime 认识域表（违反域无关）；留在根 crate 则 webui 永远够不着（根是依赖死端）。`sebas-models` 的消费方是 core 与 webui（webui 本就依赖 rusqlite——auth.db）；**`sebas-node` 不依赖它**，节点依赖图里依然没有 SQLite。`User`（auth.db）与 `UsageRecord`（usage.db）**不进** `sebas-models`：模式靠共享的 trait + derive 统一，归属按写入者——它们分别留在 `sebas-webui` 与 `sebas-router`。

### D3 derive 生成路径硬编码为 `::sebas_db::schema::SchemaColumn`，并扩展出 `#[derive(ActiveRecord)]`

- `sebas-schema-derive` **不依赖** `sebas-db`——它只发射路径字符串，依赖由使用方提供。
- 在既有 `SchemaColumns`（列元数据）之上扩展 `#[derive(ActiveRecord)]`：从 struct 声明生成固有 impl——`save(&mut Connection)`（按主键 upsert，SQL 由 `schema_columns()` 拼出）、`find(conn, pk)`、`all(conn)`、`delete(conn, pk)`，以及 `Record` trait 的实现（表名、主键参数、`to_params` / `from_row`）。**固有能力**：impl 生成在 struct 定义处，因此 `row.save(...)` 这种对象风格调用不需要 trait 转发，也不要求 struct 搬进 `sebas-db`。
- 单列主键才支持生成的 `find` / `delete`；复合主键（`session_map` 的 `(chat_id, thread_id)`）生成 `find_by` / `delete_by`（按全部键列）。不支持的场景编译期报错。
- **被否备选**：加 `#[schema(crate = "...")]` 属性让调用方指定根路径。更灵活，但今天只有一个落点，多一个配置面的收益为零。**触发条件**：出现第二个持久层 crate，或需要把模型 reroot 到非 `sebas-db` 的路径时再加。

### D3b ActiveRecord 的边界：trait 在 runtime、impl 在定义处、异步端口不变

- **`sebas-db` 只认识 trait**：泛型 `Record` trait（表名、主键、`to_params` / `from_row`）让 actor 能提供类型化门面（`handle.save(&r)` / `handle.find::<R>(pk)`），而不让 runtime 知道任何域类型——spec 的域无关要求逐字保留。
- **异步调用方走两跳**：`DbStateEngine`（dispatch 端口 trait 的实现）改为在 `handle.exec` 闭包里调用生成的方法——dispatch 的 `StateStoreEngine` 端口与 `MemoryEngine` 测试替身**原样保留**，ActiveRecord 是存储侧的实现模式，不是跨进程 API 的变化。
- **多表原子性 = store 上的闭包**：既有 `Cmd` 闭包机制就是 unit-of-work——`handle.exec(|conn| { a.save(conn)?; b.save(conn)?; ... })` 一个事务内完成。不做跨记录的自动事务。
- **非标准查询不消失**：聚合（如「启用的 root 数」）与按非键列条件（如「某 provider 的别名」）保留手写 SQL，但**必须返回 struct 实例**（spec 场景已钉）。
- **无类型 JSON 载体出局**：`Item = Map<String, Value>` 作为持久化契约的地位由本 change 终结——provider 的 JSON blob 列先按现状搬运（重塑属 `single-state-dir`），但读写一律经 `ProviderRow`。

### D4 actor 原样搬迁，API 不变

`StateWriter::start(db_path) -> StateHandle` 的形状与语义（专用 OS 线程 `sebas-state-db`、通道容量 128、线程内 open+sync、就绪信号）整体搬入，不重命名、不改容量、不改错误语义。搬动即验证：既有 `tests/state_subscription_test.rs` 与 `state_persistence_test.rs` 不改一行而全绿。

### D5 两种事务行为都提供，**不替调用方改选**

`auth.db` 用 `Immediate` 不是随意：它靠 `Mutex` 而非单写线程串行，`Immediate` 避免读-升级-写之间的锁竞争窗口。共享层因此暴露两种事务入口（或让调用方直接拿 `&mut Connection` 自行 `transaction_with_behavior`），**本 change 不改任何调用方的选择**。零变化基线的硬要求。

### D6 `user_store` 只复用「配方」，版本机制保留

`user_store` 改为从 `sebas-db` 取连接与 pragma，但**保留** `CREATE TABLE IF NOT EXISTS` + `PRAGMA user_version` + 未来版本拒绝（`IncompatibleVersion`）。理由：统一版本机制会改变 `auth.db` 的兼容性行为（今天它拒绝未来版本；`sebas.db` 忽略版本值），这是 spec 级变更，**已推迟**（无发布版，且它要解决的兼容问题今天不存在——见 `quarantine-database-reset` 的 Non-goals）。**这一条是本 change 最重要的自我约束**——顺手统一会让本 change 的「零行为变化」验收失效，而收益为零。

### D7 不做连接池、不做多连接

`state-store` spec 已钉「All mutations SHALL be applied ... serialized one at a time」。共享层提供单写 actor 与单连接两种形态，不引入池。

## Risks / Trade-offs

- **[提取时无意改动 pragma 值或顺序]** → 加断言测试：打开连接后读回 `journal_mode == "wal"`、`busy_timeout == 5000`、`foreign_keys == 1`，对两个 DB 各跑一次。pragma 是可直接观测的，不靠代码比对。
- **[derive 生成 SQL 的正确性]** → 生成的 upsert / find / delete 用既有 repo 函数的输出做黄金样本比对（同表同行的 SQL 与行反解逐字一致）；每张表一个往返测试（save → find → 字段全等）。
- **[`Record` trait 泄漏域概念]**（有人把域类型塞进 trait 或默认方法）→ spec 要求公开面不出现域表名/行类型；用 1.x 的机械断言扫描。
- **[迁移 `user_store` 时顺手改了事务行为]** → 保留其 `Immediate` 调用并让既有并发/约束测试（`user_store` 测试区与 `webui` 的 auth 用例）不改一行通过。
- **[derive 路径改造打破既有 5 个模型]** → 编译期即暴露；额外用一个「同一 struct 在根内外的列集一致」测试钉住列元数据不变。
- **[`sebas.db` 的迁移逻辑迁移后判定变化]** → 用「既有 DB 文件在改造前后启动得到同一 `SyncOutcome`」作为回归：`migration.rs` 既有 19 个测试直接作为闸门。
- **[impl-in-crate 约束被误解，有人试图在 `sebas-db` 里给域类型写 impl]** → 编译器直接拒绝（孤儿规则）；design 已把「模式统一靠 trait + derive、归属按写入者」写死。

## Migration Plan

无数据迁移（schema 与 pragma 不变，DB 文件照旧可读）。顺序：

1. 建 `sebas-db` 骨架并接进 workspace，确认叶子属性（无 rusqlite 之外的域依赖）。
2. 搬连接配方 + schema 原语 + 同步算法 + actor，附 pragma 断言测试；根 crate 改为引用（暂不删旧文件，先并存编译）。
3. 改 `sebas-schema-derive` 的生成路径，扩展 `#[derive(ActiveRecord)]`（含 `Record` trait 实现与固有 CRUD 生成）。
4. 建 `sebas-models`，把 5 个行 struct 迁入并挂 derive；非标准查询改为返回 struct 实例的域查询函数。
5. 删根 crate 的旧实现（repo 自由函数并入 `sebas-models` 的查询函数），`DbStateEngine` 改经生成的方法；跑 `tests/state_persistence_test.rs` / `state_subscription_test.rs`（应一行不改而全绿）。
6. 迁 `sebas-webui::user_store` 到共享配方 + `User` 的 ActiveRecord CRUD，保留其事务与版本行为。
7. 全量回归：`invoke testsuite-e2e` + `invoke testsuite-acceptance`。

**回滚**：分步提交，每步可 revert。无持久化状态需要回滚。若第 6 步发现 `user_store` 与共享配方的差异无法在不改行为的前提下弥合，**停在第 5 步并把差异记入本文件**——共享 runtime 的主要收益（第三处不必再抄）由第 2-5 步已经拿到。

## Open Questions

- JSON / JSONL 文件持久化（`state.json` / `providers.json` / `archive.json`）是否最终也归入 `sebas-db`：今天它们散在 4 个 crate 里各自 tmp+rename。它们不是 SQLite runtime，本 change 不收；是否收归是一条独立的架构问题，可在后续 change 里评估。不影响本 change 的 spec、做法与任务分解。
