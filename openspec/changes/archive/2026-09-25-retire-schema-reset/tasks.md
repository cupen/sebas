## 1. derive 层：rename_from 标注

- [x] 1.1 `sebas-schema-derive` 解析 `#[column(rename_from = "...")]` 并编译期校验（≠自身列名、不与同 struct 其它字段重复），`sebas-db` 的 `SchemaColumn` 增加 `rename_from: Option<&'static str>` 字段（`schema.rs` 侧类型同步）；单测覆盖解析、重复拒绝、自身冲突拒绝，`cargo test -p sebas-schema-derive` 通过
  - 证据：`sebas-schema-derive` 新增 `ColumnMeta.rename_from` 解析 + 编译期拒绝（自身列名 / 同 struct 重复），`expand` 发 `rename_from:`；`SchemaColumn` 加 `rename_from: Option<&'static str>`。`cargo test -p sebas-schema-derive` 21 passed（含 4 条新测）。
- [x] 1.2 补文档注释（derive crate 顶部类型映射表加 rename_from 行），`cargo doc` 无警告即可
  - 证据：derive crate 顶部 `#[column(...)]` 属性清单补 `rename_from = "旧列名"` 行与编译期拒绝说明；`cargo doc -p sebas-schema-derive -p sebas-db --no-deps` 无警告（顺带把既有的 ```ignore 示例块改为 ```text，消除 `invalid_rust_codeblocks` 警告）。

## 2. 注册清单结构化（design D3 前置）

- [x] 2.1 `sebas-db` 的 `TableSchema` 把 `create_ddl` 拆为 `create_table_ddl` + `index_ddls: &[&'static str]`，各注册清单（根 `repo.rs` 与 `sebas-models` 的 KV/provider 表）与首建/重建路径（`rebuild_schema`）改为两段组装，`schema.rs` 现有全部测试不改断言跑通
  - 证据：`TableSchema` 拆为 `create_table_ddl` + `index_ddls`；根 `repo.rs`（SETTINGS/PROJECTS）、`sebas-models`（provider/runtime_state/project）、`sebas-router/usage.rs`、`sebas-db/fixtures.rs` 全部改两段；`rebuild_schema` 按「建表段 + 索引段」组装。
- [x] 2.2 校验首建布局不变：新建库的 `sqlite_master` SQL 与拆分前逐字一致（临时测试或断言），防注册重构悄悄变形
  - 证据：拆分前后各 dump 一次新库 `sqlite_master`（临时 `tests/zz_golden_dump_test.rs`，已删），`diff` 逐字一致；并落耐久断言 `fresh_db_sqlite_master_layout_matches_registered_ddl_verbatim`（repo.rs，逐条 SQL 与注册字面量相等 + 语句集合无多余）。

## 3. 计划构建（design D1a）

- [x] 3.1 `sebas-db` 的 schema 模块新增只读计划构建：逐表 diff 出动作清单（Add / Rename{old,new} / TypeChange / Drop / CreateTable），rename 对解析（`rename_from` 命中 live 旧列）与受限预判（`pragma index_list`/`index_info`/PK 成员）在此完成；单测：种子旧结构库断言产出的动作序列正确
  - 证据：`build_plan`（只读）逐表产出有序动作；`plan_builds_ordered_action_sequence_read_only` 断言序列 `改名 alpha.title -> name` → `补列 alpha.tag` → `删列 alpha.stale`、计数 (1,1,0,1)、`destructive`，并断言计划阶段库字节不变。
- [x] 3.2 rename 未命中（旧列不存在）退化为 Add + WARN；「一缺一多」无标注不判改名；单测各一条
  - 证据：`declared_rename_with_missing_source_degrades_to_plain_add`（退化为补列，renamed=0）+ 日志测试断言 WARN 含 `退化为普通补列`/`old_note`；「一缺一多」不猜由 `undeclared_missing_plus_extra_is_dropped_and_added_not_guessed` 钉住。

