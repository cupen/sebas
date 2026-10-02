## Purpose

把工作台会话面从纯内存态升级为可恢复态：会话转录条目、泊车审批、会话级 usage 计数按周期 checkpoint 进 SQLite 状态库，core 进程启动时回放——崩溃、强杀、断电不再丢失对话史，重启后待批审批恢复可决定。（背景：QA round9 实测强杀 core 后 23 个会话转录清零；`run.rs` 关停 dump 路径已退休，内存态没有任何落盘通道，唯一例外是显式归档快照。）

## ADDED Requirements

### Requirement: 周期 checkpoint 落盘

core SHALL 以周期性 checkpoint（默认不超过 30 秒一次，可配置）把每个会话的转录条目、泊车审批登记与会话级 usage 计数写入状态库（projects.db 域）；checkpoint SHALL 原子替换（单事务），崩溃时留下的只能是完整旧快照或完整新快照。回合进行中的增量允许丢失至上一 checkpoint—— SHALL NOT 因 checkpoint 阻塞或拖延回合事件流。落点遵守持久层准入：建表只经状态库表注册表，写入经单写 actor。

#### Scenario: 会话闲置超过一个 checkpoint 周期

- **WHEN** 一个已完成若干回合的会话静置超过 checkpoint 周期
- **THEN** 其转录、usage 已出现在状态库中，且再次静置不再产生重复写入（内容未变时不重写）

#### Scenario: checkpoint 不阻塞回合

- **WHEN** 某会话回合正在流式输出且 checkpoint 到期
- **THEN** 流式事件不因 checkpoint 写入而延迟或乱序

### Requirement: 启动回放

core 启动时 SHALL 把最近 checkpoint 的转录、usage 回放进对应会话（浏览器打开会话即见完整历史），把已泊车的审批重新登记为待批——工作台审查卡 SHALL 恢复呈现且可决定，决定照常送达执行体。回放以 checkpoint 为准 SHALL 诚实呈现截断点：checkpoint 之后、中断之前发生的内容按丢失处理，不伪造完整性。会话注册表（session_map）与回放内容 SHALL 一致：注册表里有而 checkpoint 缺失的会话按空转录呈现（现状行为）。

#### Scenario: 强杀后重启转录完整

- **WHEN** core 被强制杀死（无优雅退出）并按同参数重启，操作者打开此前有若干回合的会话
- **THEN** 转录显示至最近 checkpoint 的全部条目，会话级 usage 与 checkpoint 一致，非「未上报」

#### Scenario: 待批审批跨重启恢复

- **WHEN** 一个审批卡处于待批状态时 core 被强杀并重启
- **THEN** 重新打开该会话后审查卡恢复呈现，操作者可 Allow/Deny，决定送达执行体并照常落 tool_result

#### Scenario: checkpoint 截断诚实呈现

- **WHEN** 中断发生在最后一条用户消息之后、checkpoint 之前
- **THEN** 重启后该消息与其回复不出现（按丢失处理），会话可继续对话，不出现半截或伪造条目

### Requirement: 归档语义不受影响

显式 close 归档的快照落盘语义 SHALL 保持现状；回放不得把已归档会话的内容重新挂回活动会话，也不得与归档条目重复计数。

#### Scenario: 归档会话重启后不复活

- **WHEN** 会话被 close 归档后 core 重启
- **THEN** 该会话内容只在归档中，活动会话面不出现其转录副本
