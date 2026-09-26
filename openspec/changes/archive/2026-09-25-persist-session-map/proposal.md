# Proposal: persist-session-map

## Why

会话映射今天走的是**最脆的一条路**：`sessions.json` 只在关停时用截断式 `std::fs::write` 写一次（`src/run.rs:590-601`），**无 fsync、无 chmod、非原子**。进程被 SIGKILL，全部会话映射丢失——不是「丢最后几笔」，是整份丢。

而 spec **已经要求了正确的行为**：`session-persistence`「Runtime state is not persisted by this store」明文写着「The agent session map SHALL be persisted in the state store and written per mutation, rather than only at daemon shutdown」，并带场景「Session map survives unclean exit — reflects the last committed state, not the last graceful shutdown」。同时 `state-store`「Runtime state boundaries」却仍在描述旧的关停快照为权威、`session_map` 表是「reserved placeholder」。**两条 spec 互相矛盾**，实现站在旧的那一边。

迁移的落点是现成的：`session_map` 表已注册（`repo.rs:604-615`），`load_session_map` / `save_session_map`（`repo.rs:468-514`）已写好且**没有任何生产调用方**。表里只有 5 列（`chat_id` / `thread_id` / `session_id` / `last_active_unix` / `project_dir`），而映射实际携带约 11 个字段（`acp_session_id` / `current_model` / `desired_mode` / `pending_*` / `label` / `prompt_preview` / `awaiting_first_prompt` 等）。

## What Changes

- **会话映射写入状态库，按变更即落盘**：替换关停快照，获得既有 spec 已要求的 per-mutation 持久性；SIGKILL 只丢未提交的那一笔。
- **`session_map` 表按目标形状重建**（承载映射的全部字段），并落为 ActiveRecord struct：`SessionMapEntry` 一行即一个实例，`entry.save(&store)` 即落盘（机制见 `extract-sebas-db`）；不再受「只能加列」的束缚——产品尚未发布，旧库被重置一次即可。落点是 `projects.db`（增长的用户数据；分层规则见 `single-state-dir`）。
- **退休关停快照路径**：删 `[dispatch] state_file` 配置键、关停时的 dump、以及 `src/session_boot.rs` 的读取与 `<path>.corrupt-<unix>` 隔离（库的损坏由 `state-store` 的损坏要求管辖）。
- **不做遗留导入**：无发布版，没有需要搬运的既有安装；旧 `sessions.json` 不再被读取，文件留在盘上可随手删除。
- **消除两条 spec 的矛盾**：`state-store` 的「Runtime state boundaries」改为指向 `session-lifecycle`/`session-persistence` 已在要求的行为；`session-lifecycle` 的恢复要求改为「从状态库恢复、按变更落盘、绝不因会话映射自身而拒绝启动」。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

- `state-store`: 「Runtime state boundaries for persisted session state」更新——删除「关停快照为权威 / 会话表是预留占位 / 迁移是延后步骤」的表述，改为「按变更持久化、由 session-persistence 与 session-lifecycle 管辖行为」，其余（许可名单不持久化、spawn 占位不落盘）不变。
- `session-lifecycle`: 「Restart recovery with corruption tolerance」更新——恢复来源由文件改为状态库，持久性由关停快照改为按变更落盘，损坏语义与状态库的损坏规则对齐（不可打开的库按状态库规则拒绝启动；会话映射本身不可读绝不阻止启动）。

## Impact

- **改动**：`src/sebas_state/repo.rs`（`SessionMapRow` 重建为完整形状、读写接上引擎）、`src/sebas_state/engine.rs`（映射方法接线）、`src/run.rs`（删关停 dump）、`src/session_boot.rs`（删文件读取与隔离）、`sebas-dispatch/src/state.rs`（`MappingDto` 与行结构合一）、`src/config.rs`（删除 `[dispatch] state_file` 键）、`tasks.py` / `AGENTS.md`（沙箱菜谱）。
- **测试面**：`tests/restart_recovery_test.rs`（今天写入并断言 `sessions.json.corrupt-*`）、`tests/sigterm_cleanup_test.rs`（预置并断言重序列化）、`tests/support/mod.rs`（路径钉）。
- **schema**：`session_map` 重建（含 NOT NULL 列与默认值）。无发布版，故不要求加列兼容——开发机上旧库被重置一次即可；重置会隔离旧文件保留痕迹（见 `quarantine-database-reset`）。
- **验收**：新增「SIGKILL 后映射完整」用例（本 change 的核心收益）+ 既有 `restart_recovery_test` / `sigterm_cleanup_test` 改写后全绿 + `invoke testsuite-e2e` / `testsuite-acceptance` 全绿。

## Non-goals

- **不改会话映射的语义**（哪些字段、Dormant 恢复、pending 语义一律不动）；本 change 只换存储与持久性。
- **不做遗留导入**、不保留旧 `sessions.json` 的可用性——无发布版，没有需要搬的数据。
- **不为 `[dispatch] state_file` 保留过渡期**：直接删键。配置里残留该键会以未知键报错——无发布版时这比留一个「能解析但不生效」的假键更诚实。
- **不动 `projects.json` / `archive.json`**——属 `migrate-project-registry` 与其后续。
- **不为会话映射引入保留期或清理**：会话生命周期由既有 close/terminate 语义管理。
- **不做加列兼容设计**：不为了「旧库能直接升级」把新列都做成可空。
