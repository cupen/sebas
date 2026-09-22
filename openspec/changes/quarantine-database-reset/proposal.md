# Proposal: quarantine-database-reset

## Why

今天「结构不可调和」的处理是**删除状态库并重建空 schema**（`src/sebas_state/migration.rs:89-101`），不可逆且只有一行 warn。开发期每次 schema 演进都会静默清掉本地的项目、provider 与会话映射。

但**产品尚未发布**，所以原计划里围绕「保护既有安装数据」的一整套东西——改名列/删列迁移词表、类型变更、多余列 fail-closed、每个库的重置策略（允许/拒绝）、版本语义统一、把 `auth.db` 纳入迁移机制、JSON→SQLite 的数据导入——**它们的唯一价值就是保护已有安装的数据**。没有发布版，就没有已有安装。因此本 change 把它们全部推迟，只保留一条成本极低、把不可逆变成可逆的加固：**重置前隔离旧文件**。

## What Changes

- **重置改为隔离**：重建空 schema 之前，把库文件（含 `-wal` / `-shm`）改名为 `<path>.reset-<unix>`，并在日志里打出隔离路径与触发原因。沿用 `src/session_boot.rs:315-323` 对损坏 state 文件的 `<path>.corrupt-<unix>` 先例。
- **其余一律不动**：加列仍是唯一的原位迁移词表；版本值本身从不触发重置；不可打开的库仍拒绝启动且绝不重置；`auth.db` 仍走自己的 `user_version` 机制。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

- `state-store`: 「Schema self-description and startup sync」——重置动作由「删除文件并重建」改为「隔离旧文件并重建」，日志须给出隔离路径；其余语义（加列原位、版本值不触发重置、损坏不重置）逐字不变。

## Impact

- **改动**：仅 `src/sebas_state/migration.rs`（`reset_and_rebuild`）与其调用点日志措辞。
- **测试面**：既有测试 `extra_column_resets_database`（`migration.rs:481-511`）需改写为断言隔离文件存在且内容为重置前的库；其余迁移测试不动。
- **不变**：无 schema 变更、无线格式变更、无配置变更。验收 = 既有 `state_persistence_test` / `state_subscription_test` 全绿 + `invoke testsuite-e2e` / `testsuite-acceptance` 全绿。
- **交付面**：极小。这是本批 change 里唯一「改一条路径 + 一个测试」的规模。

## Non-goals

**以下全部推迟到接近首次发布时**（触发条件：出现需要保留数据的外部安装，或准备首个正式发布）。它们今天不做，不是因为难，而是因为**没有需要保护的对象**：

- **改名列 / 删列 / 类型变更的迁移词表**：无发布版时，改列类型或改列名直接重置即可——这正是开发期的正常节奏。词表的价值只在「不能丢数据」时才成立。
- **未声明多余列的 fail-closed（拒绝启动）**：原计划用「拒绝启动」替代「静默重置」以保护数据；无发布版时，降级场景下**拒绝启动反而更烦人**（在分支间来回切会卡住），重置更顺。
- **每个库的重置策略（允许 / 拒绝）与 `auth.db` 的 never-delete 要求**：最强论据是「一次列重命名会删掉操作员的用户账号，而账号不可再生」。开发期账号可重建（`sebas webui-passwd` 一条命令），论据随之消失。
- **版本语义统一（`schema_meta` 与 `user_version` 二选一）**：今天两套机制各自能用；统一是整洁性收益，不是缺陷修复。
- **JSON→SQLite 的一次性数据导入与备份**：没有已发布版本的数据需要搬。
- **保留期 / 隔离文件的清理策略**：开发期隔离文件堆积可接受，日志已给出路径。
- **不引入迁移步骤表或显式版本脚本**：与 `state-store`「model struct is the single source of truth」的既有契约冲突，且今天无需求。
