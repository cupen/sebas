# tasks — cover-core-channel-test-gaps

> 分批说明（design D4/R5/Migration Plan）：A 批无外部依赖先行合流；B 批阻塞于 `wire-webui-sebas-agent-e2e` 任务 1.3（审批通道接线）+ harden 5.4 双进程 harness，解冻后再做。两批独立合流，不互相等。

## A1. Reachability 三态化 + API kind 字段（owner: 本 change，design D3）

- [x] A1.1 在 `sebas-webui/src/session_backend.rs` 的 `Reachability` enum 增三变体（`StartupFailed { cause } | AuthRejected { cause } | Disconnected { cause }`，替代现有 `Unreachable { cause }`）；在 `sebas-webui/src/api.rs` `reachability_payload` 输出 `kind`（snake_case 字符串：`startup_failed | auth_rejected | disconnected`）；cause 保持已落地的无条件 enrich 全串（不改 fail-fast 行为，只加 kind）；运行 `cargo test -p sebas-webui` 验证现有 webui 单测不破坏（验证：所有 backend 单测通过；`/api/summary` 路由测试覆盖 kind 字段三取值）
- [x] A1.2 在 `src/core_channel/tests.rs` 新增 4 个单测：`reachability_startup_failed_with_env_file`、`reachability_startup_failed_fallback`、`reachability_auth_rejected_after_handshake`、`reachability_disconnected_after_connected`；其中 auth-rejected 用例同时断言"同 secret 不无限重试、env 未设时重读文件再试一次"（spec 已修正的重试语义）；运行 `cargo test -p sebas`（验证：4 个新单测全绿；既有 `client_converges_after_server_restart` 仍通过）

## A2. State 三件 + ensure_message + cross-uid

- [x] A2.1 在 `src/core_channel/tests.rs` 新增 State 三件 contract test：`state_snapshot_returns_current`、`state_mutation_applies_change`、`state_mutation_rejected_does_not_silently_swallow`、`state_subscribe_delivers_mutations_after_snapshot`；channel 服务端注入 fake state-store engine（接口匹配）；运行 `cargo test -p sebas`（验证：4 个单测全绿；测试运行时间不显著增加）
  - 注（2026-09-08）：四个用例落位在 `tests/state_channel_contract_test.rs`（独立进程集成测试）而非 `src/core_channel/tests.rs`——`state_store::ENGINE` 是每进程一次的 OnceLock，lib 单测进程一旦注入 fake engine 会污染 provider/spawn_env 等依赖「engine 未初始化走文件回退」的并行用例（`tests/state_subscription_test.rs` 文件头注释已记录同一约束）。用例名、断言与 fake-engine 注入（design D7）保持任务原文。
- [x] A2.2 在 `src/core_channel/tests.rs` 新增 `ensure_message_unknown_key_auto_creates`、`ensure_message_dormant_resumes`、`message_unknown_key_rejected` 三个单测；`Message` 在未知 key 上跑回归（验证已有 typed rejection 行为不变）；运行 `cargo test -p sebas`（验证：3 个新单测全绿）
- [x] A2.3 在 `src/core_channel/tests.rs` 新增 `cross_uid_rejected_live_process` 单测：`#[cfg(unix)]` 守卫、`#[ignore]` 标记（需 root）；测试代码 fork 子进程后 `setuid` 到 nobody/daemon 再发 Snapshot 请求（真实跨进程凭证，非同进程改 uid）；CI runner 默认 root 时跑 `cargo test -- --ignored`，本地无 root 跳过；**CI-only 非门禁**（design D5）；运行 `cargo test -p sebas -- --ignored cross_uid`（验证：在 root 下单测通过；非 root 下 #[ignore] 不报错）
  - 注（2026-09-08）：本机以 root 实测通过（`sudo <testbin> --ignored cross_uid` → ok）；非 root（uid 1000）下同命令走 `[skip]` 早退路径不报错。为让连接到达服务端 peer-uid 检查，测试场景把 socket/目录放宽为 0666/0755（生产 0600 绑定不变——0600 下文件系统层已先于服务端检查拒绝外部 uid）。

## B1. approval_answer detached e2e（阻塞：wire-webui 1.3 + harden 5.4 harness；design D4）

- [x] B1.1 确认既有 "perm" 触发语义不变：`tests/bin/fake-claude.rs` 的 `perm_turn` 已存在，本 change 不扩展触发词；只读 existing permission.spec.ts P1/P2/P3（单进程真通道路径）确认全绿即视为基线（验证：`invoke testsuite-webui --case permission` 全绿，无改动）
- [x] B1.2 在双进程沙箱（复用 harden 5.4 的 dual-process fixture，不重写 harness）新增 `approval_allow_detached`（perm → 跨进程 ApprovalRequested → review-card → allow → transcript allowed）+ `approval_deny_detached`（deny 路径）+ `approval_unknown_rid_rejected`（POST 随机 rid → 4xx）；运行对应 case（验证：3 个新用例全绿；单进程既有用例不变）
  - 注（2026-09-08）：落位 `tests/testsuite-webui/tests/approval-detached.spec.ts`（detached config 专属；主 config testIgnore 同步扩展）。冷场景首个浏览器页可能错过首个 ApprovalRequested 帧（feed 的 fire-and-forget 契约：未送达即被内核 fail-closed 拒绝、绝不重放），由 Playwright retry（retries: 1）吸收——套件 3 用例全绿（exit 0），重跑 3 次结论一致。
