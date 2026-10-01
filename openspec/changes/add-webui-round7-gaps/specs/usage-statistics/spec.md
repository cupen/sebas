## ADDED Requirements

### Requirement: 会话级 token 用量可见

操作者 SHALL 能在会话呈现面（会话详情/转录区）看到该会话累计的 token 用量（input/output），数据源为调度引擎随会话快照输出的既有 usage 累计；展示 SHALL 随回合完成而增长。对未上报 token 计数的 agent（如通用 ACP 内核），界面 SHALL 如实呈现「未上报」语义，SHALL NOT 以 0 或其它实数冒充。

本需求不改变 router 逐请求 timeseries 的口径与归属：`/usage` 页继续只反映 router 代理链路。

#### Scenario: claude 会话累计 token 随回合增长

- **WHEN** 一个 fake-claude 会话完成至少一个回合
- **THEN** 会话呈现面出现该会话的累计 input/output token，数值与引擎累计一致
- **AND** 再完成一个回合后数值增长

#### Scenario: 未上报 token 的 agent 如实呈现

- **WHEN** 一个通用 ACP 会话（不提供 token 计数）完成回合
- **THEN** 会话呈现面明确显示未上报/不可得语义，而非 0

#### Scenario: 用量页口径不受影响

- **WHEN** 操作者打开 `/usage` 页
- **THEN** 内容仍为 router 逐请求 timeseries，ACP 会话 token 不混入该页
