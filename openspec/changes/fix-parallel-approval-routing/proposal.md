# Proposal — fix-parallel-approval-routing

## Why

第三轮 GUI QA（`C:/Users/cupen/AppData/Local/Temp/sebas-qa/qa-evidence/w2/`，4 轮 `parallel` 场景实测）发现：≥2 张审批卡同时挂起时，操作员对某张卡的决定会落到**另一张卡**——4 轮中 2 轮完全颠倒（点 Read 卡 Allow → 实际 Bash 被执行、Read 被拒绝）；刷新页面后挂起卡显示顺序翻转，顺序与内部待批队列一致时路由才正确。这是安全相关缺陷（拒绝的工具可能被执行），且违反 `permission-flow`「Parallel approvals render concurrently」的独立可寻址要求——卡片可见但不可正确寻址。

## What Changes

- 修复审批决策路由：点击卡片所携带的 `request_id` SHALL 原样到达引擎并路由到对应的 hook_callback；任何情况下不得按「待批队列顺序」隐式配对。
- 修复卡片内容与 `request_id` 的配对稳定性：WS 推送条目与读模型（GET /approvals）条目合并时按 `request_id` 幂等配对，任何到达顺序/枚举顺序下，卡片呈现的工具名+input 与其提交决策的 request_id 保持一致。
- 挂起审批卡的显示顺序 SHALL 确定（同一待批集合在刷新前后不无因翻转），作为可测试的呈现契约。
- 单卡路径（ask/edit 模式单审批）回归保护：既有行为不回归。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `permission-flow`：「Parallel approvals render concurrently」要求补充决策正确性与顺序确定性场景——每张卡的决定 SHALL 精确路由到该卡的 request_id（乱序决策、页面刷新后决策均正确）；卡片显示顺序 SHALL 确定。

## Impact

- 前端 `sebas-webui/frontend/src`（transcript-view 审批卡渲染、审批面与 WS 推送的合并逻辑、api/client answerPermission）。
- 后端排查面：`sebas-webui` 审批读模型枚举序、`sebas-dispatch` 泊车登记路由、`vendor/cc-agent-sdk` 0.1.7 补丁（上一轮 fix-webui-qa-round2 改过 hook_callback 锁时序，排查需覆盖此路径）。
- 无 wire 破坏性变更；不改审批端点形状。
- 验证：前端单测（乱序合并配对）+ dispatch 单测（按 request_id 路由）+ GUI 手测 `parallel` 四轮（含刷新后决策）。

## Non-goals

- 不改审批卡 UI 形态、按钮语义（Allow once / Allow for session / Deny / Escalate）与权限模式门控。
- 不新增审批端点或 wire 字段（现有 request_id 已在 wire 上）。
- 不处理远端（feishu/node）审批面——本期只修工作台路径，远端面回归由既有套件守护。
