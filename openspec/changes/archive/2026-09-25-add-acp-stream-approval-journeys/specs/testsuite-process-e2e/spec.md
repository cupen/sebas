## ADDED Requirements

### Requirement: ACP 桩并行工具环剧本

`fake-claude` 桩 SHALL 提供「并行工具环」驱动剧本（触发词形态，与既有 perm / tool-loop 剧本同机制）：单个 ACP 回合产生两个 tool_use，并连发两个 hook_callback 审批请求使其**同时待批**，两个请求都在待批中时才开始等待决定；每个请求收到决定后各自落 tool_result（allow → 成功文本，deny → is_error），全部落定后输出环后正文并以 result 正常收尾。既有场景与触发词的行为 SHALL NOT 改变。

#### Scenario: 单回合并发产生两个待批请求

- **WHEN** 以触发词 `parallel` 驱动桩完成一个回合
- **THEN** 该回合先后发出两个不同 request_id 的 hook_callback 审批请求，且第二个发出时第一个仍待批
- **AND** 两个请求逐一收到决定后，回合以终文本与 result 正常收尾

#### Scenario: 决定组合逐请求生效

- **WHEN** 两个待批请求收到不同决定组合（allow/allow、allow/deny、deny/deny）
- **THEN** 每个 tool_use 的 tool_result 与其自身收到的决定一致，互不影响
- **AND** deny 不阻止另一工具的执行与回合收尾

### Requirement: ACP 并行权限进程级旅程

验收套件 SHALL 含一条经 webui 用户面（HTTP API + WS）驱动的进程级 journey：以并行剧本会话提交一回合，断言两个审批请求各自独立泊车与决策、回合在全部决策后推进至终态，且全程无真实上游外呼。

#### Scenario: 并行权限环 journey 全绿

- **WHEN** 进程级 e2e 套件以并行剧本会话执行「提交 → 两请求泊车 → 逐一决策 → 回合推进」旅程
- **THEN** journey 断言两个 request_id 各自独立出现、决策按请求生效、会话终态正确
- **AND** 重复执行旅程断言转录形状与终态一致（id/时间戳除外）
