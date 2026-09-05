## MODIFIED Requirements

### Requirement: webui 主控部署形态

watchdog 默认 SHALL 将 webui 注册为主控服务（`[watchdog.webui] enabled` 默认 `true`），而 core（飞书 bot 服务）SHALL 默认停用（`[watchdog.core] enabled` 默认 `false`）。webui 进程作为 core session channel 的客户端，跨 core 重启保持存活。webui 是核心的另一个通道（`web`），与飞书一样通过通道抽象与核心交互。

#### Scenario: 默认部署只起 webui

- **WHEN** 无显式配置覆盖 watchdog 默认值
- **THEN** `sebas run`（watchdog 守护）只拉起 webui 服务，core 与 router 均不启动

#### Scenario: 通过 webui 服务页启用 core

- **WHEN** 操作者在 webui 服务页将 core 设为启用
- **THEN** watchdog 拉起 core（飞书 bot 服务），webui 通过 core session channel 继续显示同一会话状态
