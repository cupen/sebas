# deploy-mode Specification

## Purpose
Defines sebas deployment shapes and multi-channel shared session state: which services the watchdog brings up by default (webui-primary control shape), how optional channels hook into that default, and how concurrent channels converge on one session authority through the channel abstraction — independent of any single channel's configuration.

## Requirements

### Requirement: Webui-primary deployment shape

The watchdog SHALL default to registering webui as the primary control service (`[watchdog.webui] enabled` default `true`), while the core (session authority) SHALL default to disabled (`[watchdog.core] enabled` default `false`). An optional IM service (hosting channel adapters) SHALL by default follow its channel's enablement — enabled channel launches the IM service, disabled channel does not — and SHALL be overridable via `[watchdog.im]`. Webui and the IM service SHALL both be clients of the core session channel and SHALL stay alive across core restarts; they are the core's two channels (`web` and any IM channel), interacting through the channel abstraction.

#### Scenario: 默认部署只起 webui

- **WHEN** 无显式配置覆盖 watchdog 默认值且飞书未启用
- **THEN** `sebas run`（watchdog 守护）只拉起 webui 服务，core 与 router 均不启动

#### Scenario: 通道启用时 im 服务默认拉起

- **WHEN** 配置启用某 IM 通道且无 `[watchdog.im]` 覆盖
- **THEN** `sebas run` 拉起 im 服务（其内注册该通道适配器），webui 与 im 跨 core 重启保持存活

#### Scenario: 通过 webui 服务页启用 core

- **WHEN** 操作者在 webui 服务页将 core 设为启用
- **THEN** watchdog 拉起 core（会话核心），webui 与 im 通过 core session channel 继续显示同一会话状态

### Requirement: Multi-channel shared session state

When multiple channels (e.g. webui and an IM channel) are enabled together, they SHALL converge on the same session authority: sessions from every channel appear in the same snapshot, and a session created/changed/removed on either side SHALL be visible to the other through the shared state. Shared session state SHALL be expressed through the channel abstraction and `ChannelKey`; channel-specific id prefixes are the respective channels' opaque references and SHALL NOT be special-cased by the core.

#### Scenario: IM-channel session appears in webui list

- **WHEN** an inbound message on an IM channel creates a session
- **THEN** that session is visible in webui's `GET /api/sessions`, and does not fall into any project's inbox grouping semantics

#### Scenario: webui session not operable from another channel

- **WHEN** a webui session exists
- **THEN** the other channel does not render its cards nor receive its outbound events (that channel has no reply target for the session)
