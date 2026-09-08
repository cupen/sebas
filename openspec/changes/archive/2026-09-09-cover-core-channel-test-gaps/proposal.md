## Why

上次梳理得出 webui→core 通信面有三块真未测的洞：(a) `reachability()` 不区分 startup failure vs runtime down——`fail-fast-on-startup-errors` change 已写 spec 规约（"core session channel 启动失败时 banner 显示 startup failure cause"），但 channel 单测 + Playwright e2e 都没补；(b) `approval_answer()` 走 detached webui 的端到端闭环——COVERAGE.md 缺口 1 已声明属 `wire-webui-sebas-agent-e2e` 任务 1.3 的遗留（进程内审批有，detached 通道没有）；(c) `set_session_model()` webui 端正向切换 e2e 缺——`models.spec.ts` 只测了"无模型会话终态杀伤"。本期专门给这三个洞补测，并把 cross-uid 拒绝、StateSnapshot/Mutation/Subscribe、`ensure_message` IM 路径几条次要洞一并补齐。

## What Changes

- **新增（核心一）**：`reachability()` startup-failure 区分测试——`Reachability` 三变体枚举 + `/api/summary.reachability.kind` 字段（owner 归本 change；fail-fast 已落地的 cause 字符串富化沿用不改）；单测覆盖 channel client 拿到 socket-not-exist 与 socket-but-core-dead 两种情形返回不同 `Reachability`；Playwright 覆盖 webui banner 按 kind 区分（startup_failed 显示 "core startup failed: <原因>" 全串，disconnected 显示 "core is not connected"）。
- **新增（核心二）**：`approval_answer()` detached 拓扑端到端闭环测试——单进程真通道路径已有（permission.spec P1/P2/P3 全绿为基线，不重写），本期只补双进程 detached 变体：fake-claude 触发 gated tool call → core 推到 channel `ApprovalRequested` 帧 → 跨进程到达 webui 渲染 review-card → 用户点 allow/deny → 走 channel `ApprovalAnswer` 回 core → core 把结果送到 acp 子进程 → 子进程以允许/拒绝语义继续执行；另补 `unknown_rid` typed rejection。harness 复用 harden 5.4 的双进程装配。
- **新增（核心三）**：`set_session_model()` webui 正向切换 e2e——fake-claude 启动后 PUT `/api/sessions/{key}/model` 切换到另一 model、断言 `ModelChanged` 事件出现、`current_model` 同步；切到不存在 model 时 typed rejection 端到端呈现。
- **新增（次要）**：`ensure_message()` IM 路径 channel 单测——IM 投递语义（未知 key 自动建会话 + dormant 懒复活）；cross-uid 拒绝 live process 单测（启动两个不同 uid 进程，第二个发请求被拒）；`StateSnapshot/StateMutation/StateSubscribe` 三件 contract test（channel 转发到 state-store engine 路径）。
- **修改**：`tests/acceptance/COVERAGE.md` 把本期用例补进 `core-session-channel` 与 `testsuite-webui-browser` 段。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `core-session-channel`: 「Honest degradation when the core is unreachable」 requirement 补充 startup-failure 区分的 scenario 与 cross-uid 拒绝的 live process 覆盖；「Channel transport and authentication」 requirement 补充 live-process 的跨 uid 拒绝 scenario；新增「State store channel surface」 requirement（StateSnapshot/Mutation/Subscribe 路径）。
- `webui`: 「Session backend seam」 requirement 补充 startup-failure banner 区分 + approval_answer 端到端 + set_session_model 正向 e2e 三类 scenario。

## Impact

- 受影响测试：
  - `src/core_channel/tests.rs`（A 批 12 个：reachability 三态 4、State 三件 4、ensure_message/Message 3、cross-uid 1）。
  - `tests/testsuite-webui/tests/session-mgmt.spec.ts`（补 startup-failure banner 旅程，断言 kind=startup_failed）。
  - 双进程沙箱（复用 harden 5.4 fixture）：detached approval allow/deny + unknown_rid（B1）。
  - `tests/testsuite-webui/tests/models.spec.ts`（补正向 set_session_model 切换旅程，B2）。
- 受影响 production 代码（极小）：
  - `sebas-webui/src/session_backend.rs`：`CoreChannelBackend` 的 `connect` 区分 startup-failure 与 runtime-down；`/api/summary` 的 `reachability.cause` 字段扩展（如尚未在 fail-fast change 落地上实现）。
  - `src/core_channel/client.rs`：connect 失败时把 errno 与上下文打包进 `Reachability`。
- 受影响 spec：`openspec/specs/core-session-channel/spec.md`（MODIFIED 2 + ADDED 1）、`openspec/specs/webui/spec.md`（MODIFIED 1 条 `Session backend seam`）。
- 不影响：CI workflow、vitest 单测已有 happy-path、router / feishu / im 路径（IM 端 `ensure_message` 单测加在 channel 层，不动 IM 现有覆盖）。

## Non-goals

- 不改 fail-fast 已落地的 cause 富化行为（无条件 enrich + 闩锁语义）——本 change 只加 kind 判别器。
- 不重写 permission.spec 既有 P1/P2/P3（单进程真通道基线已绿）。
- 不为 macOS 单独补 channel 测试（macOS 不是 release target）。
- 不补 `StateSnapshot/Mutation/Subscribe` 的端到端 Playwright（state-store 自身有覆盖，本期只在 channel 单测里覆盖转发路径）。
- 不改 `fail-fast-on-startup-errors` 的既有规约（仅补其未覆盖测试用例）。
- 不重写 `wire-webui-sebas-agent-e2e` change 自身的 tasks（仅把其中 1.3 "approval_answer e2e" 的测试子集移到本 change 完成，避免双 change 重复打账）。
- 不动 im-service 的 `ensure_message` 在 im 侧的覆盖（仅补 channel 层单测）。
- 不为 channel 加新 API（仅补测 + 把现成区分逻辑做对）。