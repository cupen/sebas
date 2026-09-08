## Context

`fail-fast-on-startup-errors` 与 `wire-webui-sebas-agent-e2e` 两个 change 已分别在 spec 层补了 startup-failure 区分、approval_answer e2e 规约，但实现与测试尚未闭环：
- `CoreChannelBackend::connect` 失败仅返回 generic error，`/api/summary.reachability.kind` 字段尚未存在——webui banner 无法区分 startup-failed / auth-rejected / disconnected。
- `wire-webui-sebas-agent-e2e` 任务 1.3 留下的 COVERAGE 缺口 1："审批事件经核心通道推送到 detached webui 的接线属进行中的 `wire-webui-sebas-agent-e2e` 任务 1.3；落地后补 `allow / deny` 两条旅程"——这是已知的 bug，进程内审批有，detached 端到端没测过。
- `set_session_model` webui 正向 e2e 缺；`StateSnapshot/Mutation/Subscribe` 三件未覆盖；cross-uid 拒绝 live process 未跑过（`src/core_channel/tests.rs:3-4` 注释已自陈）。

本期只补测 + `Reachability` 三态枚举与 `kind` 字段的最小 production 增量（owner 归本 change，见 D3）。

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
- 不动 `fail-fast-on-startup-errors` 已落地的 cause 富化与闩锁行为（只消费，不修改）。
- 不补 record/replay 通道（spec 单独列在 COVERAGE 缺口 4）。
- 不为 state-store 引擎自身加测试（已有覆盖）。

## Decisions

### D1：`Reachability` 三态区分用枚举变体，而非 kind 字段或字符串匹配
- 决策：`Unreachable { cause }` 拆为三变体 `StartupFailed { cause } | AuthRejected { cause } | Disconnected { cause }`（`Reachable` 不变，cause 保持 `String` 全串、沿用已落地的 enrich）。`/api/summary` JSON 输出新增 `reachability.kind`（snake_case 字符串：`startup_failed | auth_rejected | disconnected`，仅不可达分支带 kind + cause）；前端根据 kind 渲染 banner 文案。
- 依据：枚举变体让 webui 不会因 cause 文案变化误判；变体而非 kind 字段——Rust 侧穷举匹配逼着每个消费点处理三态，加字段则靠纪律；snake_case 字符串与现有 JSON 风格一致；schema 演进向后兼容（缺 kind 字段时前端按 `disconnected` 处理）。
- 备选：仅靠 cause 文案字符串匹配 → 否决：脆弱、文案改动即破前端。
- 备选 2：保留 `Unreachable` 加 `kind` 字段 → 否决：两层表达同一事实，match 时仍要先拆 kind；变体一步到位。

### D2：cause 富化沿用已落地的无条件 enrich，本 change 只加 `kind`

- 决策：不新增、不收窄 cause 读取逻辑。fail-fast task 2.4 已落地 `reachability()` 全失败分支的无条件 enrich（client.rs `enrich_with_startup_summary`）+ 闩锁 ready 自清除——stale 读已有防护，"仅 ENOENT 分支读"的限制是过度谨慎且与已落地代码矛盾，直接沿用。本 change 的 production 增量只有：`Reachability` 三变体枚举 + `/api/summary.reachability.kind` 输出 + 前端按 kind 选文案。
- 依据：cause 是人类可读串（全串 `core startup failed: <原因>`，前端原文渲染），kind 是机器可读判别器；两者正交，不互相代替。

### D3：`kind` 字段的 owner 是本 change（条件已消解）

- 决策：实测确认 fail-fast 只落地了 cause 字符串富化，`Reachability` 仍是两变体、`reachability_payload` 仍只输出 `{ok, cause}`——`kind` 三态枚举 + API 字段 + 前端选文案全部由本 change 落地，无 double-write。原先的"若 fail-fast 未落地则一起补"条件句删除。
- 依据：owner 写死才能合流；两边对称的条件句等于没人负责。

### D4：approval e2e 的真 gap 是 detached 拓扑，不是触发源（premise 已修正）

- 决策：fake-claude "perm" 触发已存在（`tests/bin/fake-claude.rs: perm_turn`），permission.spec 已走 live WS + follow-up composer 的真通道路径（单进程形态）——原先"触发源是 webui 后端 fake"的判断过时。真正的缺口是**双进程 detached 形态**（与 COVERAGE 缺口 1 一致）：同一流程在独立 core + 独立 webui 上跑通。本 change 不重写既有 P1/P2/P3，只新增 detached 变体 + `unknown_rid` 拒绝；harness 复用 harden 5.4 的双进程装配。
- 依据：单进程与 detached 是两种拓扑，不互相代替；重写已绿的用例无收益，只增 flake 风险。
- 备选：mock channel frame → 否决：e2e 名义上要求真通道；mock 反而失去覆盖价值。

### D5：cross-uid live process 测试走 POSIX `setuid()`，CI-only 非门禁

- 决策：单测用 `nix::unistd::setuid` 切到 nobody/daemon（`/etc/passwd` 现成账户）；若无权限则 `#[ignore]` 跳过——CI runner 默认 root，可以切；本地开发账户通常无权限，跳过不报错。**本用例是 CI-only 非门禁**：不挡本 change 的绿（见 tasks 5.2），避免"三年没人跑过"的 ignore 测试成为合流瓶颈。
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
- [R5] approval detached e2e 依赖 `wire-webui-sebas-agent-e2e` 任务 1.3 的接线实现 → mitigation：A/B 分批隔离阻塞面——A 批（kind + State + ensure + cross-uid）无外部依赖先行合流；B1 假定接线已存在，若 apply 时未实现则 B1 注记阻塞等待，harness（harden 5.4）可先行落地不空转。

## Migration Plan

分 A/B 两批独立合流（阻塞面隔离），每批内 commit 独立可回滚：

**A 批（先行，无外部依赖）：**

1. **Reachability 三态化 + API kind 字段**：`Reachability` 三变体枚举；`reachability_payload` 输出 `kind`；前端按 kind 选 banner 文案；channel 单测新增 4 个 case（startup-failed with file / fallback / auth-rejected 含重读重试语义 / disconnected）。运行 `cargo test --workspace`。
2. **State 三件 + ensure_message + cross-uid**：channel 单测补 8 个 case；cross-uid 为 CI-only 非门禁；跑 `cargo test --workspace`。

**B 批（等 1.3 + harden 5.4 harness 解冻）：**

3. **approval detached e2e**：复用双进程 harness；新增 allow-detached / deny-detached / unknown-rid 三条；运行对应 webui case。
4. **set_session_model e2e + 账本**：fake-claude 多 model 反射；models.spec 新增 happy-path + rejection 两条（先写清与既有终态用例的区分条件）；COVERAGE 同步。

回滚：每笔 commit 单一关注点，独立 revert。A/B 批各有独立的 tasks 验收（5.2），B 批可复用 A 批与 harden 合流时的共享 3 连绿（产品代码无变更时）。

## Open Questions

- 无 production 归属问题（kind owner 已定为本 change；cause enrich 沿用 fail-fast 落地行为）。仅剩外部阻塞：`wire-webui-sebas-agent-e2e` 任务 1.3（审批通道接线）是否在 B 批 apply 之前完成？若未完成，B1 阻塞等 1.3，A 批不受影响先行合流。