# feishu-option Specification

## Purpose

飞书接入作为 sebas 的可选功能：显式开关 `[feishu] enabled`、接入判定，以及凭据半配置与 env 覆盖的配置校验矩阵。部署形态与多通道共享会话状态见 `deploy-mode` capability。

## Requirements

### Requirement: Feishu 显式启用开关

sebas SHALL 提供飞书接入的显式开关节 `[feishu] enabled`。`enabled` 无默认值：当其显式缺失时，接入与否 SHALL 回退到历史隐式判定（`app_id` 与 `app_secret` 双非空即接入）；当 `enabled` 显式给出时，SHALL 以显式值为准。凭据完整性 SHALL 独立于开关校验：`app_id` 与 `app_secret` 恰好其一非空（半配置）时，无论 `enabled` 为缺省、`false` 还是 `true`，sebas SHALL 在配置校验时以配置错误拒绝启动，错误信息 SHALL 指明两者必须同时配置、或同时留空以不接入飞书；`enabled = true` 而凭据为空串属于半配置意图，SHALL 同样拒绝启动。env 覆盖（`SEBAS_FEISHU_APP_ID` / `SEBAS_FEISHU_APP_SECRET`）SHALL 发生在上述判定与校验之前：非空 env 值覆盖对应 TOML 字段，空值 SHALL NOT 覆盖；覆盖后的凭据同受本矩阵约束。启用判定仅决定 feishu 适配器是否注册进 im 服务的通道注册表（见 `channels`、`im-service` capability）；它 SHALL NOT 影响任何其他通道的注册。core 进程 SHALL NOT 读取该开关建立飞书连接（core 不再链接飞书实现）。

#### Scenario: 显式关闭时进程以 webui 主控形态运行

- **WHEN** 配置 `[feishu] enabled = false`（或缺失且凭据双空）
- **THEN** feishu 适配器不注册、im 服务不建立飞书 WebSocket 连接、不做 token 获取、不出站请求飞书 API
- **AND** watchdog 默认只启动 webui 服务，core 停用（见 `deploy-mode`）

#### Scenario: 显式开启但凭据不完整

- **WHEN** 配置 `[feishu] enabled = true` 但 `app_id` 或 `app_secret` 为空
- **THEN** 启动校验报错并拒绝启动，指明飞书凭据缺失，feishu 适配器不注册

#### Scenario: 隐式启用仍可用

- **WHEN** 配置未写 `enabled` 字段，但 `app_id` 与 `app_secret` 均非空
- **THEN** feishu 适配器仍按历史行为注册接入（向后兼容）

#### Scenario: core 单独运行时不校验飞书凭据

- **WHEN** 配置含完整 `[feishu]` 凭据，部署仅启动 core（未启用 im 服务）
- **THEN** core 正常启动且不建立任何飞书连接

#### Scenario: 凭据半配置拒绝启动

- **WHEN** `app_id` 与 `app_secret` 恰好其一非空，且 `enabled` 为缺省、`false` 或 `true` 任一状态
- **THEN** 配置校验报错并拒绝启动，错误信息指明 `app_id` 与 `app_secret` 必须同时配置、或同时留空以不接入飞书

#### Scenario: env 覆盖在判定与校验之前生效

- **WHEN** TOML 中 `[feishu]` 凭据双空，但 `SEBAS_FEISHU_APP_ID` 与 `SEBAS_FEISHU_APP_SECRET` 均提供非空值
- **THEN** 覆盖后凭据双非空，按隐式判定接入飞书
- **WHEN** env 仅提供其一（另一字段无论 TOML 还是 env 均为空）
- **THEN** 同样构成半配置，配置校验报错并拒绝启动
