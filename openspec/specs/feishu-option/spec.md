# feishu-option Specification

## Purpose

飞书接入作为 sebas 的可选功能：显式开关、接入判定，以及「webui 为主控端、飞书为辅助入口」的部署形态下，双通道共享同一会话状态的行为面。

## Requirements

### Requirement: Feishu 显式启用开关

sebas SHALL 提供飞书接入的显式开关节 `[feishu] enabled`。缺省值 SHALL 为 `false`。该开关与历史隐式判定（`app_id` 与 `app_secret` 同时非空）的关系如下：当 `enabled` 显式缺失时，接入与否 SHALL 回退到历史隐式判定（双非空即接入）；当 `enabled` 显式给出时，SHALL 以显式值为准，且若 `enabled = true` 而凭据不完整，SHALL 在启动时以配置错误拒绝启动。启用判定仅决定 feishu 适配器是否注册进核心的通道注册表（见 `channels` capability）；它 SHALL NOT 影响任何其他通道的注册。

#### Scenario: 显式关闭时进程以 webui 主控形态运行

- **WHEN** 配置 `[feishu] enabled = false`（或缺失且凭据为空）
- **THEN** feishu 适配器不注册、进程不建立飞书 WebSocket 连接、不做 token 获取、不出站请求飞书 API
- **AND** watchdog 默认只启动 webui 服务，core 停用

#### Scenario: 显式开启但凭据不完整

- **WHEN** 配置 `[feishu] enabled = true` 但 `app_id` 或 `app_secret` 为空
- **THEN** 启动校验报错并拒绝启动，指明飞书凭据缺失，feishu 适配器不注册

#### Scenario: 隐式启用仍可用

- **WHEN** 配置未写 `enabled` 字段，但 `app_id` 与 `app_secret` 均非空
- **THEN** feishu 适配器仍按历史行为注册接入（向后兼容）

### Requirement: webui 主控部署形态

watchdog 默认 SHALL 将 webui 注册为主控服务（`[watchdog.webui] enabled` 默认 `true`），而 core（飞书 bot 服务）SHALL 默认停用（`[watchdog.core] enabled` 默认 `false`）。webui 进程作为 core session channel 的客户端，跨 core 重启保持存活。webui 是核心的另一个通道（`web`），与飞书一样通过通道抽象与核心交互。

#### Scenario: 默认部署只起 webui

- **WHEN** 无显式配置覆盖 watchdog 默认值
- **THEN** `sebas watchdog` 只拉起 webui 服务，core 与 gateway 均不启动

#### Scenario: 通过 webui 服务页启用 core

- **WHEN** 操作者在 webui 服务页将 core 设为启用
- **THEN** watchdog 拉起 core（飞书 bot 服务），webui 通过 core session channel 继续显示同一会话状态

### Requirement: 双通道共享会话状态

当 feishu 与 webui 同时启用时，两者 SHALL 汇聚到同一会话权威：webui 会话（`web-*` 前缀）与飞书会话（`oc_*` / `ou_*` chat_id）在同一个快照中可见，任何一侧创建/变更/移除的会话 SHALL 通过共享状态对另一侧可见。共享会话状态 SHALL 通过通道抽象与 `ChannelKey` 表达；`web-*` 与 `oc_*` 分别是 `web` 与 `feishu` 两个通道的 key，不由核心特判前缀。

#### Scenario: 飞书会话出现在 webui 列表

- **WHEN** 一条飞书消息创建了一个会话
- **THEN** 该会话在 webui 的 `GET /api/sessions` 中可见，且不落入任何项目的 inbox 分组语义

#### Scenario: webui 会话对飞书不可操作

- **WHEN** webui 会话（`web-*` 或 `agent-*` 前缀）已创建
- **THEN** 飞书侧不渲染其卡片，也不接收其出站事件（飞书无此会话的回复目标）