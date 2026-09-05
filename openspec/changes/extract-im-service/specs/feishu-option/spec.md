# feishu-option Specification（delta）

## MODIFIED Requirements

### Requirement: Feishu 显式启用开关

sebas SHALL 提供飞书接入的显式开关节 `[feishu] enabled`。缺省值 SHALL 为 `false`。该开关与历史隐式判定（`app_id` 与 `app_secret` 同时非空）的关系如下：当 `enabled` 显式缺失时，接入与否 SHALL 回退到历史隐式判定（双非空即接入）；当 `enabled` 显式给出时，SHALL 以显式值为准，且若 `enabled = true` 而凭据不完整，SHALL 在 im 服务启动时以配置错误拒绝启动。启用判定仅决定 feishu 适配器是否注册进 im 服务的通道注册表（见 `channels`、`im-service` capability）；它 SHALL NOT 影响任何其他通道的注册。core 进程 SHALL NOT 读取该开关建立飞书连接（core 不再链接飞书实现）。

#### Scenario: 显式关闭时进程以 webui 主控形态运行

- **WHEN** 配置 `[feishu] enabled = false`（或缺失且凭据为空）
- **THEN** feishu 适配器不注册、im 服务不建立飞书 WebSocket 连接、不做 token 获取、不出站请求飞书 API
- **AND** watchdog 默认只启动 webui 服务，core 停用

#### Scenario: 显式开启但凭据不完整

- **WHEN** 配置 `[feishu] enabled = true` 但 `app_id` 或 `app_secret` 为空
- **THEN** im 服务启动校验报错并拒绝启动，指明飞书凭据缺失，feishu 适配器不注册

#### Scenario: 隐式启用仍可用

- **WHEN** 配置未写 `enabled` 字段，但 `app_id` 与 `app_secret` 均非空
- **THEN** feishu 适配器仍按历史行为注册接入（向后兼容）

#### Scenario: core 单独运行时不校验飞书凭据

- **WHEN** 配置含完整 `[feishu]` 凭据，部署仅启动 core（未启用 im 服务）
- **THEN** core 正常启动且不建立任何飞书连接

### Requirement: webui 主控部署形态

watchdog 默认 SHALL 将 webui 注册为主控服务（`[watchdog.webui] enabled` 默认 `true`），而 core（会话核心）SHALL 默认停用（`[watchdog.core] enabled` 默认 `false`）。im 服务（飞书 bot 宿主）SHALL 默认跟随 `[feishu]` 启用判定（飞书启用即拉起，停用即不拉起），并可经 `[watchdog.im]` 显式覆盖。webui 与 im 进程都是 core session channel 的客户端，跨 core 重启保持存活。二者是核心的两个通道（`web` 与 `feishu`），通过通道抽象与核心交互。

#### Scenario: 默认部署只起 webui

- **WHEN** 无显式配置覆盖 watchdog 默认值且飞书未启用
- **THEN** `sebas run`（watchdog 守护）只拉起 webui 服务，core 与 router 均不启动

#### Scenario: 飞书启用时 im 服务默认拉起

- **WHEN** 配置启用飞书且无 `[watchdog.im]` 覆盖
- **THEN** `sebas watchdog` 拉起 im 服务（其内注册飞书适配器），webui 与 im 跨 core 重启保持存活

#### Scenario: 通过 webui 服务页启用 core

- **WHEN** 操作者在 webui 服务页将 core 设为启用
- **THEN** watchdog 拉起 core（会话核心），webui 与 im 通过 core session channel 继续显示同一会话状态
