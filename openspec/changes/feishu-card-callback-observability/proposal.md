## Why

飞书卡片回调（card.action.trigger）有两条官方投递形态：订阅配置生效时走 `type=event` 帧（常规分发可达），**未生效时走 `type=card` 帧——所有官方 SDK（Go/Python/Java/Node/openlark）一律丢弃**。后者发生时 sebas 侧零日志、零反馈，飞书全部按钮（权限卡 / provider 卡 / 表单）静默全灭；本次排查权限按钮失效耗掉整轮代码考古才定位到"配置未生效 + SDK 丢帧"的组合。此外 im 前端 `on_button` 存在两处静默早退（缺 request_id、决定无法识别），用户点了没有任何可见反馈。同类故障不应再靠考古定位。

## What Changes

- **回调到达性自检**：im 前端维护两个计数——已发出带按钮卡片数、曾收到 `card.action.trigger` 数；当"发过按钮卡 ≥N（如 3）且从未收到过任何回调"时打一条 WARN，指明检查开发者后台卡片回调订阅是否已随版本发布生效（`type=card` 帧即未生效的标志形态）。指标先进日志，不强求挂 API。
- **不可识别点击可见化**：`on_button` 两处静默早退（缺 request_id / 决定无法识别）改为向聊天回执"点击未识别"提示（复用既有过期卡/文本回执形态），并保留 INFO 日志；点击必须有可见结果，不再无痕返回。
- **跟进 openlark 上游**：main 分支已把 card 帧日志升为 WARN 并新增 `register_callback` 业务响应通道（toast/卡片内联应答），随其发版升级即得官方告警与增强；升级动作本身不属本变更实现项，作跟进注记。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `feishu-bridge`：新增「按钮卡片回调到达性可观测」要求（自检计数 + WARN 门槛）与「按钮点击 SHALL 产生可见反馈」要求（覆盖不可识别/缺字段点击，杜绝静默早退）。

## Impact

- 代码：`sebas-im/src/frontend.rs`（`on_button` 可见反馈 + 回调计数）、提示卡渲染一处。
- 验证边界：回调到达性自检依赖真实飞书 ws（沙箱无真实长连接），只能验证计数逻辑本身；WARN 触发语义以单测模拟计数序列覆盖。

## Non-goals

- 不改 openlark、不 vendor fork——官方路径是 `type=event` 帧，订阅配置随版本发布生效即通。
- 不做回调丢失后的自动重发或补偿机制。
- 权限三档语义与 `allow_session = grant_all` 的既有实现分叉不在本变更处理（另案）。
