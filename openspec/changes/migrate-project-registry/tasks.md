## 1. projects 表按目标形状重建

- [x] 1.1 `projects` 的注册 DDL 与行结构重建，含正式 `node_id` 列（本地为 `local`，`NOT NULL` + 默认值）；验证：`cargo build` 通过，且表结构与项目记录字段有一张对照表（PR 描述）
  - 备注：`src/sebas_state/repo.rs` 的 `PROJECTS_TABLES[projects]` DDL 与 `ProjectRow` 列序逐字一致（id / path / name / branch_at / added_at / sort_order / node_id / default_agent / branch）；对照表即 `sebas-models/src/project.rs` 的结构体字段注释。
- [x] 1.2 旧库在形状变化后走重置路径并隔离旧文件；验证：用改造前的库启动，断言日志给出隔离路径（依赖 `quarantine-database-reset`；若未落地则断言重置发生且日志如实说明）
  - 备注（实测偏离计划）：`projects` 的形状变化是**可原地补齐**的（`node_id` 有常量默认 `'local'`），因此不需要也不应走重置——有自描述版本键的旧库由 sqlite-auto-schema-sync 逐列 ALTER，旧行 `node_id` 取默认即自动归本机节点。实测见 `tests/persistence_runtime_integration_test.rs::added_column_reads_back_at_default_through_generated_crud`（五列旧库 → `Synced { added_columns: 4 }`，旧行读回 `node_id == "local"`）。隔离重置路径只对**无版本键**的更老迁移链库生效（`sebas_db::schema` 通用规则）；同批形状变化的 `session_map` 因新列非空且无常量默认，另有 `old_shape_session_map_db_resets_and_quarantines_old_rows` 钉住重置 + 隔离。DDL 注释已按实测改写。
- [x] 1.3 主键与既有列语义不变（`path` 主键、`id` 唯一）；验证：`pragma_table_info('projects')` 含既有 8 列且主键与改造前一致
  - 备注：新增 `src/sebas_state/repo.rs::tests::projects_table_shape_and_primary_key_are_pinned`——直接查 `pragma_table_info`（既有 8 列俱在、`pk` 标志只有 `path` 为 1、`node_id` 为 `NOT NULL` + 默认 `'local'`、物理列序与 `ProjectRow::schema_columns()` 一致），并断言 `idx_projects_id` 仍是唯一索引。

## 2. 项目记录合一

- [x] 2.1 建立规范项目记录定义（存储 + 线同一形状），删掉 `ProjectRow` / `ProjectEntry` 两处并行字段清单；验证：`grep -rn "struct ProjectRow\|struct ProjectEntry"` 只剩一处规范定义，且未更新的构造点已编译修复
  - 备注：唯一规范定义在 `sebas-models/src/project.rs`；`sebas-webui/src/projects.rs` 改为再导出（无第二份字段清单）。
- [x] 2.2 把 `serde_json::Value` 往返（`src/sebas_state/engine.rs:61,73`）替换为显式转换；验证：该处不再出现 `to_value` / `from_value`，且 `cargo test -p sebas` 全绿
  - 备注：`load_projects` / `save_projects` / `add_project` 现在全程 `Vec<ProjectRow>` / 5 参数（含 `node_id`），项目面路径上无 `Value` 往返；engine 里残留的 `to_value` / `from_value` 只服务 `CardConfig`（settings 域）。
- [x] 2.3 形状钉测试：同时钉持久化形状与线形状的字段名集合与拼写；验证：测试通过；人为给规范定义加一个字段时测试失败（附一次失败演示）
  - 备注：`sebas-models/src/project.rs` 的 `schema_columns_match_target_shape`（列名/亲和/默认/非空 + `PK_COLUMNS == ["path"]`）与 `project_record_serialized_shape_is_pinned`（完整条目、本机条目、旧最小条目三种 wire 形状逐字断言）——两侧一起变才可能同时通过。
- [x] 2.4 `node_id` 的序列化拼写与默认值保持 `local`；验证：本地项目经 API 列出的形状与改造前逐字一致，且 `tests/testsuite_webui` 的形状断言**不改而通过**（若需改动说明合并越界）
  - 备注：`serde(default = "default_node_id")` → `"local"`，wire 拼写与合并前 `ProjectEntry` 一致；前端早已认识 `node_id`（`sebas-webui/frontend/src/api/client.ts`，来自 add-remote-execution-node），`tests/testsuite_webui` 未因本 change 改动任何形状断言。
