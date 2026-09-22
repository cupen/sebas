## Why

现行 sqlite-auto-schema-sync 的语义是「能加列则加，不能加则删库重建」：类型不符、多余列、缺表、未知版本格式一律删掉整个状态库。开发期可弃数据的前提正在失效——项目进入真实使用后，providers、projects、session_map 里的数据有保全价值，一次无意的 struct 误改就会静默清空用户状态。且 rusqlite 0.40 bundled（SQLite ≥ 3.50）已具备 `RENAME COLUMN`（3.25+）与 `DROP COLUMN`（3.35+）方言能力，事务内 DDL 表重建也完全可行：当初的「方言限制」理由不再成立，删库只是策略选择而非被迫。

## What Changes

- **废除重置路径**：sync 不再删除任何 DB 文件；结构 diff 成为唯一真相，缺 `schema_meta` / 未知 `version_format` 也走同一 reconcile（WARN 记录），版本键继续纯诊断。
- **列改名自动迁移**：derive 新增 `#[column(rename_from = "旧列名")]` 标注；diff 时先解析改名对执行 `ALTER TABLE RENAME COLUMN`，未标注的「一缺一多」按删+加处理（不猜）。
- **类型变更自动迁移**：事务内表重建（12 步配方）——按注册 DDL 建新表、按列名交集拷贝存量数据（SQLite 亲和转换）、换名、重建索引；**BREAKING**（原行为是重置，现行为是保数据迁移）。
- **多余列自动 DROP**：struct 删字段 → `ALTER TABLE DROP COLUMN`，数据随列丢弃；被索引/约束引用的受限列走同一重建路径。
- **破坏性步骤前备份**：执行重建/删列前 `VACUUM INTO` 单文件备份到库旁；备份失败则拒绝执行破坏性迁移（拒启动，不动数据）。
- **失败语义**：reconcile 中途失败 → 事务回滚 + 拒启动 + 诚实诊断，绝不回退成删库。

## Capabilities

### New Capabilities

（无——schema 同步语义属于 state-store 既有能力面的改写）

### Modified Capabilities

- `state-store`：「Schema self-description and startup sync」需求改写——重置场景全部移除，替换为改名/类型变更/删列的保数据迁移场景、备份场景、失败拒启场景；「Corrupt store is not silently reset」保留不动（损坏边界与本次语义严格分离）。

## Impact

- 代码：`src/sebas_state/migration.rs`（sync 主逻辑重写：rename 解析 → add → drop → rebuild 分派 + 备份）、`sebas-schema-derive/src/lib.rs`（`rename_from` 属性解析与编译期校验）、`src/sebas_state/repo.rs`（仅注释/示例级改动，表结构不变）。
- 硬约束不变：`ADD COLUMN` 补不了 PRIMARY KEY/UNIQUE 列、NOT NULL 需常量默认——derive 编译期拒绝规则原样保留。
- 数据安全边界：损坏（打不开/页校验失败）仍拒启且不动文件；重置语义整体消失。

## Non-goals

- 不做列改名的启发式猜测（只认显式 `rename_from` 标注）。
- 不做多列联合改名、表改名/表删除的自动迁移（未注册的多余表维持忽略）。
- 不 diff 索引漂移（索引只在首建/重建时按 DDL 生成，与现状一致）。
- 不提供 reset 逃生舱 CLI 或交互式确认；备份文件是唯一的人工恢复手段。
- 正式发布后的长期迁移策略（版本链、灰度）不在本变更范围。
