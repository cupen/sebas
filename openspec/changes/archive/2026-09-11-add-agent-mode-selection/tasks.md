# Tasks: add-agent-mode-selection

## 1. 后端 wire 与映射

- [x] 1.1 `sebas-webui/src/api.rs`：`CreateSessionRequest` 加 `mode: Option<String>` + 词汇校验（ask/edit/allow/auto，未知值 400），thread 到 `spawn_with` / `create_placeholder`；验证：`cargo check -p sebas-webui` 通过
- [x] 1.2 `sebas-webui/src/session_backend.rs`：`SessionBackend` trait 的 `spawn_with`/`create_placeholder` 加 `mode` 参数，进程内实现与默认实现同步更新（远端节点在进程内后端仍如实拒绝）；验证：`cargo check -p sebas-webui`
- [x] 1.3 `src/core_channel/protocol.rs` + `client.rs`：`CoreChannelRequest::Spawn` 加 `mode` 字段（含占位会话 mapping 记住 mode、首条消息 spawn 带出）；核对 protocol serde 策略对新旧帧的兼容性；验证：`cargo check -p sebas`，`cargo test -p sebas --lib core_channel` 通过
- [x] 1.4 `src/core_channel/server.rs`：core 侧接收 mode 并传递——本机走 `backend.spawn_with(..., mode, ...)`，`spawn_remote` 把 mode 传给 `projection.spawn_on`（替换硬编码 `None`）；验证：`cargo test -p sebas --lib core_channel`
- [x] 1.5 `sebas-dispatch/src/engine/mod.rs`：`Out::WebSpawn` 与 `web_spawn`（含占位路径第二处 emit）加 `mode`；同步修复精确解构该事件的既有测试（`spawn_race_test.rs`、`session_endpoints_test.rs`、`core_channel/tests.rs`）；验证：`cargo test -p sebas-dispatch -p sebas-webui`
- [x] 1.6 `src/dispatch.rs`：`handle_web_spawn` 中 kind=claude 时按映射表追加 `--permission-mode`（ask/缺省不传、edit→acceptEdits、allow/auto→bypassPermissions），映射函数单一出处；其它 kind 静默忽略（非致命）；验证：单元测试断言 argv 组装（含四种 mode 输入）

## 2. 中途切换与 driver

- [x] 2.1 `sebas-acp/src/claude/driver.rs`：启动 argv 应用 `--permission-mode`（映射后），记录 `current_permission_mode`；存活探针改发当前 mode 而非硬编码 Default；验证：`cargo test -p sebas-acp`（含 fake-claude-cli 集成用例）
- [x] 2.2 `sebas-acp/src/session.rs` + driver：新增 `AcpCommand::SetMode` 与 `AcpEvent::ModeChanged { mode }`，运行时切换经 SDK `set_permission_mode`，控制面词汇→SDK 枚举映射（ask→Default、edit→AcceptEdits、allow/auto→BypassPermissions）；失败发非致命 Error 事件；验证：`cargo test -p sebas-acp`
- [x] 2.3 webui 新增 `POST /api/sessions/{key}/mode`（`SetModeRequest { mode }`，同创建词汇校验）：本机走 `AcpCommand::SetMode`，远端走投影 set_mode（`SessionOp::SetMode` 控制面入口）；快照 mode 字段（desired 立即更新、effective 随 ModeChanged 落定）；验证：`sebas-webui/tests/session_endpoints_test.rs` 新增 happy/typed-rejection 用例通过
- [x] 2.4 占位/恢复路径的 mode 状态：resume 与重启恢复的会话从 mapping 读回 desired mode 初始化 `current_permission_mode`；验证：`cargo test -p sebas` 会话恢复相关单测

## 3. 前端

- [x] 3.1 `sebas-webui/frontend/src/api/client.ts`：`createSession` 加 `mode` 参数；新增 `setSessionMode(key, mode)`；session 快照类型加 mode 字段；验证：`rtk tsc`（或前端 typecheck）通过
- [x] 3.2 `workbench-composer.ts`：创建表单加 mode `wa-select`（默认"agent 默认"= 不发送），提交时随 createSession 传出的 wire 字段；验证：`invoke testsuite-webui --case spec`（相关 spec 更新后）通过
- [x] 3.3 会话头部 mode 展示与切换：扩展既有 mode-tag 渲染（本机/远端通用），加切换下拉，失败走既有非致命错误呈现；验证：Playwright 用例（`testsuite-webui`）断言切换后 mode 标签更新

## 4. fake 层与测试强化（全零 token）

- [x] 4.1 `tests/bin/fake-claude.rs`：`--permission-mode bypassPermissions` 时 `perm` 场景跳过 hook_callback 直接放行；journal 记录运行时 mode 切换（`{"type":"mode_change",...}`）与 argv 中的 `--permission-mode`；运行时 set_permission_mode 回 success 并更新 fake 内部 mode；验证：`cargo build`（fake-claude 随 workspace bin 构建）+ `sebas-acp` fake 集成用例
- [x] 4.2 `tests/testsuite_e2e_test.rs` 新增 `mode_threads_to_agent_argv`（带/不带 mode 建会话，读 journal 断言 argv）与 `mode_mid_session_switch`（切换 journal 记录 + 快照更新 + 未知 mode 400）；验证：`invoke testsuite-e2e --case mode_threads_to_agent_argv`、`--case mode_mid_session_switch` 通过
- [x] 4.3 `tests/testsuite_acceptance_test.rs` 新增远端节点 mode 旅程（EchoBody 桩）：mode=allow 创建后 `run:` 免门控直接执行、中途切回 ask 恢复 waiting、离线拒绝点名节点；验证：`invoke testsuite-acceptance --case remote_node_mode_journey` 通过
- [x] 4.4 更新 `tests/acceptance/COVERAGE.md` 矩阵（新增旅程与 requirement 映射）与相关 Playwright spec 文件；验证：`invoke testsuite-webui --case spec` 结构门禁通过

## 5. 收尾

- [x] 5.1 全量回归：`rtk cargo test`（workspace 单测）+ `invoke testsuite-e2e` + `invoke testsuite-acceptance` 全绿；确认全程无真模型调用
- [x] 5.2 沙箱手工验证（AGENTS.md 调试菜谱）：带 mode 创建→perm 场景免审批→切换→优雅退出清理；`openspec validate add-agent-mode-selection --strict` 通过