## 4. 执行器与安全网（design D1b/c、D4、D5）

- [x] 4.1 备份函数：计划含破坏性动作时 `VACUUM INTO '<db>.pre-sync'`（已存在先删后建），备份失败拒启动且不动库；单测覆盖成功、失败两路
  - 证据：`backup_database` 用 `VACUUM INTO ?1`（已存在先删后建）；`backup_failure_blocks_migration_and_leaves_db_untouched` 走失败路，成功路由类型重建/删列各测覆盖。
- [x] 4.2 事务执行器：Add / Rename / Drop 快路径照拼、TypeChange 与受限 Drop 走 12 步重建（FK OFF→事务→ON），全程单事务，失败回滚 + 拒启动诊断点名步骤；单测：类型变更保数据、受限删列走重建、注入失败步骤后库字节不变（对比 mtime+内容哈希）
  - 证据：`run_in_transaction` 单事务执行，重建按 12 步（FK OFF 在事务外 → 建 `<table>__sync_tmp` → 交集 INSERT → DROP → RENAME → 重建索引 → `foreign_key_check` → meta/version → commit → FK ON）；`failed_migration_rolls_back_and_leaves_db_byte_identical` 断言字节 + mtime 不变。
- [x] 4.3 删除重置路径：`SyncFail::Incompatible`、`reset_and_rebuild`（现居 `sebas-db/src/schema.rs`）、重置分支全部移除，未知/缺失 `version_format` 改 WARN + 照常 reconcile，`SyncOutcome` 换结构化动作计数（design D6/D7）；`rg -n "reset_and_rebuild\|Incompatible" src/ sebas-db/ sebas-models/` 无残留，旧重置类测试改写为迁移断言
  - 证据：`SyncFail` 枚举与 `reset_and_rebuild` 全删，`SyncOutcome::Synced { added_columns, renamed, rebuilt, dropped }`；未知/缺失 `version_format` 仅 WARN + 照常 reconcile；`rg -n "reset_and_rebuild|Incompatible" src/ sebas-db/ sebas-models/` 无输出（exit 1）；旧重置测试改写为迁移/拒启断言。

## 5. 全量测试与验收

- [x] 5.1 spec 场景逐条落测试：fresh/加列/声明改名保数据/未声明不猜/类型重建保行/删列弃数据/未知版本 reconcile/版本值不触发/备份先行/备份失败阻断/失败回滚拒启——对应 `specs/state-store/spec.md` 全部场景，`cargo test -p sebas -p sebas-db -p sebas-models` 全绿
  - 证据（沙箱前已跑）：`cargo test -p sebas-db -p sebas-models -p sebas-schema-derive` = 43/10/21 passed；根 crate `cargo test`（全部 target，`env -u ANTHROPIC_*`）= 434 lib + 各集成 target 全 ok，exit 0；`cargo test --test persistence_runtime_test` 7 passed（含机械门禁 `table_diffing_exists_only_in_sebas_db`）。
- [x] 5.2 旧迁移链库入库路径：user_version=2 无 schema_meta 的种子库首开被 reconcile 吸纳且数据保留（原 `legacy_user_version_db_without_meta_resets_on_first_open` 反转为保数据断言）
  - 证据：`legacy_migration_chain_db_is_reconciled_and_keeps_data`（user_version=2 无 schema_meta 的种子库首开补列、数据保留、无 `.pre-sync`/`.reset-` 产物）——原 `legacy_user_version_db_without_meta_resets_on_first_open` 已反转。
