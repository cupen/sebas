## ADDED Requirements

### Requirement: 不可用 agent 的禁用提示如实

会话创建对话框对不可用 agent 的禁用选项 SHALL 如实呈现不可用原因与启用条件。
native 内核不可用时，提示 SHALL 指向真实启用条件——core 进程启动 env
（`SEBAS_AGENT_PROVIDER_API_KEY` 或 `SEBAS_AGENT_ROUTER_URL` 至少其一）——并如实
说明该条件无法在 WebUI 内满足；SHALL NOT 引导操作者到对启用无效的界面
（QA round13 实证：在「设置 → 模型」配置 provider 不能点亮 native，现提示
「到「设置 → 模型」配置」失实）。其它 agent 的不可用提示（如启动定义缺失指向
「设置 → Agent」）不受本要求影响，按既有口径保留。

#### Scenario: native 禁用提示指向真实启用条件

- **WHEN** native 内核因缺 env 被禁用（`/api/summary` 的 execution_bodies 中
  native `ok=false`）
- **THEN** 新建会话对话框中 native 选项的禁用提示说明需以 core env 方式启用
  （提及 `SEBAS_AGENT_PROVIDER_API_KEY` 或 `SEBAS_AGENT_ROUTER_URL` 至少其一）
- **AND** 提示中不出现「到「设置 → 模型」配置」字样

#### Scenario: 配置 provider 不改变 native 可用性

- **WHEN** 操作者在「设置 → 模型」添加或修改任意 provider 配置
- **THEN** native 选项的可用性保持由 core env 决定，不因该配置而点亮或变化，
  禁用提示保持如实
