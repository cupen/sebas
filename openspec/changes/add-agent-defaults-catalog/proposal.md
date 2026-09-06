# Add Agent Defaults 与 Backend Catalog 选择器（移植自被取代的 add-webui-model-config）

## Why

集成取舍时（见 `archive/2026-09-06-add-webui-model-config/SUPERSEDED.md`），provider
管理实现采用上游 `refactor-provider-data-model`（providers.json 单一真相源、三协议
槽位、WebUI 正式 provider 管理页），被 drop 的 `add-webui-model-config` 里有**两项上游
至今没有的独有能力**需要在新架构上重建：

1. **agent defaults**：操作员可以为新会话设定默认 provider 与默认 model——上游 composer
   的模型下拉只能从「最近一个暴露 `available_models` 的会话」推导，没有任何"设默认"的入口。
2. **backend catalog 选择器**：模型下拉在**任何会话存在之前**就应提供 backend 级模型
   目录（由 provider 配置推导），而不是依赖"恰好有个会话暴露过模型列表"。

## What Changes

- **Router admin**：新增 defaults 读写面（GET/PUT），持久化默认 provider 与默认
  model；与 providers CRUD 同一控制秘密与认证域。
- **WebUI BFF**：代理 defaults 读写（POST-only 写、GET 读），沿用既有错误状态码透传。
- **Composer**：模型选择器数据源优先级——会话存在时沿用会话 `available_models`（ACP
  agent 声明优先）；无会话时取 defaults 指向 provider 的 catalog（provider 管理页 probe
  所得，preset 派生跟随代码表）；两者皆无时如实显示不可用。
- **Provider 管理页**：增加"设为默认"动作（对选中 provider / model）。

## Capabilities

### New Capabilities

（无——全部落在既有能力的需求扩展上）

### Modified Capabilities

- `agent-workbench`：composer 模型选择器数据源需求——无会话时从 backend catalog
  （defaults 指向的 provider）取选项，而非仅从既有会话推导；如实上报不可用。
- `webui`：HTTP route surface 增加 agent defaults 读写端点（GET/PUT，BFF 代理）。
- `provider-management`：provider 管理页增加"设为默认"动作的需求。

## Impact

- `sebas-router`（admin API：defaults 端点 + 持久化）、`sebas-webui/src/routes.rs`
  （BFF 代理）、`sebas-webui/frontend`（settings-modal 设默认动作、workbench-composer
  选择器数据源）。
- 兼容性：纯新增面，无既有行为变更；defaults 未设置时 composer 行为与现状完全一致。

## Non-goals

- 不改 provider 数据模型与真相源（维持 providers.json + router admin）。
- 不做会话级模型 override（已有 wire-webui-sebas-agent-e2e / acp-model-selection 覆盖）。
- 不引入状态库写路径（被取代方案的通道 op 不复活）。
