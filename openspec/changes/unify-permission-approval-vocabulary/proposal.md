## Why

权限审批的决策词表在 spec 间分裂：`agent-driver`（09-09）声称所有驱动共用 `allow_once / allow_session / deny / escalate` 四词词表，但代码里 `escalate` 是 native 内核专属——ACP 路径上静默降级为 AllowOnce（`sebas-webui/src/session_backend.rs:389-397`），该降级规则只活在注释引用的 design D6/R5 里，无 spec 记载。同时 `permission-flow`（08-24）仍自称管理「Claude 工具权限全程」（3 词词表、飞书卡片唯一表面），与 agent-driver 的跨驱动路由声称领地重叠；"escalate" 一词另有二义（审批决策 vs `acp-driver` 挂起检测的击杀阶梯），glossary 未消解。

## What Changes

- **`agent-driver`**：跨驱动权限路由 requirement 修订——词表仍为四词，但明确 `escalate` 仅对 native 内核会话有意义；新增 SHALL：ACP 驱动收到 `escalate` 决策时降级为 `allow_once` 并记录 warn（追认现状，行为不变）。
- **`permission-flow`**：范围收窄为本 hook 路径的飞书侧呈现与 per-chat allowlist 权威；跨驱动路由、request_id 命名空间化（`<kind-slug>:<raw-id>`）的归属让渡给 `agent-driver`；3 词词表保留（hook 路径无 escalate）。
- **`acp-driver`**：挂起检测 requirement 文本中「escalate: interrupt ×3 → disconnect → drop」改称「kill ladder」，消除与审批决策的同名冲突（纯措辞，SHALL 语义不变）。
- **glossary**（非 capability，随 tasks 更新）：新增「escalate 二义消解」条目。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-driver`: 权限路由 requirement 增加 escalate 的 native 专属限定与 ACP 降级 SHALL。
- `permission-flow`: Purpose 与领地收窄到飞书侧呈现 + allowlist；词表与归属对齐。
- `acp-driver`: 挂起检测 requirement 措辞消歧（escalate → kill ladder）。

## Impact

- 纯 spec 文本 + glossary 修订，**无代码行为变更**；`session_backend.rs` 降级逻辑不变，仅补齐对应单测断言（若缺）。
- 影响读者：spec 间引用 permission 决策词表的测试 spec（testsuite-webui-browser 审批卡片旅程）措辞需同步核对。

## Non-goals

- 不改变任何审批/降级行为（escalate 降级为 AllowOnce 是追认，不是新设计）。
- 不重写 `permission-flow` 的 hook 机制本身（park/oneshot/request_id 关联语义不动）。
- 不在本 change 内处理 permission-flow / feishu-reactions 其余年代断层（另立 change）。
