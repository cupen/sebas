## ADDED Requirements

### Requirement: ACP 桩驱动的浏览器呈现覆盖

浏览器级工作台旅程中凡断言 **ACP 驱动器行为**（并行审批卡片、thinking 呈现、流式结算形态）的用例，SHALL 以 `fake-claude` 桩剧本经浏览器沙箱装配驱动；沙箱配置按既有惯例（claude-empty / claude-stream）为所需剧本各提供独立 agent 装配段。

**与 `extend-test-model-scenarios` 的双载体分工（有意并行，非重复覆盖）**：同一浏览器呈现面在两条执行通路上各有独立用例，两者的**事件生产者不同**——本 change 经 ACP 子进程（`fake-claude` 桩）产生审批请求与流式帧，`extend-test-model-scenarios` 经 router 内置 `test` 模型（native 内核通路）产生。浏览器呈现层虽同，但从生产者到 UI 的链路（ACP 驱动解析 / hook 泊车 / 帧投递 vs native 内核直投）不同，任一通路的回归都不能被另一通路发现。因此 `并行审批卡片` 在两侧各有一条浏览器旅程：本侧为桩驱动的 ACP 通路权威，test 模型侧为 native 通路权威；两侧 SHALL NOT 相互替代、SHALL NOT 因对方存在而豁免，账本按各自 capability 分别记行。桩剧本无法驱动而只能由 router test 模型驱动的浏览器呈现，SHALL 留给 `extend-test-model-scenarios`。

#### Scenario: 并行审批卡片各自独立（桩驱动）

- **WHEN** 以并行剧本 agent 会话在浏览器提交一个多工具任务
- **THEN** 两个审批卡片各自独立出现（独立卡片、独立工具名/参数），而非一张合并卡
- **AND** 逐一决策后回合继续推进，两个工具结果如实呈现

#### Scenario: thinking 过程折叠在浏览器中真链路呈现

- **WHEN** 以 thinking 剧本 agent 会话在浏览器完成一个 thinking/正文交替的回合
- **THEN** thinking 段落以过程折叠（collapsed process fold）呈现，正文独立于折叠之外按序展示
- **AND** 回合结算后 thinking 折叠与正文顺序保持、内容不丢失

#### Scenario: 流式正文在回合结算时切换为 markdown 渲染

- **WHEN** 以流式触发词（drip）会话在浏览器提交消息，turn_engaged 期间观察增量正文
- **THEN** 回合进行中增量正文以 live-tail 纯文本形态上屏（与 conversation-streaming 既有断言一致）
- **AND** 回合结算后同一正文转为 markdown 渲染且内容拼接一致，无重复或丢段
