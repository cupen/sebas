# 设计：feishu-card-callback-observability

## Context

飞书卡片回调有两条投递形态：订阅配置生效走 `type=event` 帧（常规分发可达），未生效走 `type=card` 帧（官方 SDK 一律丢弃）。后者发生时 sebas 侧零日志零反馈，全部按钮静默全灭——本次权限按钮失效排障靠整轮代码考古才定位。此外 `on_button` 有两处静默早退（缺 request_id、决定无法识别），用户点了没有任何可见结果。见 proposal.md — Why。

## Goals / Non-Goals

**Goals:** 回调到达性自检（双计数 + 阈值 WARN，仅日志）；不可识别点击的聊天内可见回执。
**Non-goals:** 不改 openlark、不 vendor fork；不做回调丢失后的重发/补偿；不做指标 API（先进日志）；权限三档语义与 allow_session 分叉另案（permission-mode-auto-gate）。

## Decisions

### D1. 计数器放 im 前端进程内（`sebas-im/src/frontend.rs`）

- **sent 计数**：挂在所有「带按钮卡片」的发送出口（权限卡、provider 卡、表单卡——以 port 侧交互卡发送点为准，实现时盘点；粗粒度 = 每次成功发出一张交互卡 +1）。
- **received 计数**：挂在 `card.action.trigger` 的分发入口（`on_button` 被调处），只要帧到达就 +1，无论后续是否可识别。
- 实现：`AtomicU64` 即可（无锁、跨 await 安全）；warn-latch 用 `AtomicBool`。

### D2. WARN 触发语义 = 发送时评估 + 单次闩锁

每次成功发出按钮卡后评估 `sent >= 3 && received == 0`：满足且闩锁未置 → 打一条 WARN（文案指明：检查开发者后台卡片回调订阅是否随版本发布生效；`type=card` 帧即未生效的标志形态）并置闩。收到任一回调后闩锁复位为「条件不可能再满足」（received > 0 后表达式恒假，无需显式复位逻辑）。选发送时点而非后台定时器：无新发送就没有新的不可达风险，定时轮询只会空转。

### D3. 不可识别点击回执 = 文本回执（`ensure_message`）

两处早退（缺 request_id / 决定无法识别）改为经 `self.port.ensure_message(key, 文本, [])` 向来源聊天发一条「点击未识别」说明（附 decision 原始值缩略；缺 request_id 场景说明可能来自卡片版本错位）。不翻卡：两类点击都定位不到可翻的卡片。既有 INFO 日志保留。发送失败仅 `warn!` 不上抛（反馈尽力而为，不因回执失败放大故障）。

### D4. 测试边界（如实声明）

沙箱无真实飞书 ws 长连接——回调到达性只能验证计数与触发**逻辑**（单测模拟计数序列），端到端到达性依赖真实环境人工复测（bd sebas-033 在跟）。可见回执可单测（fake port 断言 ensure_message 调用与文案）。

## Risks / Trade-offs

- [发送点盘点遗漏（新卡种不加计数）] → 实现时在 port 层找交互卡发送汇聚点（若有单一出口最佳）；汇报中列明挂接点清单。
- [`type=card` 帧在 openlark 升级前仍不可见] → 本变更只保证 sebas 侧可观测（发出侧计数真实存在），SDK 丢帧不可见是既有边界，WARN 文案已指认该形态。
- [文本回执对群聊有打扰] → 仅不可识别点击触发（正常使用不出现）；文案一行内。

## Migration Plan

纯 im 前端增量，无迁移。回滚 = revert。

## Open Questions

（无——阈值 3 已由 proposal 定；触发点、闩锁、回执形态为本设计的实现裁决。）
