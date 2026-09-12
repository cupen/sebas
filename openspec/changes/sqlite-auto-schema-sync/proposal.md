## Why

现在每加一个字段都要手写 `migration_N` 函数（迁移链已到 v2），而日常最常见的变化恰恰是"model struct 加个列"。项目**尚未正式发布、没有需要保全的存量数据**，开发期应当允许不停的破坏性 schema 修改——这类变化不该消耗任何人工脚本。Rust 没有运行时反射，但可以用编译期 derive 宏从 struct 定义提取列清单，达到同等效果：**代码里的 model 对象就是 schema 的单一事实源**，启动时与实际表结构对比，能加则加，不能加则重置。

## What Changes

- **编译期"反射"**：row struct（`repo.rs` 的 `*Row` 类型）挂 `#[derive(SchemaColumns)]`（workspace 新增 proc-macro crate），编译期生成列元数据（列名、SQLite 亲和类型、常量默认值）；struct 字段即事实源。
- **启动 diff 同步**：对每张注册表对比派生列清单 vs `PRAGMA table_info`——缺列自动 `ALTER TABLE ADD COLUMN`。
- **不兼容即重置（开发期语义）**：类型不符、多余列、缺表、未知版本格式 → 删库并按当前 model 重建空 schema，日志如实记录"schema 不兼容已重置"。破坏性修改从此零人工。
- **manual 迁移链退役**：`MIGRATIONS` 数组、版本逐级推进、TooNew 拒启、迁移前备份一并删除（没有存量数据需要兼容，逃生舱失去存在理由）；`migration.rs` 收敛为同步/重建逻辑。
- **版本字段自描述（观测用途）**：版本落库为 meta 键值（`version_format` + `version`），当前 format `date`、值为日期常量 `SCHEMA_VERSION`（YYYYMMDD，改 schema 时 bump；不是 wall-clock）。作用是诊断"这库是哪个 schema 日期的"，并为发布后可能的迁移机制预留格式位。
- **一次性对齐**：现有 5 表（providers/model_aliases/settings/projects/session_map）的 struct 逆向对齐作为接入基线（对不上就重置，无需迁移动作）。

## Capabilities

### New Capabilities

（无——schema 演进属于 state-store 既有能力面的延伸）

### Modified Capabilities

- `state-store`：「Schema version and auto-migration」改写为开发期语义：自描述版本字段 + 缺列自动同步 + 不兼容重置（不再逐级迁移、不再拒启过高版本）；「Pre-migration backup」REMOVED（重置前不留备份）；「Corrupt store is not silently reset」保留并划清边界——**损坏（corruption）仍拒启**，重置只针对 schema 不兼容，两者不得混淆。

## Impact

- 代码：`src/sebas_state/{migration.rs, repo.rs, db.rs}`（迁移框架拆除 + 同步逻辑）、workspace 新 proc-macro crate；core 打开 DB 的路径与单写者语义不变。
- 硬约束：SQLite `ADD COLUMN` 不能加 PRIMARY KEY/UNIQUE 列、NOT NULL 必须带常量默认值——derive 宏在此类字段上编译期拒绝，把问题挡在构建时。
- 数据安全边界：重置只发生在 schema 不匹配时；DB 文件损坏（打不开/页校验失败）仍按既有要求拒启。

## Non-goals

- 不做重置前的数据备份（开发库可弃；确想保留时手动拷文件即可）。
- 不做任何 legacy 版本入轨或格式切换迁移。
- drop/rename 列、改列类型仍不"自动改"（SQLite 方言限制）——处理方式是重置重建，不是原地改。
- 正式发布后的兼容/迁移策略不在本变更范围（届时需重审"不兼容即重置"语义，另立变更）。
