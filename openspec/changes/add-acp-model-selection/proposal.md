# add-acp-model-selection

## Why

opencode 走 ACP 时会话**默认用 cwd 配置的模型**（无配置则 agent 自主选），sebas 无法为 ACP 会话选择模型，用户也用不上 opencode-go 套餐的免费模型。实测确认：opencode 的 ACP `session/new` 响应带 `configOptions`（完整 `model` 下拉，含免费模型），且标准 ACP 的 `session/set_config_option {configId:"model"}` 可切换。这是 ACP 原生能力，sebas 接上即可获得会话级模型选择，无需 opencode 专属逻辑。

## What Changes

- **`AcpDriver` 上抛模型列表**：会话建立（new/load）时把响应里的 `configOptions` 中的 model 类选项转成语料暴露给调用方（模型 id 列表 + 当前值），作为 webui 下拉的数据源。
- **新增 `SetModel` 会话命令**：`AcpCommand` 增加 `SetModel { session_id, model_id }`，`AcpDriver` 收到后发 `session/set_config_option {configId:"model", value:model_id}`；失败(无效模型/agent 不支持)显式报错。
- **webui 模型下拉**：`POST /api/sessions` 增加可选 `model`；会话详情/快照暴露该 ACP 会话的可用模型与当前模型，创建会话表单下拉选择（数据来自 driver 上抛的 configOptions）。

## Capabilities

### New Capabilities

- `acp-model-selection`: ACP 会话级模型选择——模型列表暴露、选择命令、webui 数据源与表单。

### Modified Capabilities

- `acp-driver`: 会话建立响应的 `configOptions` 上抛（模型列表数据源）；新增 `SetModel` 命令下发与错误映射。

## Non-goals

- 不做非 ACP 驱动的模型选择（Claude 专用驱动保持现状）
- 不做模型别名/路由（gateway-model-aliases 与 provider 路由不动）
- 不做 `set_config_option` 的其他 config id（mode/effort 等暂不暴露，driver 侧可透传但本期不建 UI）
- 不实现 opencode 专属逻辑（机制对任意暴露 configOptions 的原生 ACP agent 通用）

## Impact

- `sebas-acp/`：`session.rs`（AcpCommand + 新增事件/快照类型）、`acp_driver/mod.rs`（解析 configOptions、SetModel 处理）
- `sebas-webui`：`api.rs`（CreateSessionRequest + model 字段、快照暴露模型）、`session_backend.rs`（命令透传）、创建会话表单（前端）
- `src/`：`dispatch.rs`/`session_boot.rs`（SetModel 命令路由、spawn 时 model 注入）——如适用
- 依赖：无新增（`agent-client-protocol` 已含 SetSessionConfigOptionRequest / SessionConfigOption）