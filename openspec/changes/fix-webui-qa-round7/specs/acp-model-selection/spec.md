## MODIFIED Requirements

### Requirement: Model change via session/set_config_option

The system SHALL implement model switching on an ACP session by issuing the standard ACP `session/set_config_option` with `configId = "model"` and the chosen model id; the Claude-specific driver SHALL implement the same operator-facing semantic over its own control protocol (`set_model`). A rejected or unknown model value SHALL surface an explicit error and SHALL NOT change the session's current model silently. A successful Claude switch SHALL be reflected optimistically in the session's current model and SHALL be superseded by the next wire frame that carries a model name.

一次被拒绝的模型切换 SHALL 是状态自洽的终态事件：拒绝回执到达后，会话 SHALL NOT 遗留任何处于运行相位的回合——若拒绝发生时并无真实 agent 回合在跑，调度引擎不得因该拒绝事件把占位回合推入 WORKING 并滞留；若拒绝打断了一个真实回合，该回合 SHALL 以明确的失败终态收尾。收尾时限 SHALL 由回合状态语义保证（拒绝即终态），而非依赖停滞 watchdog 的超时兜底。回合中断标记 SHALL 按回合身份关联与消费：某个回合的中断标记只能由该回合自身的终结事件消费，不得注入到后续无关回合的呈现里；无相应操作者动作时 SHALL NOT 出现「操作者中断」类条目。

#### Scenario: Selecting a model switches the session

- **WHEN** the user picks a model from the session's list
- **THEN** the driver sends `session/set_config_option {configId:"model", value:<chosen>}` (or the Claude control-protocol equivalent)
- **AND** on success the session reports the new model

#### Scenario: Invalid model is rejected explicitly

- **WHEN** the agent rejects the model value (unknown id)
- **THEN** the caller receives an explicit error naming the model
- **AND** the session's current model is unchanged

#### Scenario: Claude switch applies from the next turn

- **WHEN** the operator switches a Claude session's model mid-session
- **THEN** the driver issues the control-protocol model switch, the session's current model updates, and subsequent prompts run under the chosen model

#### Scenario: 拒绝后回合不留挂起

- **WHEN** 一个 fakeacp 会话先成功切换到可用模型，再切换到被拒模型（类型化拒绝卡已如实上屏）
- **THEN** 该会话在拒绝回执后不再显示运行态，composer 恢复可提交
- **AND** core 日志与 turn 状态均无滞留回合，停滞 watchdog 未被触发（远早于 600 秒即收尾）

#### Scenario: 无虚假操作者中断条目

- **WHEN** 上述拒绝收尾后，操作者发送一条新消息并正常完成回合
- **THEN** 该新回合的呈现只有其真实内容，不出现「回合被停止（操作者中断了本次回复）」类条目
