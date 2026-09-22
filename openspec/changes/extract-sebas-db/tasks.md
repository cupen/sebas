## 1. 建 sebas-db 骨架

- [ ] 1.1 新建 `sebas-db` 目录与 `Cargo.toml`（依赖仅 rusqlite / serde / thiserror），加入 workspace members 与根 crate 依赖；验证：`cargo build -p sebas-db` 通过
- [ ] 1.2 写 `src/lib.rs` 模块骨架（`conn` / `schema` / `record` / `writer`）与 crate 文档，声明「domain-agnostic」的公开面纪律（不得出现域表名/行类型；`record` 模块只含泛型 `Record` trait）；验证：`cargo doc -p sebas-db` 无警告，且 `cargo tree -p sebas-db` 不含任何 sebas-* crate
- [ ] 1.3 加机械断言测试（根 crate 集成测试）：解析 `cargo tree -p sebas-db` 与 `sebas-db` 公开符号，断言不含角色实现、不含域表名（`projects` / `session_map` / `users` 等字面量）；验证：测试通过，且临时引入域依赖时失败（附一次失败演示）

## 2. 下沉连接配方与 schema 原语

- [ ] 2.1 搬入连接配方（open / open_readonly / WAL + busy_timeout=5s + foreign_keys=ON）并暴露两种事务入口；验证：新增断言测试读回 `journal_mode == "wal"`、`busy_timeout == 5000`、`foreign_keys == 1` 全部通过
- [ ] 2.2 搬入 `SchemaColumn` / `TableSchema` / `type_affinity` / `SyncOutcome` / `SyncFail` 与启动同步算法（列级 diff、ADD COLUMN、自描述版本戳、不兼容重置）；验证：`sebas-db` 内新增的迁移单测通过，且 `src/sebas_state/migration.rs` 既有 19 个测试迁移后仍全绿
- [ ] 2.3 搬入单写 actor（`StateWriter` / `StateHandle` / `Cmd` / `CmdOutcome`），保持线程名、通道容量 128、就绪信号与错误语义不变；验证：`sebas-db` 内 actor 单测通过，且并发序列化行为有测试覆盖

## 3. ActiveRecord 机制（derive + trait + 类型化门面）

- [ ] 3.1 在 `sebas-db` 定义泛型 `Record` trait：表名、主键参数、`to_params`（写）/ `from_row`（读）；验证：trait 及其文档不出现任何域类型名，单测用测试表覆盖 upsert/查/列/删四操作
- [ ] 3.2 扩展 `sebas-schema-derive`：`#[derive(ActiveRecord)]` 生成 `Record` 实现 + 固有方法 `save(&mut Connection)` / `find(conn, pk)` / `all(conn)` / `delete(conn, pk)`（复合主键生成 `find_by` / `delete_by`，单列主键之外的不支持场景编译期报错）；验证：derive 单测覆盖单列与复合主键、不支持主键的编译失败
- [ ] 3.3 生成的 SQL 与既有 repo 自由函数黄金样本比对（同表同行 upsert 逐字一致），每张测试表一个 save → find 往返全等测试；验证：比对测试通过
- [ ] 3.4 `StateHandle` 增加类型化门面：`save(&R)` / `find::<R>(pk)` / `all::<R>()` / `delete::<R>(pk)`（经既有 actor 串行执行），多表原子性走既有 `exec` 闭包；验证：单测断言门面操作经单写线程执行且闭包事务整体提交/回滚

## 4. sebas-models：core 各表的 struct

- [ ] 4.1 新建 `sebas-models`（依赖 `sebas-db` / `sebas-domain` / serde），把 `ProjectRow` / `SessionMapRow` / `ProviderRow` / `ModelAliasRow` / `SettingRow` 迁入并挂 `#[derive(ActiveRecord)]`；验证：`cargo build --workspace` 通过，且 derive 首次在非根 crate 工作（3.2 的证明即本任务）
- [ ] 4.2 `repo.rs` 的非标准查询（`import_defaults_once`、按 provider 查别名等）改为 `sebas-models` 内返回 struct 实例的查询函数；验证：`grep -rn "query_row" sebas-models/src/` 的每处返回类型都是表 struct 或标量聚合，无 `Map<String,Value>`
- [ ] 4.3 `DbStateEngine` 改为在 `handle.exec` 闭包里调用生成的方法，`StateStoreEngine` 端口 trait 与 `MemoryEngine` 测试替身不变；验证：`cargo test -p sebas --test state_persistence_test --test state_subscription_test` 一行未改全绿
- [ ] 4.4 删根 crate 的 `repo.rs` 自由函数与行 struct 旧定义；验证：`grep -rn "struct ProjectRow\|struct SessionMapRow\|struct ProviderRow\|struct ModelAliasRow\|struct SettingRow" src/` 无命中（定义只在 `sebas-models`）
- [ ] 4.5 列元数据一致性：同一 struct 在迁移前后 `schema_columns()` 输出逐列相等；验证：迁移前录制基线，迁移后断言相等
- [ ] 4.6 复核域 schema 未下沉；验证：`grep -rn "CREATE TABLE" sebas-db/src/ sebas-models/src/` 无输出（DDL 仍在根 crate 注册表），且 `cargo tree -p sebas-db`、`cargo tree -p sebas-models` 均不含角色实现

## 5. user_store 复用共享配方与 ActiveRecord

- [ ] 5.1 `sebas-webui::user_store` 改为从 `sebas-db` 取连接与 pragma，删除手抄配方（`user_store.rs:278-311` 的复制部分），**保留** `TransactionBehavior::Immediate` 与 `user_version` 机制；验证：`cargo test -p sebas-webui` 全绿，其中 user_store 测试区与 auth 相关用例一行未改
- [ ] 5.2 `User` struct 挂 `#[derive(ActiveRecord)]`（表 `users`），标准 CRUD 换用生成方法，`last-root` 计数等聚合保留手写 SQL 但返回标量/实例；验证：user_store 既有用例全绿，且手写 SQL 只剩非标准查询
- [ ] 5.3 加 `auth.db` 的 pragma 断言测试（与 2.1 同形）；验证：`journal_mode`/`busy_timeout`/`foreign_keys` 三个读数与改造前一致
- [ ] 5.4 验证未来版本仍被拒绝：构造 `user_version` 大于当前常量的库；验证：仍以 `IncompatibleVersion` 拒绝（行为未变）
- [ ] 5.5 复核 `user_store` 不再自建第二套连接机制；验证：`grep -rn "pragma_update" sebas-webui/src/` 无输出

## 6. 全量回归与收口

- [ ] 6.1 跑进程级 e2e：`invoke testsuite-e2e`；验证：全绿
- [ ] 6.2 跑验收套件：`invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位并记录
- [ ] 6.3 沙箱复核两个 DB 的落点与所有权（`SEBAS_STATE_DB` / `SEBAS_WEBUI_AUTH_DB` 均钉在沙箱内）；验证：按 AGENTS.md 沙箱菜谱启动后两个库都在沙箱目录、真实 `~/.sebas` 未被触碰
- [ ] 6.4 更新 `AGENTS.md` / `CLAUDE.md` 的 crate 速查表，加入 `sebas-db` 与 `sebas-models` 的定位（runtime vs ActiveRecord struct）、准入规则与「模式统一、归属按写入者」的说明；验证：速查表内容与 `specs/persistence-runtime/spec.md` 一致
