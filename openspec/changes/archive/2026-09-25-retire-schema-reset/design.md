# 设计：retire-schema-reset——schema 同步废除删库，改为保数据自动迁移

## Context

现状：`src/sebas_state/migration.rs` 在启动时做「缺列 `ADD COLUMN`，其余不兼容（类型不符 / 多余列 / 缺表 / 未知版本格式）→ 删库重建」。derive 宏（`sebas-schema-derive`）已在编译期产出列元数据，但只覆盖「加列」一种迁移。rusqlite 0.40 bundled = SQLite ≥ 3.50，`RENAME COLUMN` / `DROP COLUMN` / 事务内 DDL 均可用。运行时 FK 开启（repo 写路径依赖 FK 顺序）。动机与用户价值见 proposal.md — Why；行为契约见 specs 增量。

## Goals / Non-Goals

**Goals:**
- reconcile 分派完整化：改名（RENAME）/ 类型变更（事务内重建）/ 删列（DROP 或重建）三条自动迁移路径，数据按列名保全。
- 重置路径整体消失：任何 sync 结果都不删 DB 文件；未知/缺失版本键只 WARN 并照常 reconcile。
- 破坏性步骤（重建/删列）前 `VACUUM INTO` 单文件备份；失败语义 fail-closed（回滚 + 拒启动）。
- derive 新增 `#[column(rename_from)]`，编译期校验标注合法性。

**Non-Goals:**
- 不做改名启发式（不猜）、不迁移表级改名/删表、不 diff 索引漂移（见 proposal Non-goals）。
- 不动单写者、WAL、`db.rs` open 语义、损坏拒启路径。
- 不提供 reset CLI / 交互确认。

## Decisions

### D1. reconcile 两阶段：先纯读构建计划，再备份、后单事务执行

启动同步拆成三步：**(a) 计划构建**（只读：逐表 diff 派生列 vs `PRAGMA table_info`，解析 rename 对，预判受限列，产出带序动作清单）；**(b) 备份**（当且仅当计划含破坏性动作：rebuild / drop——`VACUUM INTO` 不能在事务内跑，必须在开事务前完成）；**(c) 单事务执行**（全部表的全部动作 + stamp 版本键，一提交全生效）。备选「逐动作即时执行」被否：无法保证备份先于破坏、也无法原子生效。备选「每表一事务」被否：多表联动改 schema 时留下半迁移状态。

### D2. 改名：显式 `#[column(rename_from = "旧列名")]`，执行期退化规则

derive 解析新属性；编译期校验：`rename_from` ≠ 自身列名、同 struct 内不得与其它字段的 `rename_from` 重复。运行期：live 表存在旧列 → `ALTER TABLE RENAME COLUMN old TO new`；旧列不存在（新装库 / 已迁过）→ 退化为普通 add + WARN。改名与类型变更叠加时先 RENAME 再按普通类型不符走重建（D3），两步同在 D1 的事务里。备选「schema_meta 存上次列清单做启发式匹配」被否：误判面大（真删+真加会被猜成改名），显式标注把迁移意图留在 code review 里。

### D3. 重建配方：官方 12 步，注册清单结构化拆分

类型不符 / 受限删列统一走表重建：`PRAGMA foreign_keys=OFF`（事务外）→ 开事务 → 以临时名建新表 → `INSERT INTO new(cols) SELECT cols FROM old`（按派生列名交集，新列落常量默认，SQLite 亲和自动转换）→ `DROP` 原表 → 临时表 `RENAME TO` 原名 → 按注册索引段重建索引 → 提交 → `foreign_keys=ON`。

前置重构：`REGISTERED_TABLES` 的 `create_ddl` 整段字符串拆为 `create_table_ddl` + `index_ddls: &[&str]` 两个字段——重建需要参数化表名（临时名），整段字符串替换表名太脆。首建 / 重建路径从两段重新组装，对「首建」行为零变化。备选「运行期字符串替换表名」被否：注释、列定义里出现表名子串时静默出错。

### D4. 删列：`DROP COLUMN` 快路径 + 受限预判升级重建

多余列先查 `pragma index_list` / `pragma index_info` / `pragma table_info`（PK 成员）预判引用：无引用 → 直接 `ALTER TABLE DROP COLUMN`；被引用（索引、PK、约束）→ 升级为 D3 重建。预判失准（异常形状旧库）时，执行期 SQLite 报错按 D5 fail-closed，不做静默重试——保守但可诊断。数据语义：删字段即弃该列数据（proposal 已定），日志点名表与列。

### D5. 备份与 fail-closed

备份：计划含破坏性动作时，`VACUUM INTO '<db目录>/<db文件名>.pre-sync'`（3.27+ 一致性快照；目标已存在则先删后建，覆盖式单文件，不堆积）。备份失败 → 整个 sync 拒启动，报错点名备份路径，不执行任何迁移。执行失败 → 事务回滚 + 拒启动 + 诊断点名失败步骤；`SyncFail::Incompatible` 变体连同重置分支整体删除（fail-closed 不再有任何「兜底成删库」的出口）。`VACUUM INTO` 选型备选「rusqlite backup API 页循环」：功能等价但代码更长；「裸文件拷贝」被否：WAL 下可能拷出不一致快照。

### D6. 版本键：纯诊断固化

`version_format` 检查从「不符即重置」改为「缺失/未知 → WARN + 照常 reconcile」；`schema_meta` 缺表照旧 `CREATE TABLE IF NOT EXISTS` 自建。结构 diff 是唯一动作触发，`SCHEMA_VERSION` 改 schema 时 bump 的约定不变。旧迁移链库（无 schema_meta）首开即被结构 reconcile 吸纳，不再触发任何删除。

### D7. 观测面：`SyncOutcome` 演进

`Reset` 变体删除；新增结构化动作计数（`renamed` / `rebuilt` / `dropped` / `added_columns`），供日志与测试断言。每次非常量动作（rename/rebuild/drop/add）都有 WARN/INFO 日志点名表、列、原因——「重置日志写明触发点」的既有要求平移为「迁移日志写明触发点」。

## Risks / Trade-offs

- [亲和强制转换悄悄改值（如 INTEGER→TEXT 把数字变字符串）] → 迁移日志点名涉及的表列如实提示；备份文件兜底；SQLite 亲和规则本身确定可预期。
- [重建期间崩溃留临时表] → 全程单事务，崩溃即回滚；`__sync_tmp` 名以启动时孤儿检测兜底（计划构建发现同名临时表先 DROP，上一轮未完成的事务不可能提交过）。
- [`DROP COLUMN` 预判漏掉罕见约束形态] → 执行期报错 fail-closed（D5），不会损坏数据，最坏是拒启动让人改代码。
- [`rename_from` 写错旧列名] → 退化为 add + WARN（数据留旧列），不丢数据但也不迁移；derive 校验拦住拼错自身列名的低级错误。
- [备份占盘] → 每库至多一份覆盖式文件；备份永不自动删除（用户手工清理）。

## Migration Plan

单提交落地，无灰度。现有开发库结构本就与 model 一致 → 升级后首开 `UpToDate`，无迁移发生；带旧格式版本键的库首开走 WARN + reconcile（结构一致则仅补 meta 键）。回滚 = revert 提交；`.pre-sync` 备份文件在 revert 后的二进制下无消费者，不冲突。

## Open Questions

（无——改名检测、删列语义、备份与失败语义均已在拷问中定死并落入 specs 增量。）
