## Context

`fail-fast-on-startup-errors` 与 `wire-webui-sebas-agent-e2e` 两个 change 已分别在 spec 层补了 startup-failure 区分、approval_answer e2e 规约，但实现与测试尚未闭环：
- `CoreChannelBackend::connect` 失败仅返回 generic error，`/api/summary.reachability.kind` 字段尚未存在——webui banner 无法区分 startup-failed / auth-rejected / disconnected。
- `wire-webui-sebas-agent-e2e` 任务 1.3 留下的 COVERAGE 缺口 1："审批事件经核心通道推送到 detached webui 的接线属进行中的 `wire-webui-sebas-agent-e2e` 任务 1.3；落地后补 `allow / deny` 两条旅程"——这是已知的 bug，进程内审批有，detached 端到端没测过。
- `set_session_model` webui 正向 e2e 缺；`StateSnapshot/Mutation/Subscribe` 三件未覆盖；cross-uid 拒绝 live process 未跑过（`src/core_channel/tests.rs:3-4` 注释已自陈）。

本期只补测、不动 production API 形状（`/api/summary.reachability.kind` 字段如尚未在 `fail-fast-on-startup-errors` 落地期间实现，本 change 一起带上，作为最小 production 改动）。

## Goals / Non-Goals

**Goals:**
- channel client 端 `Reachability` 三类区分（startup_failed / auth_rejected / disconnected）+ `SEBAS_STARTUP_ERROR_FILE` 读取。
- `/api/summary.reachability.kind` 字段暴露，让 webui banner 能按 kind 渲染。
- approval_answer end-to-end：fake-claude 触发 gated tool call → channel ApprovalRequested → webui review-card → POST answer → channel ApprovalAnswer → acp 子进程 allow/deny 语义。
- set_session_model webui 正向 e2e + unknown model typed rejection。
- cross-uid live process 拒绝测试（不依赖 setuid hack）。
- StateSnapshot/Mutation/Subscribe 三件 contract test。
- ensure_message IM 投递语义 channel 单测。

**Non-Goals:**
- 不动 channel wire protocol。
- 不补 macOS 单独测试。
- 不动 `fail-fast-on-startup-errors` 已写但未实现的部分（仅与之并轨补测；如有 production 改动遗留，本 change 一起带上）。
- 不补 record/replay 通道（spec 单独列在 COVERAGE 缺口 4）。
- 不为 state-store 引擎自身加测试（已有覆盖）。

## Decisions

### D1：`Reachability` 三态区分用枚举而非字符串
- 决策：`Reachability` 增 `kind: StartupFailed | AuthRejected | Disconnected | Connected` 字段；cause 字段为 `Option<String>`。`/api/summary` JSON 输出新增 `reachability.kind`（snake_case 字符串）；前端根据 kind 渲染 banner 文案。
- 依据：枚举让 webui 不会因 cause 文案变化误判；snake_case 字符串与现有 JSON 风格一致；schema 演进向后兼容（缺 kind 字段时前端按 `disconnected` 处理）。
- 备选：仅靠 cause 文案字符串匹配 → 否决：脆弱、文案改动即破前端。

### D2：`SEBAS_STARTUP_ERROR_FILE` 在 connect 失败 ENOENT 分支读取，避免被运行时误读
- 决策：`CoreChannelBackend::connect` 看到 socket 路径 ENOENT 时读取 `SEBAS_STARTUP_ERROR_FILE`（若 env 设置且文件存在）；不在每次重连时都读——避免 core 已 ready 但 socket 被回收时反复读过期文件。
- 依据：沙箱联调契约已经把 `SEBAS_STARTUP_ERROR_FILE` 作为启动失败摘要载体；客户端读取发生在「socket 从未出现过」这条事实成立时。
- 备选：每次 connect 都读 → 否决：会让已正常运行的实例在 socket 偶尔断连时被误判为 startup-failure。

### D3：`/api/summary.reachability.kind` 字段如未实现则本 change 一起补
- 决策：若 `fail-fast-on-startup-errors` apply 时未落地 `kind` 字段，本 change 在 channel client `reachability()` 实现补上；tasks 1.3 标注依赖。
- 依据：本 change 补测必须有 production 字段可断言；两 change 在 spec 层已对齐规约，production 落地可以一并做。
- 备选：等 `fail-fast-on-startup-errors` apply → 否决：拉长交付窗口、产生新的「已声明未实现」窗口期。

### D4：approval_answer e2e 用 fake-claude 触发"perm"触发词
- 决策：扩展 `tests/bin/fake-claude.rs` 增加 gated tool call 触发（已有 `trigger_words` 机制可复用）；新增 "perm" 触发：fake-claude 在 `session/prompt` 触发后走 `session/request_permission` 路径，core 推到 channel ApprovalRequested 帧，webui 渲染 review-card。
- 依据：现有 `permission.spec.ts` 已经在测 review-card deny / allow-once / allow-session 三条路径，但**触发源是 webui 后端 fake 服务而非真 channel ApprovalRequested 帧**——本期把触发源切到真通道。
- 备选：mock channel frame → 否决：e2e 名义上要求真通道；mock 反而失去覆盖价值。

