## ADDED Requirements

### Requirement: core 连接状态常驻指示

主界面 SHALL 提供一个常驻的 core 连接状态指示（徽标/圆点），数据源为既有 `core.reachability` 订阅与主动查询：可达时以低调的常态呈现（不抢注意力），不可达时转为醒目呈现并与既有断线横幅联动（同一事实两种强度，不互相矛盾），恢复可达时自动翻回常态。指示的 kind/cause 细节 SHALL 可经悬停等轻交互获知。

#### Scenario: 健康时低调呈现

- **WHEN** core 可达（reachability.ok = true）
- **THEN** 常驻指示显示正常态，不附加告警文案

#### Scenario: 断连时醒目且联动

- **WHEN** core 断连（ok = false，kind = disconnected 等）
- **THEN** 指示转为醒目异常态，悬停可见 cause
- **AND** 页面既有断线横幅同时存在，两者状态一致

#### Scenario: 恢复自动翻转

- **WHEN** core 恢复可达
- **THEN** 指示自动翻回正常态，断线横幅消失，无需刷新页面
