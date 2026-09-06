## MODIFIED Requirements

### Requirement: webui 主控部署形态

watchdog 默认 SHALL 将 webui 注册为主控服务（`[watchdog.webui] enabled` 默认 `true`），而 core（会话核心）SHALL 默认停用（`[watchdog.core] enabled` 默认 `false`）。im 服务（飞书 bot 宿主）SHALL 默认跟随 `[feishu]` 启用判定（飞书启用即拉起，停用即不拉起），并可经 `[watchdog.im]` 显式覆盖。webui 与 im 进程都是 core session channel 的客户端，跨 core 重启保持存活。二者是核心的两个通道（`web` 与 `feishu`），通过通道抽象与核心交互。

#### Scenario: 默认部署只起 webui

- **WHEN** 无显式配置覆盖 watchdog 默认值且飞书未启用
- **THEN** `sebas run`（watchdog 守护）只拉起 webui 服务，core 与 router 均不启动

#### Scenario: 飞书启用时 im 服务默认拉起

- **WHEN** 配置启用飞书且无 `[watchdog.im]` 覆盖
- **THEN** `sebas run` 拉起 im 服务（其内注册飞书适配器），webui 与 im 跨 core 重启保持存活

#### Scenario: 通过 webui 服务页启用 core

- **WHEN** 操作者在 webui 服务页将 core 设为启用
- **THEN** watchdog 拉起 core（会话核心），webui 与 im 通过 core session channel 继续显示同一会话状态