### D5：cross-uid live process 测试走 POSIX `setuid()`，CI runner 默认 root
- 决策：单测用 `nix::unistd::setuid` 切到 nobody/daemon（`/etc/passwd` 现成账户）；若无权限则 `#[ignore]` 跳过——CI runner 默认 root，可以切；本地开发账户通常无权限，跳过不报错。
- 依据：单测注释明确说需要 live process；用现成 nobody/daemon 账户比造一个临时账户干净。
- 备选：用 namespaces → 否决：依赖过深、跨平台性差；`setuid` + 现成账户足够。
- 备选 2：在容器内跑（CI runner docker）→ 复杂度过高、本期不展开。

### D6：set_session_model e2e 让 fake-claude 暴露多 model
- 决策：扩展 fake-claude 在 initialize 阶段 `session/configOptions.model` 返回多 model（包含 `ok-model` 与 `bad-model`）；happy-path 切到 `ok-model`、rejection path 切到 `bad-model`。
- 依据：fake-claude 已经支持 configOptions 反射（`docs/acp-opencode-smoke.md` 间接确认）；多 model 切换可观察 `current_model` 同步。
- 备选：mock channel → 否决：同上。

### D7：StateSnapshot/Mutation/Subscribe 三件做 channel 转发层单测
- 决策：单测直接发 StateSnapshot/StateMutation/StateSubscribe 请求，断言服务端把请求路由到 state-store engine；engine 端可注入 fake impl。单测验证 channel 协议层转发语义，不重复测 engine 自身逻辑。
- 依据：state-store 已有覆盖；channel 仅需验证"请求能到 engine、engine 响应能回包"。
- 备选：起真 state-store engine → 复杂度上升、测试运行时间变长；不必要。

### D8：`ensure_message` IM 投递语义 channel 单测
- 决策：单测发 EnsureMessage 在 (a) 未知 key、(b) dormant key、(c) active key 上分别验证；active key 等价 Message 单测对比行为一致。
- 依据：`CoreChannelRequest::EnsureMessage` 已在协议枚举中存在（`protocol.rs`），无 client 路径触发；channel 层需要补单测覆盖服务端处理逻辑。
- 备选：在 im-service 侧覆盖 → 不在本 change 范围。

## Risks / Trade-offs

- [R1] fake-claude 增加 perm 触发可能影响现有 `permission.spec.ts` 行为 → mitigation：保留 webui 后端 fake 与 channel ApprovalRequested 两套触发路径（webui 单测继续用前者；channel e2e 用后者），分别注释说明。
- [R2] cross-uid 测试在 Windows CI runner 上无法运行 → mitigation：`#[cfg(unix)]` 守卫；`#[ignore]` 标记需 root；CI 上跑得通则不过；本地无 root 跳过。
- [R3] `SEBAS_STARTUP_ERROR_FILE` 读取竞态（core 进程 fork 后客户端立即读）→ mitigation：单元测试中先 touch 文件再起 backend；spec scenario 用「顺序时间」描述，不依赖精确时序。
- [R4] 新增 `/api/summary.reachability.kind` 字段可能影响现有 webui vitest 单测断言 → mitigation：现有测试用 `.toMatchObject({ ok: true })` 等宽口径，不依赖完整 schema；新增字段加 `#[serde(default)]` 与 `Option<...>` 处理向后兼容。
- [R5] approval_answer e2e 依赖 `wire-webui-sebas-agent-e2e` 任务 1.3 的接线实现 → mitigation：本 change 假定该接线已存在（COVERAGE.md 已声明属进行中任务）；若 apply 时未实现，本 change tasks 1.5 标明"阻塞等 1.3"。

## Migration Plan

按四笔 commit 顺序独立可回滚：

1. **Reachability 三态化 + API kind 字段（commit 1）**：`src/core_channel/client.rs` `reachability()` 增 `kind` 枚举；`SEBAS_STARTUP_ERROR_FILE` 读取；`sebas-webui/src/api.rs` `reachability_payload` 输出 `kind`；`/api/summary` 路由测试覆盖三类 kind；channel 单测新增 4 个 case（startup-failed with file / startup-failed fallback / auth-rejected / disconnected）。运行 `cargo test --workspace`。
2. **State 三件 + ensure_message + cross-uid（commit 2）**：channel 单测补 5 个 case（StateSnapshot/Mutation/Mutation 拒绝/Subscribe/ensure_message）；cross-uid live process 单测在 unix 下跑；跑 `cargo test --workspace`。
3. **fake-claude perm 触发 + approval_answer e2e（commit 3）**：扩展 fake-claude 增加 perm 触发；Playwright `permission.spec.ts` 新增 allow / deny 两条端到端；启动时切真通道 ApprovalRequested 路径（替换 webui 后端 fake）；运行 `invoke testsuite-webui --case permission`。
4. **set_session_model e2e + 账本（commit 4）**：扩展 fake-claude 增加多 model；Playwright `models.spec.ts` 新增 happy-path + unknown-model rejection 两条；COVERAGE 同步；跑 `invoke testsuite-webui` 全量 3 连绿。

回滚：每笔 commit 单一关注点，独立 revert。

## Open Questions

- `fail-fast-on-startup-errors` 是否在 apply 阶段同时落地 `kind` 字段？若是，本 change tasks 1.1-1.3 跳过（commit 1 仅补测）；若否，commit 1 同时承担字段落地。`depends-on: fail-fast-on-startup-errors` 关系在 tasks.md 标记。
- `wire-webui-sebas-agent-e2e` 任务 1.3（审批通道接线）是否在本次 apply 之前完成？若未完成，本 change commit 3 阻塞等 1.3。