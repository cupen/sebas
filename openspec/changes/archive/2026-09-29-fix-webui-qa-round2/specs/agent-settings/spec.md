## ADDED Requirements

### Requirement: Agent form covers spawn-critical fields

agent 新建/编辑表单 SHALL 覆盖 spawn 所需字段：sessions 目录、work 目录与启动参数（键值形态），校验口径与 config 种子条目一致；用表单建出的 agent SHALL 能直接 spawn 可用会话，无需手改 config。编辑表单 SHALL 以存储值预填全部字段，包括 display name。

#### Scenario: GUI-created agent is spawn-ready

- **WHEN** 操作员经设置表单以完整字段（含 sessions 目录、work 目录、参数）创建 agent 并用其开session
- **THEN** 会话以表单填写的目录与参数 spawn 成功并完成回合

#### Scenario: edit prefills display name

- **WHEN** 操作员打开已存有 display name 的 agent 的编辑表单
- **THEN** display name 字段回填存储值而非留空
