## 1. projects 表按目标形状重建

- [ ] 1.1 `projects` 的注册 DDL 与行结构重建，含正式 `node_id` 列（本地为 `local`，`NOT NULL` + 默认值）；验证：`cargo build` 通过，且表结构与项目记录字段有一张对照表（PR 描述）
- [ ] 1.2 旧库在形状变化后走重置路径并隔离旧文件；验证：用改造前的库启动，断言日志给出隔离路径（依赖 `quarantine-database-reset`；若未落地则断言重置发生且日志如实说明）
- [ ] 1.3 主键与既有列语义不变（`path` 主键、`id` 唯一）；验证：`pragma_table_info('projects')` 含既有 8 列且主键与改造前一致

## 2. 项目记录合一

- [ ] 2.1 建立规范项目记录定义（存储 + 线同一形状），删掉 `ProjectRow` / `ProjectEntry` 两处并行字段清单；验证：`grep -rn "struct ProjectRow\|struct ProjectEntry"` 只剩一处规范定义，且未更新的构造点已编译修复
- [ ] 2.2 把 `serde_json::Value` 往返（`src/sebas_state/engine.rs:61,73`）替换为显式转换；验证：该处不再出现 `to_value` / `from_value`，且 `cargo test -p sebas` 全绿
- [ ] 2.3 形状钉测试：同时钉持久化形状与线形状的字段名集合与拼写；验证：测试通过；人为给规范定义加一个字段时测试失败（附一次失败演示）
- [ ] 2.4 `node_id` 的序列化拼写与默认值保持 `local`；验证：本地项目经 API 列出的形状与改造前逐字一致，且 `tests/testsuite_webui` 的形状断言**不改而通过**（若需改动说明合并越界）
- [ ] 2.5 展示层字段不进记录；验证：记录定义中不含任何仅用于展示的计算字段，展示字段由转换产出

## 3. 远程项目读改写走 state 方法

- [ ] 3.1 core channel 的 projects CRUD 补节点维度（新增/列出/重排/删除）；验证：`cargo test -p sebas -p sebas-webui` 全绿，且远程条目的增删改在库中可见
- [ ] 3.2 内嵌 webui 与 standalone webui 两条路径都改走 state 方法（`projects.rs` 的文件读写删净）；验证：两种拓扑各一条集成用例通过
- [ ] 3.3 **远程项目重启存活用例**：注册一个远程节点项目并重启进程；验证：项目仍在且节点归属正确（对照旧实现：该项只在文件里、库中不存在）

## 4. 不做遗留导入（核对项）

- [ ] 4.1 确认 `projects.json` 的值**不被导入**：库为新权威，文件不再被读取；验证：单测构造「文件有远程项目 + 库空」，断言启动后库仍为空、远程项目不出现（**不**出现导入标记）
- [ ] 4.2 确认未引入导入标记键；验证：`grep -rn "imported" sebas-webui/src/projects.rs` 无输出

## 5. 诚实降级

- [ ] 5.1 去掉 core 不可达时的文件回退：项目面呈现 unavailable + cause，变更入口置灰；验证：用例断言不回退文件、不呈现文件派生列表
- [ ] 5.2 恢复后重新读库并清除 unavailable；验证：用例构造「库与文件不一致」，断言呈现的是库
- [ ] 5.3 复核项目面不再有任何文件派生路径；验证：`grep -rn "projects.json" sebas-webui/src/` 无生产代码命中

## 6. 退休路径变量与测试面

- [ ] 6.1 退休 `SEBAS_PROJECTS_PATH`（逻辑名由 `single-state-dir` 映射表接管）；验证：设置该变量后行为与不设置一致
- [ ] 6.2 改 Playwright 助手与旅程（`tests/testsuite-webui/tests/helpers/detached.ts` 等）：不再依赖 `projects.json`，改为经 API 断言；验证：Playwright 旅程全绿
- [ ] 6.3 改 `tests/support/mod.rs` 的路径钉；验证：`cargo test` 全绿
- [ ] 6.4 更新 `tasks.py` 与 `AGENTS.md`（`SEBAS_PROJECTS_PATH` 退休说明）；验证：沙箱仍能起、文档与实际一致

## 7. 全量回归

- [ ] 7.1 standalone 拓扑验收：起 core 与独立 webui，注册本地与远程项目、重启、读列表；验证：两种项目都在，且 `projects.json` 全程未被创建
- [ ] 7.2 跑 `invoke testsuite-e2e`；验证：全绿
- [ ] 7.3 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
