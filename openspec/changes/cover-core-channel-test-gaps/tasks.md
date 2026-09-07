# tasks — cover-core-channel-test-gaps

## 1. Reachability 三态化 + API kind 字段

- [ ] 1.1 在 `sebas-webui/src/session_backend.rs` 的 `Reachability` enum 增 `kind: ReachabilityKind`（`StartupFailed | AuthRejected | Disconnected | Connected`）+ `cause: Option<String>`；在 `sebas-webui/src/api.rs` `reachability_payload` 函数输出 `kind`（snake_case 字符串）；运行 `cargo test -p sebas-webui` 验证现有 webui 单测不破坏（验证：所有 backend 单测通过；`/api/summary` 路由测试覆盖 kind 字段）
- [ ] 1.2 在 `src/core_channel/client.rs` 的 `CoreChannelBackend::connect` ENOENT 分支读取 `SEBAS_STARTUP_ERROR_FILE`（env 变量）；文件不存在或 env 未设时 fallback 到 `core session channel socket not found at <path>`；运行 `cargo test --workspace` 验证（验证：新增 `startup_failed_with_env_file` / `startup_failed_fallback` 两个单测）
- [ ] 1.3 在 `src/core_channel/tests.rs` 新增 4 个单测：`reachability_startup_failed_with_env_file`、`reachability_startup_failed_fallback`、`reachability_auth_rejected_after_handshake`、`reachability_disconnected_after_connected`；运行 `cargo test -p sebas`（验证：4 个新单测全绿；既有 `client_converges_after_server_restart` 仍通过）

## 2. State 三件 + ensure_message + cross-uid

- [ ] 2.1 在 `src/core_channel/tests.rs` 新增 State 三件 contract test：`state_snapshot_returns_current`、`state_mutation_applies_change`、`state_mutation_rejected_does_not_silently_swallow`、`state_subscribe_delivers_mutations_after_snapshot`；channel 服务端注入 fake state-store engine（接口匹配）；运行 `cargo test -p sebas`（验证：4 个单测全绿；测试运行时间不显著增加）
- [ ] 2.2 在 `src/core_channel/tests.rs` 新增 `ensure_message_unknown_key_auto_creates`、`ensure_message_dormant_resumes`、`message_unknown_key_rejected` 三个单测；`Message` 在未知 key 上跑回归（验证已有 typed rejection 行为不变）；运行 `cargo test -p sebas`（验证：3 个新单测全绿）
- [ ] 2.3 在 `src/core_channel/tests.rs` 新增 `cross_uid_rejected_live_process` 单测：`#[cfg(unix)]` 守卫、`#[ignore]` 标记（需 root）；测试代码用 `nix::unistd::setuid` 切到 nobody/daemon 后发 Snapshot 请求；CI runner 默认 root 时跑 `cargo test -- --ignored`，本地无 root 跳过；运行 `cargo test -p sebas -- --ignored cross_uid`（验证：在 root 下单测通过；非 root 下 #[ignore] 不报错）

## 3. fake-claude perm 触发 + approval_answer e2e

- [ ] 3.1 扩展 `tests/bin/fake-claude.rs` 增加 "perm" 触发词：当 `session/prompt` 文本含 "perm" 时，fake-claude 走 `session/request_permission` 路径，发出 gated tool call 等候审批决定；运行 `cargo build --bin fake-claude` 验证编译通过（验证：构建成功；既有 fake-claude 触发词行为不变）
- [ ] 3.2 在 `tests/testsuite-webui/tests/permission.spec.ts` 重写 P1/P2/P3 用例：从真通道 ApprovalRequested 帧触发（替换现有 webui 后端 fake）；新增 `approval_allow_end_to_end`（fake-claude "perm" 触发 → review-card 出现 → 点 allow → transcript 呈现 allowed 工具结果）、`approval_deny_end_to_end`（同上，deny 路径）、`approval_unknown_rid_rejected`（POST `/api/permissions/<random-rid>/answer` 收到 4xx）；运行 `invoke testsuite-webui --case permission`（验证：3 个新用例全绿）
- [ ] 3.3 若 `wire-webui-sebas-agent-e2e` 任务 1.3（审批通道接线）尚未完成，本任务阻塞；阻塞解除后 3.2 落地；在 tasks.md 末尾注记该依赖（验证：阻塞解除状态明示）

## 4. set_session_model e2e + 账本

- [ ] 4.1 扩展 `tests/bin/fake-claude.rs` 增加多 model 反射：在 initialize 阶段 `session/configOptions.model` 返回 `["ok-model", "bad-model"]` 两个 model id；`set_config_option{model=ok-model}` 成功回 `ModelChanged`；`set_config_option{model=bad-model}` 回 `Error`（typed rejection）；运行 `cargo build --bin fake-claude` 验证（验证：构建成功；既有 single-model 行为不变）
- [ ] 4.2 在 `tests/testsuite-webui/tests/models.spec.ts` 新增 `set_session_model_happy_path`（fake-claude 启动 → spawn 会话 → PUT `/api/sessions/{key}/model` with `ok-model` → snapshot 同步 `current_model`）+ `set_session_model_rejects_unknown_model`（PUT with `bad-model` → 后端 typed rejection → webui 端呈现内联错误）；运行 `invoke testsuite-webui --case models`（验证：2 个新用例全绿；既有 `models.spec.ts` 用例不变）
- [ ] 4.3 更新 `tests/acceptance/COVERAGE.md`：在 `core-session-channel` 段追加本 change 索引（reachability 三态 + State 三件 + ensure_message + cross-uid + approval_answer e2e + set_session_model e2e 共 14 个新 case）；在 `testsuite-webui-browser` 段追加 spec 文件改写记录（permission.spec.ts 重写、models.spec.ts 新增）；运行 `openspec validate --changes --strict`（验证：validate 5 个 change 全过）

## 5. 验收：账本闭环

- [ ] 5.1 跑 `openspec status --change cover-core-channel-test-gaps --json` 验证四个 artifact 全部 `done`（验证：proposal/specs/design/tasks 状态均为 done；isPlanningComplete: true）
- [ ] 5.2 跑 `cargo test --workspace` + `pnpm --dir sebas-webui/frontend test` + `invoke testsuite-webui` 全量 3 连绿（验证：稳定性门槛一致；本期 14 个新 case 就位）
- [ ] 5.3 在 `tests/acceptance/COVERAGE.md` 段落末尾追加 `cover-core-channel-test-gaps` 一行指向本期 commit hash 与本 tasks（验证：账本自身可追溯）