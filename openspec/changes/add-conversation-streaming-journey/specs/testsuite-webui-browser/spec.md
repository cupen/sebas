## MODIFIED Requirements

### Requirement: agent 对话覆盖

套件 SHALL 覆盖 agent 对话的核心功能：单会话内多轮消息往返、回合状态收敛、transcript 持久化恢复、composer 输入守卫。功能下 SHALL 划分以下子功能：

- **子功能 首回合往返**：从 composer 提交文本，transcript 依次出现用户消息与桩回复，回合收敛为 Done。
- **子功能 多轮连续**：同会话两轮连续问答，按序追加、双 Done、重载不丢。
- **子功能 重载恢复**：完成回合后刷新页面回到该会话，transcript 与状态从持久化恢复，内容不丢。
- **子功能 流式分批**：桩按时间间隔发出文本 delta；用例在会话仍处于运行态时于 DOM 观察到已上屏的增量正文，随后回合收敛为 Done；零固定 sleep、不依赖 retry 兜底。
- **子功能 输入守卫**：空/空白不建回合；特殊字符与长文本能完整往返。

为支撑「流式分批」，套件 SHALL 具备确定性构造「回合进行中」窗口的桩能力：桩按可配置的时间间隔逐段发出文本 delta，且该间隔落在 driver 的挂起（hang）探测预算内，使中途窗口既不因过快而不可观测、也不因超时被判为挂起。该能力 SHALL NOT 依赖真实模型凭据。

具体用例 SHALL 围绕上述子功能展开。本 requirement 不穷举轮数、文本种类与后端组合；后续补充用例 SHALL 落在现有子功能下、在 `tests/acceptance/COVERAGE.md` 添加对应行，无需修订本 spec 即可接纳。

#### Scenario: 首回合往返

- **WHEN** 操作员在 composer 输入文本提交
- **THEN** transcript 依次出现用户消息与桩的固定回复，会话状态收敛为 Done

#### Scenario: 同会话多轮连续

- **WHEN** 在同一会话内连续提交两轮文本
- **THEN** 两次回合均按序追加、双 Done、重载后内容不丢

#### Scenario: 流式分批渲染

- **WHEN** 以流式触发词发起回合，桩按时间间隔逐段发出文本 delta
- **THEN** 在回合到达终态之前，focused conversation 的 DOM 已出现增量正文（会话此时仍处于运行态）
- **AND** 该断言不依赖重试兜底；回合随后收敛为 Done

#### Scenario: composer 输入守卫

- **WHEN** 提交空字符串/纯空白；提交含特殊字符与长文本
- **THEN** 前者不建回合；后者完整往返
