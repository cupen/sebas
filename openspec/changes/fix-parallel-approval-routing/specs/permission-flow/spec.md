## MODIFIED Requirements

### Requirement: Parallel approvals render concurrently

同一回合同时挂起多个权限请求时，每个待批请求 SHALL 同时拥有各自可直达的决策入口（可独立寻址，不排队串行呈现）；决策完成后，每个请求的条目（无论允许或拒绝）SHALL 连同各自结果保留在转写中，不得整体消失。

每张待批卡 SHALL 与唯一 `request_id` 精确配对：操作员在任一张卡上做出的决定 SHALL 原样携带该卡的 `request_id` 到达引擎，并被路由到对应的待批请求——任何到达顺序、合并顺序或页面刷新后重排都不得引起错配。待批卡的显示顺序 SHALL 确定：同一待批集合在刷新前后不得无因翻转（排序键任选，稳定即可）。

#### Scenario: two pending requests both decidable

- **WHEN** 一个回合同时挂起两个权限请求
- **THEN** 操作员可同时看到两张卡并独立决策，无需先处理完第一张才能触达第二张

#### Scenario: allowed entry remains after decision

- **WHEN** 并行请求之一被允许、另一个被拒绝
- **THEN** 两个工具条目都留在转写中，各带自己的结果（成功文本 / 错误标识）

#### Scenario: decision routes to the clicked card

- **WHEN** 两张待批卡（工具 A、工具 B）同时呈现，操作员对工具 A 的卡点 Allow、对工具 B 的卡点 Deny（先后顺序任意，包括先 Deny 后 Allow 的乱序）
- **THEN** 工具 A 被允许执行、工具 B 被拒绝，两者的 tool_result 与各自身份一致，不发生互换

#### Scenario: decision after reload still routes correctly

- **WHEN** 两个请求同时挂起期间刷新页面，挂起卡按稳定顺序重新呈现后操作员再决策
- **THEN** 每张卡的决策仍精确路由到该卡自己的 `request_id`，结果不互换

#### Scenario: pending card order is stable

- **WHEN** 同一待批集合在决策前后的两次呈现（含刷新页面后重建审批面）
- **THEN** 卡片显示顺序保持一致，不随无关状态翻转
