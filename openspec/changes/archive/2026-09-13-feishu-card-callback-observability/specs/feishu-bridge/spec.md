## ADDED Requirements

### Requirement: 按钮卡片回调到达性可观测

IM 前端 SHALL 自检按钮卡片回调的到达性：维护两个进程内计数——已发出带按钮卡片数与曾收到的 `card.action.trigger` 回调数。当「已发出带按钮卡片 ≥3 且从未收到过任何回调」时，SHALL 打一条 WARN 日志，指明应检查飞书开发者后台的卡片回调订阅是否已随版本发布生效（回调仅以 `type=card` 帧到达即订阅未生效的标志形态，该帧会被官方 SDK 丢弃）。计数与告警先进日志，不要求挂 API。告警 SHALL 带防刷屏语义（同一次进程运行内不重复触发）；回调一旦到达，条件即不再成立。

#### Scenario: 连发按钮卡零回调触发告警

- **WHEN** 进程累计发出 3 张带按钮卡片且 `card.action.trigger` 计数为 0
- **THEN** 日志出现一条 WARN，指明检查卡片回调订阅配置与 `type=card` 未生效标志形态

#### Scenario: 告警同进程不重复

- **WHEN** 告警已触发后继续发出更多按钮卡且仍无回调
- **THEN** 不再重复打同类 WARN

#### Scenario: 回调到达后条件失效

- **WHEN** 此前无回调、随后收到任一 `card.action.trigger`
- **THEN** 后续继续发按钮卡不再触发该 WARN

#### Scenario: 少于阈值不告警

- **WHEN** 进程仅发出 2 张带按钮卡片且无任何回调
- **THEN** 不打告警日志

### Requirement: 按钮点击产生可见反馈

飞书卡片按钮的每次点击 SHALL 产生该聊天内可见的结果，不再静默早退：`card.action.trigger` 处理中「缺 request_id」与「决定无法识别」两类不可识别点击，SHALL 向来源聊天回执一条「点击未识别」提示（复用既有过期卡/文本回执形态，说明点击未被识别及其可能原因），同时保留既有 INFO 日志。可识别点击的反馈（就地翻卡、过期卡）维持现状。

#### Scenario: 缺 request_id 的点击有回执

- **WHEN** 一个按钮回调缺少 request_id（顶层与 behavior value 均无）
- **THEN** 该聊天收到「点击未识别」文本回执，且日志保留 INFO 记录

#### Scenario: 决定无法识别的点击有回执

- **WHEN** 一个按钮回调的 decision 值无法解析为合法决定
- **THEN** 该聊天收到「点击未识别」文本回执（附无法识别的原始值），且日志保留 INFO 记录

#### Scenario: 可识别点击反馈不变

- **WHEN** 一个按钮回调携带合法 request_id 与决定
- **THEN** 按既有语义就地翻卡或置灰过期卡，不额外发文本回执
