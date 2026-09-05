## Why

models 配置目前是错的,错在三处脱节:

1. **native 后端模型清单来自环境变量**:`SEBAS_AGENT_MODEL`(缺省硬编码
   `claude-sonnet-4-5`)+ `SEBAS_AGENT_MODELS`(逗号分隔),与操作员实际配置的
   provider(状态库里已有 models catalog、default_model)毫无关联——WebUI 上配好
   provider,native 会话的模型下拉照样是那几个硬编码值。
2. **WebUI 的 Models 设置分区是只读的**:只能看 provider 名称和 base URL,配 API
   key、增删 provider、选默认模型都必须去飞书 `/provider` 卡片。管理面缺位。
3. **composer 模型下拉的数据源是"最近一个会话暴露的 available_models"**——会话
   派生 hack,没有活跃会话时下拉为空,数据源不该是会话而是配置。

参考 deepseek-harness(dsh)的设计:Settings → Models 里按 provider 配置凭据、
状态一目了然(已配置/未配置),配置好后模型即出现在选择器中。

## What Changes

- **数据结构决策:复用 provider,不新建 models 存储**。状态库 SQLite 是唯一写路径
  (providers.json 是派生镜像),provider 已有 api_key / base_urls / models catalog /
  default_model;平行再造一套 models 配置只会制造第四个真相源。
- WebUI 服务端新增 provider 管理 API(与飞书卡片同一状态库、同一语义):provider
  CRUD(API key 掩码回显)、probe models、设定 agent 默认 provider/模型;in-process
  直连状态库,detached 经核心通道 additive 消息由核心代写——"core 是唯一写权威"不破。
- native 后端模型来源修正:`NativeAgentBackend` 的 available_models/默认模型从
  provider 状态(默认 provider 的 catalog + default_model)推导,环境变量降级为显式
  覆盖。取代 wire-webui-sebas-agent-e2e 里 `[agent] models` 静态配置的路线。
- UI:settings-modal 的 Models 分区重做为 dsh 风格管理面(provider 卡片:名称、
  已配置状态、base URL、key 掩码、models 数、默认标记;预设/自定义新增编辑表单;
  probe;设默认);composer 模型下拉改从 backend 级 available_models(provider
  catalog)取,ACP 会话维持 agent configOptions 来源。

## Capabilities

### New Capabilities

- `model-settings`:WebUI 的 provider/模型管理面——管理 API、Models 设置 UI、
  native 后端模型清单的配置推导。

### Modified Capabilities

- `core-session-channel`:新增 provider 管理 additive 消息(CRUD / probe /
  设默认),detached webui 借道核心写状态库。
- `agent-workbench`:composer 模型下拉的数据源语义改为 backend 级(不再依赖
  活跃会话),native 数据源来自 provider catalog。

## Impact

- 代码:`sebas-webui/src/api.rs`(新路由)、`src/core_channel/{protocol,server,client}.rs`
  (additive 消息)、`src/agent_backend.rs`(模型来源)、`sebas-webui/frontend/src/views/
  settings-modal.ts`、`workbench-composer.ts`、`src/provider.rs`(probe 逻辑复用)
- 协议:核心通道消息集 additive,旧客户端不受影响
- 关联 change:wire-webui-sebas-agent-e2e 的 task 2.2(`[agent] models` 配置源)被本
  变更取代,apply 时需同步改写该 change 的任务描述

## Non-goals

- 飞书 `/provider` 卡片的行为变更(两前端共用状态库,飞书侧不动)
- gateway 路由 / model_aliases 管理面(已有 gateway-admin-api spec)
- ACP 子进程的模型目录探测(ACP 模型清单仍来自 agent configOptions)
- 密钥存储格式变更(沿用 overlay/状态库现状,掩码回显)