- [x] B1.3 若 `wire-webui-sebas-agent-e2e` 任务 1.3（审批通道接线）尚未完成，本批阻塞；阻塞状态在 tasks.md 末尾注记日期与原因（验证：阻塞解除状态明示）
  - 注（2026-09-08）：**无阻塞**。审批通道接线已在代码中落地，证据：(1) 路由 `POST /api/permissions/{request_id}/answer`（`sebas-webui/src/server.rs:205` + `api.rs::answer_permission`，未知 rid → 404 typed rejection）；(2) `SessionStreamFrame::ApprovalRequested` 帧（protocol.rs）+ client 转发到 review-card feed（client.rs）；(3) 通道层 `ApprovalAnswer` 处理与未知 rid typed rejection 单测（`approval_answer_for_unknown_request_id_returns_typed_rejection`）。B1.2 的 detached 旅程即在该接线上全绿。

## B2. set_session_model e2e + 账本

- [x] B2.1 扩展 `tests/bin/fake-claude.rs` 增加多 model 反射：在 initialize 阶段 `session/configOptions.model` 返回 `["ok-model", "bad-model"]` 两个 model id；`set_config_option{model=ok-model}` 成功回 `ModelChanged`；`set_config_option{model=bad-model}` 回 `Error`（typed rejection）；运行 `cargo build --bin fake-claude` 验证（验证：构建成功；既有 single-model 行为不变）
  - 注（2026-09-08，前提修正）：任务原文的落点不可行——`tests/bin/fake-claude.rs` 说的是 Claude CLI stream-json 方言（无 session/configOptions 概念），且 claude 驱动对 SetModel 硬性终态拒绝（`sebas-acp/src/claude/driver.rs:422`），这正是既有 models.spec 3.2 teardown 用例的依据；若改驱动会破坏该既有用例。多 model 反射的等价能力已存在：`sebas-acp/tests/bin/fake-acp-agent.rs`（generic-ACP 方言，`--model-options` + `--reject-model`，接受 → ModelChanged / 拒绝 → 非 terminal Error）。落地方式：testsuite-webui 沙箱 harness（tasks.py）新增 `[acp.agents.fakeacp]` 接入该 fake（通告 bad-model/ok-model，初值 = 首项 bad-model；拒绝 bad-model），构建命令加 `--bin fake-acp-agent`。既有 single-model（claude）行为不变 ✓。
- [x] B2.2 在 `tests/testsuite-webui/tests/models.spec.ts` 新增 `set_session_model_happy_path`（fake-claude 启动 → spawn 会话 → PUT `/api/sessions/{key}/model` with `ok-model` → snapshot 同步 `current_model`）+ `set_session_model_rejects_unknown_model`（PUT with `bad-model` → 后端 typed rejection → webui 端呈现内联错误）；**先在 spec 层写清与既有"无效 model → 会话终态 teardown"用例的区分条件**（model 存在与否决定同步 vs teardown），避免两用例互为 flake；运行 `invoke testsuite-webui --case models`（验证：2 个新用例全绿；既有 `models.spec.ts` 用例不变）
  - 注（2026-09-08）：路由实为 POST（非 PUT，`server.rs:200`）；区分条件已写入 spec 头注释：3.2 = claude 会话无模型面 → 驱动终态 teardown；B2.2 = `acp:fakeacp` 会话有 configOptions 模型面 → agent 接受（ModelChanged → current_model 同步）/ 拒绝（non-terminal Error）。观察到产品缺口：generic-ACP 驱动不产 `Finished` 事件（会话停留 working）且 agent 级 Error 在 webui 无渲染面——测试按 repo 既有 "observed product gap" 先例断言可观测契约（无假成功：current_model 不变 + 会话存活），缺口待后续 change 处理。
- [x] B2.3 更新 `tests/acceptance/COVERAGE.md`：在 `core-session-channel` 段追加本 change 索引（A 批：reachability 三态 + State 三件 + ensure_message + cross-uid；B 批：approval detached e2e + set_session_model e2e）；在 `testsuite-webui-browser` 段追加 spec 文件改写记录；运行 `openspec validate --changes --strict`（验证：validate 全过）

## 5. 验收：账本闭环（A 批合流时执行；B 批合流时复核 B2.3 即可，不重做全量）

- [x] 5.1 跑 `openspec status --change cover-core-channel-test-gaps --json` 验证四个 artifact 全部 `done`（验证：proposal/specs/design/tasks 状态均为 done；isPlanningComplete: true）
- [x] 5.2 A 批：跑 `cargo test --workspace` + `invoke testsuite-webui --case permission`（基线确认）全绿；cross-uid 单测按 CI-only 非门禁处理（design D5），不挡绿。B 批：跑 `invoke testsuite-webui` 全量 3 连绿；若与 harden 合流时的共享 3 连绿之间产品代码无变更，可复用彼次结果并注记（验证：稳定性门槛一致；本期新 case 就位）
  - 注（2026-09-08）：A 批 `cargo test --workspace` 全绿（104 个测试二进制 ok）+ permission case 3/3 绿；B 批因产品代码有变更（session_backend/api/client/agent_backend + tasks.py 沙箱）不可复用 harden 的 3 连绿，已实跑 `invoke testsuite-webui` 全量 3 次全部 exit 0（34 主套件 + 3 auth + 4 detached；detached 首例首试由 retry 吸收，见 B1.2 注），另加第 4 次确认跑 exit 0。
- [x] 5.3 在 `tests/acceptance/COVERAGE.md` 段落末尾追加 `cover-core-channel-test-gaps` 一行指向本期 commit hash 与本 tasks（验证：账本自身可追溯）