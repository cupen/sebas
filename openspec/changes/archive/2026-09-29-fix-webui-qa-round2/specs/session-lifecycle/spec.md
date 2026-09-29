## ADDED Requirements

### Requirement: Crash settles the backend turn promptly

agent 子进程回合中途死亡时，后端回合 SHALL 在 UI 错误卡片呈现的同一时间窗内进入终态——不得残留会趋向停滞看门狗地平线（数百秒）的 running/working 相位；终态可见后 SHALL 立即可接受新提交，无需等待停滞看门狗强制收尾。

#### Scenario: refocus after crash can send immediately

- **WHEN** agent 进程回合中途崩溃且 UI 已显示错误终态卡片，操作员随后重聚焦该会话
- **THEN** composer 立即可提交新消息（无残留的停止/运行中可供性），且后端回合此刻已是终态

#### Scenario: no stall-horizon residue

- **WHEN** 一次崩溃已将回合终态化
- **THEN** 该回合之后不需要停滞看门狗的强制收尾兜底（无二次「回合停滞」卡片）
