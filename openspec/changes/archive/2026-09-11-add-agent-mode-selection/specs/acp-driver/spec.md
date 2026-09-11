## ADDED Requirements

### Requirement: Claude 会话的启动权限模式

claude driver SHALL 支持在 spawn 时应用会话请求的权限模式：控制面 mode 按约定映射（`ask` → 不传 `--permission-mode`（CLI 默认逐次询问）、`edit` → `acceptEdits`、`allow`/`auto` → `bypassPermissions`），作为子进程参数下发。请求为缺省（不携带 mode）时行为与今日一致（不传该参数）。映射失败或 agent 不接受 SHALL NOT 使会话失败。

#### Scenario: spawn argv 携带映射后的权限模式

- **WHEN** 以 mode=edit 创建本机 claude 会话
- **THEN** 子进程 argv 含 `--permission-mode acceptEdits`，并以该模式执行首回合

#### Scenario: 缺省 mode 不改变 spawn 行为

- **WHEN** 创建会话未携带 mode
- **THEN** 子进程 argv 不含 `--permission-mode`，权限行为与既有会话一致

### Requirement: Claude 会话的运行时权限模式切换

claude driver SHALL 支持运行时切换权限模式（SDK `set_permission_mode`）：切换命令送达后 agent 接受与否经事件流回报——接受时发布 `ModeChanged` 事件（携带生效的 mode），拒绝或失败时发布非致命错误事件，会话继续存活。

#### Scenario: 运行时切换被 agent 接受

- **WHEN** 对运行中的 claude 会话发出 mode 切换为 allow
- **THEN** driver 经运行时通道下发切换，agent 应答后发布 `ModeChanged`（mode=allow），后续门控按新模式执行

#### Scenario: 运行时切换失败不终止会话

- **WHEN** 运行时切换请求失败（agent 拒绝或通道错误）
- **THEN** 发布一条非致命错误事件，会话继续可用，mode 保持原值

### Requirement: 存活探针不得覆盖会话权限模式

watchdog 存活探针目前周期性下发 `set_permission_mode(Default)` 作为无副作用 liveness 探针；该探针 SHALL 发送会话当前配置的 mode（或会话未配置 mode 时的 Default），SHALL NOT 把操作者设置的权限模式覆盖回默认。

#### Scenario: 探针后 mode 保持

- **WHEN** 会话以 allow 运行且存活探针持续周期下发
- **THEN** 会话权限模式保持 allow，探针仍能按既有节奏判活
