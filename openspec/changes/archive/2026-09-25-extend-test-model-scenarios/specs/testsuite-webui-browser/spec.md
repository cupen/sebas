## ADDED Requirements

### Requirement: test 场景驱动的浏览器呈现覆盖

浏览器级工作台旅程中凡断言 **LLM 响应形状**的用例，SHALL 以 `test/<scenario>` 场景会话驱动（凡会话执行路径可达 router 的形态——含 native 内核经 router URL 的通路；native 通路的可用性遵循 `testsuite-acceptance`「native 链路验收策略」的 spike 门控）。`fake-claude` 桩的既有用例继续作为其**驱动器专属契约**（deny / crash / 流式触发词等 ACP 驱动行为）的权威，迁移按账本节奏进行且 SHALL NOT 降低覆盖口径。

**与 `add-acp-stream-approval-journeys` 的双载体分工（有意并行，非重复覆盖）**：`并行审批卡片` 的浏览器旅程在两侧各有一条，因两者的**事件生产者与执行通路不同**——本侧经 router 内置 `test` 模型（native 内核通路）产生审批请求与流式帧，该 change 经 ACP 子进程（`fake-claude` 桩）产生。浏览器呈现层虽同，生产者到 UI 的链路（native 内核直投 vs ACP 驱动解析 / hook 泊车 / 帧投递）不同，任一通路的回归都不能被另一通路发现。两侧 SHALL NOT 相互替代、SHALL NOT 因对方存在而豁免，账本按各自 capability 分别记行。同理，`流式中经 UI 取消` 亦为双载体：既有 `stop-settle.spec.ts` 以 `fake-claude --slow-ms` 桩驱动（ACP 通路，账本 ✅），本 spec 的 `test/long` 用例是 native 通路的对应件，不构成重复。

test 模型使其**可确定性驱动、且桩驱动不了**的浏览器呈现 SHALL 纳入覆盖方向（新增用例按本 spec 既有规则落子功能并在 `COVERAGE.md` 加行）：

- 同回合多个 tool_use 的**并行审批卡片**（各自独立弹出与决策）；
- 零输出回合的**通知呈现**；
- **流式中经 UI 取消**（流停止、取消如实呈现、会话可继续）；
- **UI 模型切换后下一回合行为变化**（切换端到端生效的可见证明）。

#### Scenario: 并行审批卡片在浏览器中各自独立

- **WHEN** 以 `test/tools-parallel` 会话在浏览器提交一个多工具任务
- **THEN** 每个工具调用的审批卡片各自独立出现
- **AND** 逐一决策后回合继续推进

#### Scenario: 零输出通知在浏览器中呈现

- **WHEN** 以 `test/empty` 会话提交一条消息
- **THEN** 回合完成且 transcript 无可见输出
- **AND** 零输出通知出现在会话面

#### Scenario: 流式中经 UI 取消

- **WHEN** 以 `test/long` 会话流式呈现期间经 UI 发起取消
- **THEN** 流停止、取消如实呈现
- **AND** 会话此后可发起新回合

#### Scenario: UI 模型切换下一回合生效

- **WHEN** 会话先以 `test/text` 完成一回合，经 UI 切换模型为 `test/tool-use` 后提交含工具的任务
- **THEN** 下一回合出现审批卡片（新场景行为生效）
- **AND** 切换前后的回合各自保留其呈现形状
