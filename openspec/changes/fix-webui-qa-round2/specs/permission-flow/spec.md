## ADDED Requirements

### Requirement: Decision results are readable in the transcript

审批决策完成后，tool 结果内容（成功或错误文本）SHALL 可被操作员读达：折叠/展开任一转写可供性不得连带隐藏另一层（无「点子折叠先折叠父级」的死路）；决策结果标识（已执行/已拒绝）SHALL 常驻条目可见处。单工具审批回合 SHALL 呈现工具环之后的 assistant 正文，不得丢失。

#### Scenario: allow then read the result

- **WHEN** 操作员批准一张权限卡且工具执行完成
- **THEN** 转写中无需折叠任何其它条目即可读到 tool 结果内容，且其后的 assistant 正文完整呈现

#### Scenario: decision stays labeled

- **WHEN** 已决策的审批条目被渲染（含页面刷新后）
- **THEN** 该条目上仍可见其结果标识（已执行/已拒绝）

### Requirement: Parallel approvals render concurrently

同一回合同时挂起多个权限请求时，每个待批请求 SHALL 同时拥有各自可直达的决策入口（可独立寻址，不排队串行呈现）；决策完成后，每个请求的条目（无论允许或拒绝）SHALL 连同各自结果保留在转写中，不得整体消失。

#### Scenario: two pending requests both decidable

- **WHEN** 一个回合同时挂起两个权限请求
- **THEN** 操作员可同时看到两张卡并独立决策，无需先处理完第一张才能触达第二张

#### Scenario: allowed entry remains after decision

- **WHEN** 并行请求之一被允许、另一个被拒绝
- **THEN** 两个工具条目都留在转写中，各带自己的结果（成功文本 / 错误标识）

### Requirement: Mode descriptions state behavioral equivalence honestly

模式菜单的描述文案 SHALL 如实陈述各模式的门控行为；行为同档的模式 SHALL 被描述为等价——任何描述不得暗示不存在的行为差异。

#### Scenario: equivalent modes say so

- **WHEN** 操作员查看两个门控行为完全一致的模式（如 allow 与 auto 同为全放行档）的描述
- **THEN** 两段描述不声称任何行为差异，且明确说明二者等价