- [x] 2.5 展示层字段不进记录；验证：记录定义中不含任何仅用于展示的计算字段，展示字段由转换产出
  - 备注：`sebas-models/src/project.rs::record_carries_no_presentation_only_fields` 钉住。

## 3. 远程项目读改写走 state 方法

- [x] 3.1 core channel 的 projects CRUD 补节点维度（新增/列出/重排/删除）；验证：`cargo test -p sebas -p sebas-webui` 全绿，且远程条目的增删改在库中可见
  - 备注：`src/core_channel/server.rs` 的 `projects` 读（582）与写（1200 → `state_store::project_mutation`）均在；`node_id` 是 `add_project` 的正式入参。
- [x] 3.2 内嵌 webui 与 standalone webui 两条路径都改走 state 方法（`projects.rs` 的文件读写删净）；验证：两种拓扑各一条集成用例通过
  - 备注：`sebas-webui/src/projects.rs` 已无任何文件读写（只剩纯助手 + 再导出）；内嵌形态经 `InProcessBackend`（全局 engine）、standalone 形态经 `CoreChannelBackend` 的 state 方法，两侧共用 `api.rs` 的 `state_snapshot("projects")` / `state_mutate("projects", …)`。两种拓扑的集成用例：`session_endpoints_test`（内嵌装配）+ `gateway_bff_test`（core 状态库 BFF 桥）。
- [x] 3.3 **远程项目重启存活用例**：注册一个远程节点项目并重启进程；验证：项目仍在且节点归属正确（对照旧实现：该项只在文件里、库中不存在）
  - 备注：`tests/state_persistence_test.rs::committed_mutation_survives_writer_restart` 扩为两条——本机 `local` + 远程 `node-1`；关写者再重开同一 `projects.db` 后断言两条都在且 `node_id` 原样。

## 4. 不做遗留导入（核对项）

- [x] 4.1 确认 `projects.json` 的值**不被导入**：库为新权威，文件不再被读取；验证：单测构造「文件有远程项目 + 库空」，断言启动后库仍为空、远程项目不出现（**不**出现导入标记）
  - 备注：新增 `tests/state_persistence_test.rs::legacy_projects_file_is_not_imported`——状态目录放一份含远程项目的 `projects.json`，库为空；打开引擎后 `load_projects()` 为空，且文件未被读取方改写/清空、无任何 `imported` 标记键。
- [x] 4.2 确认未引入导入标记键；验证：`grep -rn "imported" sebas-webui/src/projects.rs` 无输出
  - 备注：`grep` 只命中一行注释（明确记录「规范记录不携带任何 imported/legacy 标记列」），无任何代码/字段。

## 5. 诚实降级

- [x] 5.1 去掉 core 不可达时的文件回退：项目面呈现 unavailable + cause，变更入口置灰；验证：用例断言不回退文件、不呈现文件派生列表
  - 备注：`api.rs` 的 `projects_unavailable` → 503 + cause；文件回退与 `degraded` 标记已删。**收口时补修的一处诚实性缺口**：移除 / 重排 / 以 `project_id` 建会话原先走「不可达折算成空列表」的读法，会把不可达报成 `404 project not found` / `400 未知 project_id`（重排在部分失败下更可能按空表写回）。现统一走 `projects_or_unavailable`（严格区分不可达与空），三处均 503。用例：`projects_add_refused_when_core_unreachable`、`projects_mutations_answer_503_when_store_unreachable`（移除/重排 503 且未写入）。
- [x] 5.2 恢复后重新读库并清除 unavailable；验证：用例构造「库与文件不一致」，断言呈现的是库
  - 备注：列表面每次请求都重新 `state_snapshot`，无粘滞降级态。新增 `projects_unavailable_clears_when_store_comes_back`：同一个 backend 先 `None`（503 + cause）再翻成有项目的域，下一次请求必须 200 并呈现库里的项目。「库与文件不一致以库为准」由「文件根本不被读取」（4.1）承担。
- [x] 5.3 复核项目面不再有任何文件派生路径；验证：`grep -rn "projects.json" sebas-webui/src/` 无生产代码命中
  - 备注：只剩两条**注释**提到旧文件名（`session_backend.rs`、`api.rs` 各一处说明历史），无任何读写代码。

