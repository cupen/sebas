## MODIFIED Requirements

### Requirement: 审批决策结果可见

After a parked permission request is decided (allow once, allow for
session, deny, or escalate), the outcome SHALL be observable in the UI
without manual unfolding: the tool call block produced by the approved or
denied call SHALL carry its execution outcome marker (已执行 / 已拒绝) on its
collapsed affordance, and the containing process fold's summary row SHALL
carry the same outcome marker, so a denied call is visually distinguishable
from a successful one at a glance. No fold SHALL be the only way to learn
the outcome. The result content itself SHALL be reachable through that
call's own block as the only content-fold toggle: ancestor folds on the
path may need expanding to reach the block (pure navigation that hides no
content), but no other content fold's state SHALL need toggling, and any
fold toggle SHALL never collapse or hide another layer (no deadlock).

#### Scenario: 放行后结果可见

- **WHEN** the operator allows a gated tool call and the tool executes
- **THEN** the corresponding tool call block shows its success outcome marker
  while still collapsed, and — after expanding the containing process fold
  to reach it — expanding that block alone reveals the result content, with
  no other content fold needing to be toggled

#### Scenario: 拒绝结果可辨识

- **WHEN** the operator denies a gated tool call
- **THEN** the tool call block renders the denial as an error-class outcome
  marker that is distinguishable from a success at a glance

### Requirement: Decision results are readable in the transcript

审批决策完成后，决策结果标识（已执行/已拒绝）SHALL 在**收起态**即可读（工具调用块的收起标题，以及其所属过程折叠的汇总行）；tool 结果内容 SHALL 以「展开该调用块本身」为唯一的内容开合动作即可读达——到达路径上的祖先过程折叠可为到达而展开（纯导航，不隐藏任何内容），除此之外无任何其它**内容**折叠需要切换；任一折叠的开合绝不连带隐藏另一层（无「点子折叠先折叠父级」的死路）——层级绝不死锁。单工具审批回合 SHALL 呈现工具环之后的 assistant 正文，不得丢失。

#### Scenario: allow then read the result

- **WHEN** 操作员批准一张权限卡且工具执行完成
- **THEN** 该调用块的收起标题即标明「已执行」，展开其所属过程折叠到达该块后、展开该块本身即读到结果内容（除此之外无任何其它内容折叠需要切换），且其后的 assistant 正文完整呈现

#### Scenario: decision stays labeled

- **WHEN** 已决策的审批条目被渲染（含页面刷新后）
- **THEN** 该条目上仍可见其结果标识（已执行/已拒绝）
