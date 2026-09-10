## Why

`feishu-reactions`（08-24 bootstrap 后未再更新）描述的 reaction 管线在 `extract-im-service` 后已整体失效：spec 说 phase reaction 由 core 内飞书适配器消费引擎的 `Out::React`、并 SHALL 优先挂在用户消息（`input_msg_id`）上；现实是 core 出站泵（`src/run.rs:217-220`）只驱动会话执行，聊天向 `Out` 一律静默丢弃，改由 detached `sebas im` 前端从 `SessionInfo.phase` 自行渲染，且挂在**根卡**（`sebas-im/src/frontend.rs:360`）。后果：⏳ back-pressure reaction（`sebas-dispatch/src/engine/inbound.rs:714`）在唯一可用的 IM 形态下永久不可见；引擎内整条 reaction 发射链沦为死路径，留下三代互相打架的注释（`card_state.rs:25` vs `engine/mod.rs:1516` vs `acp_events.rs:125`）。

## What Changes

- **`feishu-reactions`**：按 im-service 后的现实重写。管线归属改为「引擎产 `SessionInfo.phase` → `sebas im` 前端 `ReactionTracker` 触发 reaction」；目标语义改写为**根卡**（`card_msg_id`），删除/降级 `input_msg_id` 优先条款（现无外部消费者）；⏳ back-pressure requirement 按选定路径处理（见下）。
- **`im-service`**：新增 requirement——IM 前端 SHALL 从 `SessionInfo.phase` 自行渲染会话级 reaction（含 swap 而非堆叠、终态由 card body 表达、`CrossMark` 定义不派发）。
- **`⏳ back-pressure 决策（默认 b）**：spec 追认现状——detached 形态下排队只靠卡片内文案表达，删除不可达的 ⏳ reaction requirement；design 记录路径 (a)（`SessionInfo` 增加 queue-depth → 前端渲染 ⏳）为替代方案与非目标。
- **代码注释清理**（随 tasks）：修 `acp_events.rs:125` 自相矛盾的 WORKING→DONE 注释、`engine/mod.rs:1516` FSM 与 `card_state.rs:25` 的口径，标注 core 侧 `Out::React` 为死路径（或删除，视 tasks 选择）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `feishu-reactions`: 管线归属、目标消息语义、⏳ requirement 整体重写。
- `im-service`: 新增 IM 前端 phase reaction 渲染 requirement。

## Impact

- 纯 spec 重写 + 代码注释清理；**不改变 detached im 的实际渲染行为**（追认现状）。
- 若选 ⏳ 路径 (a)，则需改 `sebas-dispatch`（SessionInfo 加队列字段）、`sebas-im` 渲染，并新增验收断言——本 change 默认不含，另立。
- 影响读者：`feishu-cards`（终态视觉归属）、`testsuite-webui-browser`（reaction 相关断言措辞）。

## Non-goals

- 不恢复 core 内飞书适配器（`extract-im-service` 已定型，不逆转架构方向）。
- 不改变 permission-flow 的审批词表/领地（已另立 `unify-permission-approval-vocabulary`）。
- 默认不实现 ⏳ 信号链路（路径 a）；如选 a 需用户拍板后扩范围。
