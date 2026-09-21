## ADDED Requirements

### Requirement: Persisted spawning states settle on restore

恢复路径载入持久化状态时，任何处于 spawning 相位的会话 SHALL 被强制落定：要么按
既定 spawn 流程重新投递（spawn 指令重发，成功则照常激活），要么在会话投影追加一条
合成错误条目并转为明确的失败/idle 终态。恢复完成后 SHALL NOT 存在任何仍停留在
spawning 相位的会话。单个会话的落定 MUST NOT 延迟或抑制其它会话的恢复。

#### Scenario: 重启后不留僵尸 spawning

- **WHEN** 实例在某会话处于 spawning 相位时退出，随后实例重启并恢复状态
- **THEN** 该会话不再处于 spawning 相位：或已重新进入 spawn 流程，或其投影含
  合成失败条目且状态为非 spawning 终态

#### Scenario: 落定不阻塞其余恢复

- **WHEN** 恢复时同时存在 spawning 会话与多个 dormant 会话
- **THEN** dormant 会话的恢复不被 spawning 会话的落定过程阻塞

### Requirement: Turn completing without visible output appends a notice

一个真实回合（由消息或显式激活触发）终止时，若未产生任何可见输出条目（正文、
thinking、工具、错误皆无），系统 SHALL 向会话投影追加一条合成提示条目，说明回合
已结束且无输出。该提示条目在会话时间线上 SHALL 与其它条目同样可见。MUST NOT 让
一个回合在时间线上不可见地消失。

#### Scenario: 空回合有落点

- **WHEN** 会话收到一条消息，子进程正常结束回合但未产生任何输出条目
- **THEN** 会话投影追加合成提示条目，回合在时间线上可见

#### Scenario: 正常回合不受影响

- **WHEN** 回合正常产生正文或工具输出后结束
- **THEN** 不追加零输出提示条目
