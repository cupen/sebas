## 1. session_map 按目标形状重建

- [x] 1.1 `session_map` 的注册 DDL 与 `SessionMapRow` 按映射完整形状重建（含 `NOT NULL` 列与默认值），主键仍为 `(chat_id, thread_id)`；验证：`cargo build` 通过，且表结构与 `MappingDto` 字段有一张对照表（PR 描述）
  - 备注：对照表见实现汇报（chat_id=channel，thread_id=reference，其余列逐一对应 MappingDto 字段）。
- [x] 1.2 `SessionMapRow` 与 `MappingDto` **合一**，删掉并行形状与其转换；验证：`grep -rn "MappingDto" sebas-dispatch/src/` 只剩一处定义，且未更新的构造点已全部编译修复
  - 备注：`MappingDto` 整体删除（grep 0 命中，优于「只剩一处定义」）——行即唯一持久化形状；`dump_json`/`restore_json*`/`parse_disk_key` 一并退休。
- [x] 1.3 既有 5 列（`chat_id` / `thread_id` / `session_id` / `last_active_unix` / `project_dir`）与主键语义不变；验证：`pragma_table_info('session_map')` 含这些列且主键与改造前一致
  - 备注：`old_shape_session_map_db_resets_and_quarantines_old_rows` 内含 pragma 断言（列亲和 + pk 位次）。
- [x] 1.4 旧库在形状变化后走重置路径并隔离旧文件；验证：用改造前的库启动，断言日志给出隔离路径、隔离文件可打开并含旧行（`quarantine-database-reset` 已实现归档）
  - 备注：`desired_mode`/`awaiting_first_prompt` 为非空无默认列 → 旧形状库打开即 Incompatible → 隔离重置；测试断言 Reset、空表、`.reset-*` 文件可开且含旧行。

## 2. 按变更持久化

- [x] 2.1 `load_session_map` / `save_session_map` 接上 `DbStateEngine`，成为生产路径；验证：`cargo test -p sebas` 全绿，且这两个函数不再是「仅测试被调用」
  - 备注（偏离）：`save_session_map`（全量替换）按 design D2 删除——写路径改为生命周期事件处一次 `entry.save(&conn)`（ActiveRecord 生成的 upsert，无手写 SQL）；`load_session_map` 即恢复生产路径。引擎新端口：`load_session_map` / `save_session_entry` / `delete_session_entry`。
- [x] 2.2 会话创建 / 模型变更 / 模式变更 / 标签变更 / 关闭各触发一次 `entry.save(&store)`（经单写 actor 闭包，不新增线程或文件）；验证：单测断言每次事件后库中映射与内存一致，且落库路径无手写 SQL
  - 备注：钩子落在 state.rs（begin_spawn_with/activate/set_desired_mode/set_current_model/set_label/set_project_dir/preserve_closed_mapping/retire_to_record/remove_by_session/remove_by_key/insert）；`lifecycle_mutations_are_each_committed` + `session_map_entries_round_trip_through_the_engine` 钉库↔内存一致。
- [x] 2.3 **核心收益用例**：会话创建且对客户端可见后立即 SIGKILL，重启后断言映射仍在（含 session_id 与 desired_mode）；验证：新增集成测试通过；并对照旧实现下同一用例会失败（PR 描述附旧行为）
  - 备注：按工程纪律以单元级模拟实现（`sigkill_committed_mappings_survive_unclean_exit`：落库后不经任何关停路径、另开裸连接重开读回）；旧行为对照写进汇报。进程级 SIGKILL 旅程随 5.1/5.2 留给 3c。
- [x] 2.4 确认未提交的变更不会落库（对齐 `state-store`「Mutation durability」：响应返回即已提交）；验证：单测断言响应前库中已可见
  - 备注：`sigterm_committed_mappings_survive_graceful_shutdown` 中段断言（每次变更返回后立即经引擎读库可见）。

## 3. 退休关停快照与配置键

