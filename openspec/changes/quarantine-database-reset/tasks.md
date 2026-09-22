## 1. 重置改为隔离

- [x] 1.1 `reset_and_rebuild`（`src/sebas_state/migration.rs:208-222`）改为：重建前把库文件与 `-wal` / `-shm` 用同一时间戳改名为 `<path>.reset-<unix>`（唯一性不足时追加序号或 pid）；验证：单测断言三个隔离文件存在、且新库为空 schema
  - 落点随 extract-sebas-db 迁至 `sebas-db/src/schema.rs`（`reset_and_rebuild` + `quarantine_db_files`/`quarantine_base_suffix`）；用例 `reset_quarantines_db_file_with_wal_and_shm_under_one_timestamp`
- [x] 1.2 隔离文件是**重置前的完整库**：重置前先做一次提交/关闭，确保最近一次提交在隔离文件中可见；验证：单测断言隔离文件用 rusqlite 打开后能读到重置前的行
  - `sync_conn` 各不兼容分支在返回前 drop 旧连接（关闭即提交、WAL checkpoint 落盘）；用例 `extra_column_resets_database_with_rows_surviving_in_quarantine` 以 `open_readonly` 读回重置前行
- [x] 1.3 日志同时给出触发原因与隔离路径，并明说数据未迁入新库；验证：单测/集成用例断言日志含两者（用 `tracing` 的测试捕获）
  - warn! 移入 `reset_and_rebuild`（path/reason/quarantined 三字段 + 「旧数据未迁入新库」）；用例 `reset_log_names_reason_quarantine_path_and_non_migration`（全局 subscriber + 共享缓冲捕获，见测试内注释的 interest 缓存说明）
- [x] 1.4 既有契约不动：加列仍原位、版本值本身不重置、损坏不重置；验证：`migration.rs` 既有的 19 个测试（除 1.5 改写的那条）不改而全绿
  - 既有用例一行未改（除 2.1 改写的 `extra_column_*`），`cargo test -p sebas-db` 32 通过

## 2. 改写既有断言

- [x] 2.1 把 `extra_column_resets_database`（`migration.rs:481-511`）改写为「重置后原行**在隔离文件中**存活、新库为空」；验证：改写后的测试通过，且断言里出现隔离路径而非删除断言
  - 改写为 `extra_column_resets_database_with_rows_surviving_in_quarantine`
- [x] 2.2 补一条「未知版本格式 → 隔离而非删除」的用例；验证：单测断言隔离文件存在
  - `unknown_version_format_quarantines_file_instead_of_deleting`（且以 rusqlite 读回旧行）
- [x] 2.3 损坏场景回归：不可打开的库仍拒绝启动且**不**产生隔离文件；验证：既有损坏用例不改而通过
  - 既有 `corrupt_db_refuses_to_open_and_file_is_untouched` 未改而绿；另补 `corrupt_db_refusal_produces_no_quarantine_files` 断言无任何 `.reset-` 产物

## 3. 文档与收口

- [x] 3.1 在故障排查文档说明重置会隔离文件、路径形如 `<path>.reset-<unix>`、以及如何手工恢复；验证：文档含路径示例与恢复步骤
  - 新建 `docs/state-db-troubleshooting.md`（仓库此前无故障排查文档）
- [x] 3.2 复核推迟项已在 `proposal.md` 的 Non-goals 与 `design.md` D3 记录（含触发条件）；验证：两处清单一致，且不再出现迁移词表/fail-closed/auth.db never-delete 的实现任务
  - 已核对：两处触发条件一致（外部安装需保留数据 / 首个正式发布）；D3 的重评估清单是 proposal Non-goals 的子集并显式指向后者；tasks.md 无推迟项的实现任务
- [x] 3.3 跑 `invoke testsuite-e2e`；验证：全绿
  - `invoke testsuite-e2e` 58/58 全绿（首跑 `error_scenario_projects_an_error_entry` 时序偶发超时，复跑全量通过；该用例走全新库，不触及重置路径）
- [x] 3.4 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
  - `invoke testsuite-acceptance` 10/10 全绿