- [x] 5.3 沙箱联调：按 AGENTS.md 食谱（`SEBAS_STATE_DIR` 一次性目录）起 bare core，手工构造改名+类型变更+删列三个场景各跑一轮启动，核对日志点名与 `.pre-sync` 备份存在；跑 `invoke testsuite-e2e` 确认无回归
  - 证据：`SEBAS_STATE_DIR=/tmp/sebas-rs-sandbox`（全部路径钉进一次性目录，HOME 也钉）起 bare core（`core -c config --no-webui`，零端口、不碰操作员实例），四轮实测：①类型变更（projects.name 库内 INTEGER）→ WARN 点名「列 `name` 类型不符: 库内 `INTEGER` … model 期望亲和 TEXT」+ `projects.db.pre-sync` 备份 + `Synced { rebuilt: 1 }`，行数据保值（42→'42'），二启 UpToDate；②多余列（session_map.stale）→ `多余列: DROP COLUMN（数据随列丢弃，先备份）` + 备份 + `Synced { dropped: 1 }`，列消失、同表其它行完好；③未声明改名（projects.name→title，NOT NULL 无默认）→ `ERROR … 表 projects 缺列 \`name\`: 非空且无常量默认值…拒绝启动并保持数据库原样（删库重置已废除）`，core 如实降级项目域、settings 域照常，`projects.db` 与既有 `.pre-sync` md5/mtime 三轮采样全等（未动库、未产生新备份）；④声明改名（临时给 `ProjectRow.name` 加 `rename_from = "title"` 的一轮验证，跑完即回退+重建）→ `声明改名: RENAME COLUMN 保数据 old=title new="name"` + `Synced { renamed: 1 }`，非破坏性（备份 md5 未变）、数据保值。`invoke testsuite-e2e`：首两轮各有 2 例超时假红（streaming/并发/router 日志等待，逐例单独重跑全绿），第三轮 **71 passed · 0 failed**（报告 `.artifacts/verify/report-e2e.html`）。
- [x] 5.4 `SCHEMA_VERSION` bump 至落地日期常量；`corrupt_db_refuses_to_open_and_file_is_untouched` 原样保留通过（损坏边界未被波及）；两个分层库（`settings.db` / `projects.db`）各跑一轮「改名 + 类型变更 + 删列」验证各自独立适用、互不触碰
  - 证据：`SCHEMA_VERSION = "20260924"`；`corrupt_db_refuses_to_open_and_file_is_untouched` 原样保留通过；`layered_databases_migrate_independently_and_never_touch_each_other` 让 settings.db / projects.db 各跑一轮改名 + 类型变更 + 删列，逐步断言另一库文件字节 + mtime 未变、各自一份 `.pre-sync`。

## 状态备注（收尾）

- 全部 13 项任务完成，逐项证据见上（含沙箱四轮实测与 `invoke testsuite-e2e` 71 passed · 0 failed 的报告路径）。
- 门禁全绿（顺序执行、无并发重型 cargo）：`cargo build` / `cargo doc -p sebas-schema-derive -p sebas-db --no-deps`（无警告）/ `cargo test -p sebas-db -p sebas-models -p sebas-schema-derive`（43/10/21）/ `cargo test --test persistence_runtime_test`（7）/ `cargo test -p sebas-router --lib`（213）/ `env -u ANTHROPIC_* cargo test`（434 lib + 全部集成 target）/ `env -u ANTHROPIC_* cargo test --no-run`（Finished）。
- 本 worktree 已 cherry-pick 主 agent 的既有缺陷修复 `2a9a239`（→ `3252d70`，仅 `src/agent_backend.rs` + `sebas-router/src/anthropic_wire.rs`，与本变更零重叠）；未提交本变更自身改动。
- 环境前置：`cargo build` 不构建依赖 crate 的 bin，跑 `tests/acp_session_mapping_test.rs` 前需先 `cargo build -p sebas-acp`（否则 4/5 假红「agent driver handshake channel closed」）；`invoke testsuite-e2e` 前需 `cargo build -p sebas-node`。
- 残留风险：`DROP COLUMN` 受限预判只覆盖 index / PK / FK 成员；若某列仅被 trigger 或部分索引 WHERE 子句引用，会在执行期由 SQLite 报错并整事务回滚 → 仍 fail-closed 拒启（安全，但日志点名为执行失败而非预判）。
