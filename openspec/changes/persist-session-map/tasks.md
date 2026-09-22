## 1. session_map 按目标形状重建

- [ ] 1.1 `session_map` 的注册 DDL 与 `SessionMapRow` 按映射完整形状重建（含 `NOT NULL` 列与默认值），主键仍为 `(chat_id, thread_id)`；验证：`cargo build` 通过，且表结构与 `MappingDto` 字段有一张对照表（PR 描述）
- [ ] 1.2 `SessionMapRow` 与 `MappingDto` **合一**，删掉并行形状与其转换；验证：`grep -rn "MappingDto" sebas-dispatch/src/` 只剩一处定义，且未更新的构造点已全部编译修复
- [ ] 1.3 既有 5 列（`chat_id` / `thread_id` / `session_id` / `last_active_unix` / `project_dir`）与主键语义不变；验证：`pragma_table_info('session_map')` 含这些列且主键与改造前一致
- [ ] 1.4 旧库在形状变化后走重置路径并隔离旧文件；验证：用改造前的库启动，断言日志给出隔离路径、隔离文件可打开并含旧行（依赖 `quarantine-database-reset`；若该 change 未落地，则断言重置发生且日志如实说明）

## 2. 按变更持久化

- [ ] 2.1 `load_session_map` / `save_session_map` 接上 `DbStateEngine`，成为生产路径；验证：`cargo test -p sebas` 全绿，且这两个函数不再是「仅测试被调用」
- [ ] 2.2 会话创建 / 模型变更 / 模式变更 / 标签变更 / 关闭各触发一次 `entry.save(&store)`（经单写 actor 闭包，不新增线程或文件）；验证：单测断言每次事件后库中映射与内存一致，且落库路径无手写 SQL
- [ ] 2.3 **核心收益用例**：会话创建且对客户端可见后立即 SIGKILL，重启后断言映射仍在（含 session_id 与 desired_mode）；验证：新增集成测试通过；并对照旧实现下同一用例会失败（PR 描述附旧行为）
- [ ] 2.4 确认未提交的变更不会落库（对齐 `state-store`「Mutation durability」：响应返回即已提交）；验证：单测断言响应前库中已可见

## 3. 退休关停快照与配置键

- [ ] 3.1 删 `src/run.rs` 的关停 dump（`run.rs:590-601`）；验证：关停路径不再写文件，且 2.3 的用例仍通过
- [ ] 3.2 删 `src/session_boot.rs` 的文件读取与 `<path>.corrupt-<unix>` 隔离；验证：`grep -rn "corrupt-" src/session_boot.rs` 无输出
- [ ] 3.3 删除 `[dispatch] state_file` 配置键（不留过渡期）；验证：带该键的配置文件启动时报**未知键**错误（这是预期），不带该键时正常启动
- [ ] 3.4 `tasks.py` 沙箱菜谱与 `AGENTS.md` 同步移除该键与相关「必配」说明；验证：沙箱仍能起，且 `AGENTS.md` 不再要求配置该键

## 4. 测试与文档

- [ ] 4.1 改 `tests/restart_recovery_test.rs`：把「损坏文件被隔离」改写为「映射条目不可读时启动为空表且不阻塞；库损坏时按状态库规则拒绝启动」；验证：改写后的测试通过，且不删掉原有覆盖点
- [ ] 4.2 改 `tests/sigterm_cleanup_test.rs`：改为断言 SIGTERM/SIGKILL 后库中映射完整；验证：两个信号各一个用例通过
- [ ] 4.3 改 `tests/support/mod.rs` 的路径钉（移除 sessions.json 相关钉）；验证：`cargo test` 全绿
- [ ] 4.4 复核无遗留导入代码：验证：`grep -rn "sessions.json" src/ sebas-*/src/` 无生产代码命中；且无导入标记相关的键
- [ ] 4.5 更新 `openspec/specs/session-persistence/spec.md` 的 Purpose（去掉「being migrated」的进行时表述）；验证：Purpose 与两条 spec 的新要求一致

## 5. 全量回归

- [ ] 5.1 跑 `invoke testsuite-e2e`；验证：全绿
- [ ] 5.2 跑 `invoke testsuite-acceptance`；验证：全绿，出现红则回到对应步定位
- [ ] 5.3 沙箱重启旅程复核：按 AGENTS.md 菜谱起 core，创建会话，SIGTERM 后再起 core；验证：会话列表与会话状态按既有 rest 恢复语义恢复，且 `sessions.json` 全程未被创建