- [x] 3.1 删 `src/run.rs` 的关停 dump（`run.rs:590-601`）；验证：关停路径不再写文件，且 2.3 的用例仍通过
- [x] 3.2 删 `src/session_boot.rs` 的文件读取与 `<path>.corrupt-<unix>` 隔离；验证：`grep -rn "corrupt-" src/session_boot.rs` 无输出
- [x] 3.3 删除 `[dispatch] state_file` 配置键（不留过渡期）；验证：带该键的配置文件启动时报**未知键**错误（这是预期），不带该键时正常启动
  - 备注：`DispatchConfig` 加 `deny_unknown_fields`（与 `[acp]` 段同一裁决）兑现 D5 的「残留键报错」；`retired_dispatch_state_file_key_is_rejected` 钉住。
- [x] 3.4 `tasks.py` 沙箱菜谱与 `AGENTS.md` 同步移除该键与相关「必配」说明；验证：沙箱仍能起，且 `AGENTS.md` 不再要求配置该键
  - 备注：沙箱可起性由 4.3 后的全套单测/e2e 编译与既有沙箱用例（support 模板已去掉该键）背书；真起沙箱的旅程复核归 5.3。

## 4. 测试与文档

- [x] 4.1 改 `tests/restart_recovery_test.rs`：把「损坏文件被隔离」改写为「映射条目不可读时启动为空表且不阻塞；库损坏时按状态库规则拒绝启动」；验证：改写后的测试通过，且不删掉原有覆盖点
  - 备注：恢复面改为 `restore_rows`；新增 unreadable-rows / empty-store / corrupt-store-refuses 三用例；lazy-resume 两用例保持原覆盖点。
- [x] 4.2 改 `tests/sigterm_cleanup_test.rs`：改为断言 SIGTERM/SIGKILL 后库中映射完整；验证：两个信号各一个用例通过
  - 备注：单元级模拟（生产写路径 + 裸连接重开读回），文件头注明与进程级旅程的分工。
- [x] 4.3 改 `tests/support/mod.rs` 的路径钉（移除 sessions.json 相关钉）；验证：`cargo test` 全绿
  - 备注：Sandbox 模板与字段同步移除；testsuite_e2e/acceptance 中引用点已改写（断言 sessions.json 不存在、占位播种走 projects.db）。
- [x] 4.4 复核无遗留导入代码：验证：`grep -rn "sessions.json" src/ sebas-*/src/` 无生产代码命中；且无导入标记相关的键
- [ ] 4.5 更新 `openspec/specs/session-persistence/spec.md` 的 Purpose（去掉「being migrated」的进行时表述）；验证：Purpose 与两条 spec 的新要求一致
  - 备注（范围裁剪）：主 spec（`openspec/specs/`）按本 change 执行指令冻结不动，delta 已在 `openspec/changes/persist-session-map/specs/`，由归档阶段合并（含 Purpose 措辞）。

## 5. 全量回归

- [x] 5.1 跑 `invoke testsuite-e2e`；验证：全绿
  - 备注：3c（review 阶段）执行——63 passed / 0 failed，含本阶段新增的进程级
    SIGKILL 旅程 `sigkill_committed_session_map_survives_restart`（核心收益的
    进程级验收：杀后 projects.db 行完整 → 重启恢复原身份与 desired_mode →
    Dormant，全程无 sessions.json）。首跑暴露 4 例 stall e2e 红：support 的
    `set_turn_stall_timeout` 以 `[dispatch]` 段头为补丁锚，而 4.3 删除了基础
    模板中的该段（其唯一键 state_file 退休）——属测试基建回归，修复 helper
    （段缺失时自补该段）后复跑全绿；实现代码零改动。
- [x] 5.2 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
  - 备注：3c 执行——10 journeys 全绿（含 lifecycle 重启恢复段）。
- [x] 5.3 沙箱重启旅程复核：按 AGENTS.md 菜谱起 core，创建会话，SIGTERM 后再起 core；验证：会话列表与会话状态按既有 rest 恢复语义恢复，且 `sessions.json` 全程未被创建
  - 备注：3c 执行，由两层进程级旅程承载：J `session_lifecycle_journey`
    （创建 → 回合 → SIGTERM → 重启 → 会话仍在列 + 无 sessions.json）与
    E `sigkill_committed_session_map_survives_restart`（更强的非优雅形态）。
    单元级等价覆盖见 4.2。