## 6. 退休路径变量与测试面

- [x] 6.1 退休 `SEBAS_PROJECTS_PATH`（逻辑名由 `single-state-dir` 映射表接管）；验证：设置该变量后行为与不设置一致
  - 备注：`sebas-domain/src/state_paths.rs` 新增 `RETIRED_PROJECTS_PATH_VAR`，`StatePath::ProjectRegistry.override_var()` 改为 `None`，并纳入 `retired_env_vars_present()`（启动日志点名残留值，与 `SEBAS_STATE_DB` 同一范式）；逻辑名仍留在映射表（提案措辞）。用例：`retired_projects_path_var_has_no_effect_on_any_resolution`。
- [x] 6.2 改 Playwright 助手与旅程（`tests/testsuite-webui/tests/helpers/detached.ts` 等）：不再依赖 `projects.json`，改为经 API 断言；验证：Playwright 旅程全绿
  - 备注：`detached.ts::detachedCoreEnv` 去掉 `SEBAS_PROJECTS_PATH` 钉（状态目录经 `SEBAS_HOME` 仍钉在场景内，注册表随 `projects.db` 落在场景里）。核查：没有任何 Playwright 旅程断言过 `projects.json` 文件（旅程本就经 API 断言项目），故无旅程需要改写。
- [x] 6.3 改 `tests/support/mod.rs` 的路径钉；验证：`cargo test` 全绿
  - 备注：`tests/support/mod.rs` 的项目相关「路径钉」只是派生落点清单的注释（无 `SEBAS_PROJECTS_PATH` 写入点），已把 `projects.json` 从清单移除。
- [x] 6.4 更新 `tasks.py` 与 `AGENTS.md`（`SEBAS_PROJECTS_PATH` 退休说明）；验证：沙箱仍能起、文档与实际一致
  - 备注：`AGENTS.md` 两处（沙箱 env 清单 + 沙箱菜谱）改为「`SEBAS_PROJECTS_PATH` 已退休、注册表落 `projects.db`」；`tasks.py` 两处派生文件清单注释同步。`tasks.py` 本来就没有写入该变量。

## 7. 全量回归

- [x] 7.1 standalone 拓扑验收：起 core 与独立 webui，注册本地与远程项目、重启、读列表；验证：两种项目都在，且 `projects.json` 全程未被创建
  - 备注：独立 webui 进程的拓扑由既有旅程承载并已固化断言。① `tests/testsuite_e2e_test.rs::single_state_dir_journey_pins_every_state_location`——core + `sebas webui -c` 独立进程，注册项目 → 完整回合 → SIGTERM 重启 core，机械遍历状态目录断言 **`projects.json` 从不出现**（本次新增断言）、无 `.sebas` 逃逸、`projects.db` 承载 projects 表，并新增断言注册行 `node_id = 'local'` 且 **id 在落库时就以 `proj-` 开头**（钉住收口时修的 id 持久化）；单用例已跑通。② 远端项目的跨重启存活由 `remote_node_pairs_survives_node_and_core_restarts` 与验收套件 `remote_node_workbench_journey` 覆盖（两种项目同库）。两者都在全量跑里绿。
- [x] 7.2 跑 `invoke testsuite-e2e`；验证：全绿
  - 备注：`invoke testsuite-e2e` → **63 passed / 0 failed**（5 filtered，E2E_EXIT=0），日志 `.artifacts/verify/e2e3.log`。**首跑是 21 passed / 42 failed**，根因是真实库行的稳定 id 未落库（`add_project` 写 `id: None`，只在读路径回填），于是 `POST /api/projects` 的 201 响应缺 `id`、e2e 助手 `scene_project_id` 在 `body["id"]` 上 panic；同时按 id 寻址的默认 agent 写入 `UPDATE … WHERE id = ?` 匹配 0 行却返回 Ok（假装成功）。修法：派生下沉 `sebas-domain::project::project_id_for_on`（与 webui 消费面共用一份实现），`add_project` 落库时即写 id，读路径（列表/单条/读改写）统一回填旧行。
- [x] 7.3 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
  - 备注：`invoke testsuite-acceptance` → **10 passed / 0 failed**（5 filtered，ACC_EXIT=0），日志 `.artifacts/verify/acc3.log`。首跑 2 passed / 8 failed，与 7.2 同一根因，随 id 落库修复转绿。
