# Tasks: add-acp-session-id-mapping

依赖前置：`add-opencode-acp` 已落地（握手信号 `handshake: (String, bool)`、ACP resume 机制、诚实回退）。本 change 在其上扩展映射。
明确决策：不做历史记录 id 回填；映射进现有 `MappingDto`。
**字段衔接（review-add-state-store R1）**：`MappingDto.acp_session_id` 是 `add-state-store` SQLite sessions 表同名列的来源；state-store 建表迁移必须含该列。本 change 先落，state-store 后收编。

## 1. 驱动层：上抛真实 ACP session id

- [x] 1.1 `handshake` 信号从 `(String, bool)` 扩展为 `(String, bool, Option<String>)`（路由 id、resumed、acp_session_id）；`SpawnOutcome` 增加 `acp_session_id: Option<String>`，`SessionMeta` 携带。fresh 路径上抛 `NewSessionResponse.session_id`，load 成功上抛加载的会话 id，回退 fresh 上抛新 `session/new` 的 id；Claude 驱动（`handshake=None`）恒 `None`。验证：`cargo test -p sebas-acp` 全绿（现有 47+5 不回归）。
- [x] 1.2 `DriverConfig` 增加 `load_session_id: Option<String>`；`SessionStart::Load` 从 `Load(String)` 改为 `Load { routing_id: String, acp_session_id: Option<String> }`；`AcpDriver` 的 `LoadSessionRequest` 目标优先用 `load_session_id`，`None` 时回退 routing_id（兼容无独立 id 的 agent / 旧记录）。同步 4-5 处调用点（session_boot/dispatch/测试）。验证：编译通过；`acp_resume_test` 五个场景仍全绿（None 路径行为不变）。
- [x] 1.3 新增 mock 场景：`load-ok` 时 load 用**提供的 acp_session_id**（mock 记录收到的 load id）→ 断言 driver 使用真实 id 而非 routing id。验证：新测试通过（mock 夹具能力扩展）。

## 2. 持久化映射

- [x] 2.1 `MappingDto` 增加 `#[serde(default)] acp_session_id: Option<String>`；`acp_spawn_and_activate` 成功后经 `router.activate` 写入；`acp_resume_and_activate` 按 `old_sid` 读映射取 `acp_session_id` 传给 `SessionStart::Load`（无则 `None`）。验证：旧 `state.json`（无字段）读为 `None` 不报错；新建 ACP 会话后 state 含 `acp_session_id`。
- [x] 2.2 单元/集成测试：建 ACP 会话 → 断言映射写入；手动改 state 去掉 `acp_session_id` → resume 回退 fresh（`resumed=false`）；load 失败 → 原映射保留在 state。验证：`cargo test` 相关用例通过。

## 3. 端到端 + 真实验收

- [x] 3.1 沙箱真实 opencode 验收（沿用 `/tmp` 沙箱配置与 `docs/acp-opencode-smoke.md`）：fresh 建会话（记录真实 ACP id）→ 优雅退出 dump state → 重启 → 对原 key 发消息触发 resume → 断言 `LoadSessionRequest` 用的是**映射的真实 id** 且不再报 `OpenCode service failure`；若 opencode 进程重启后其会话不可 load，如实记录（design Open Question）并确认坚强回退。验证：日志显示 load 用真实 id；resume 结果记录到 change 备注。

  **验收证据（2026-09-04，沙箱 `/tmp/sebas-itest`，webui port 9877）：**
  - `agent-kinds list`：`opencode true 1.18.25`。
  - fresh 建会话（`POST /api/sessions`，backend `acp:opencode`，`project_dir=/tmp/sebas-proj`）：`session/new` 返回真实 ACP id `ses_f9442373affeUr2hQiOnmaocgc`；首 prompt `PONG-PROBE` 正常回复。
  - 优雅 SIGTERM → `sessions.json` 含 `{"web-...":{"session_id":"76e7beea-...","last_active_unix":...,"acp_session_id":"ses_f9442373affeUr2hQiOnmaocgc"}}`（映射落盘，重启后 resume 可读）。
  - 重启 core → 对原 key `POST /api/sessions/<key>/message` 触发 resume → 日志 `sebas_acp::acp_driver: ACP session/load target resolved kind=opencode routing_id=76e7beea-857d-4dab-b6db-c0e889467d2e load_target=ses_f9442373affeUr2hQiOnmaocgc`；发出的 `session/load` 携带 `"sessionId": "ses_f9442373affeUr2hQiOnmaocgc"`（真实 id，非路由 uuid）。
  - resume 结果：`RESUME-PROBE` 正常回复；`starting a fresh session` 日志 0 次；无 `OpenCode service failure`；映射 `session_id` 仍为 `76e7beea-...`（routing id 保留，`resumed=true`）→ **resume 真正恢复会话**。
  - **R1 历史重放观察**：resume 后核心日志 0 条 `agent_message_chunk`（本次 resume 的 prompt 为单 turn 无工具调用，opencode 未重放历史文本块）——记录为 Open Question，留后续 change 观察。
  - **既有观察（非本 change 引入，与 add-opencode-acp 基线一致）**：webui resume 路径 `handle_spawn_resume_without_feishu` 未携带 `project_dir`，load 的 `cwd` 回落进程目录 `/data/workbench/repos-ai/sebas-agent`（`work_dir_for` 对 ACP agent 恒 `None`）。未在本 change 处理。

- [x] 3.2 全量回归：`cargo test --workspace`（容忍 pre-existing `permission_card_snapshot` 漂移，同 add-opencode-acp 备注）+ `cargo build` 无新 warning。验证：除既有快照漂移外全绿。

## 4. 收尾

- [x] 4.1 `openspec validate --changes` 通过（含 `acp-session-mapping` 新能力 + `acp-driver` delta）。

  **验证结果（2026-09-04）**：`openspec validate --changes` → `✓ change/add-acp-session-id-mapping`（连同 add-acp-model-selection / add-opencode-acp / add-state-store / review-add-state-store 共 5 项全过，0 failed）。

- [x] 4.2 把真实 opencode 冒烟证据（agent-kinds 输出、resume load 目标、回退记录）补进 `docs/acp-opencode-smoke.md` 或 change 备注；`add-state-store` 同步 SQLite 列的注意事项标注到其 tasks（不代改）。

  **完成情况**：resume 结论 + R1 观察已补进 `docs/acp-opencode-smoke.md`（"结果记录"节）；`add-state-store` 的 SQLite `sessions` 表已含 `acp_session_id`（nullable）列契约（review-add-state-store 修订版），其 tasks 3.x 已标注"ACP 三 change 先落、state-store 后收编"（不代改）。
