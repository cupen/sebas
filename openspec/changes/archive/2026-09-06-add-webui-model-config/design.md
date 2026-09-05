# Design: add-webui-model-config

## Context

模型配置相关现状(见 proposal Why):

- **provider 真相源**:状态库 SQLite(`providers` 表,软删 + JSON config),仅
  core 进程持有写路径(`sebas_state` engine);`providers.json` 是派生镜像,gateway
  启动时合并。飞书 `/provider` 卡片(`src/provider.rs`)是今天的唯一管理面:CRUD、
  probe models(`/models` 端点推导,OpenAI base_url 优先)、DIRECT 默认 provider +
  default_model 选择。
- **native 后端模型清单**:`src/agent_backend.rs` 从 `SEBAS_AGENT_MODELS` /
  `SEBAS_AGENT_MODEL` 环境变量推导,缺省硬编码 `claude-sonnet-4-5`;`info()` 透出
  `available_models`,wire-webui-sebas-agent-e2e 已把该字段接进通道快照与
  `/api/summary`。
- **WebUI**:settings-modal Models 分区只读渲染 `/api/settings` 的 providers
  (`fix-webui-detached-status` 起读状态库快照,detached 经通道 `state_snapshot`);
  composer 模型下拉(`workbench-composer.ts`)取"最近一个暴露 available_models 的
  会话"。
- **核心通道**:additive 消息集(Spawn.backend、SetModel、审批事件),有
  `state_snapshot` 只读方法;无任何状态写方法。

## Goals / Non-Goals

Goals:

- WebUI 成为 provider/模型的完整管理面(两种部署形态一致)。
- native 后端模型清单从 provider 配置推导,环境变量降级为显式覆盖。
- composer 模型下拉改为 backend 级数据源。

Non-Goals:

- 飞书卡片行为变更;gateway 路由/model_aliases 管理面;ACP 模型探测;密钥存储格式
  变更(见 proposal Non-goals)。

## Decisions

### D1 — 数据结构:复用 provider,零新存储

provider 条目已含本变更全部所需字段(api_key、base_urls、models catalog、
default_model),agent 默认选择复用既有「DIRECT 默认 provider + default_model」
语义(同一对状态,不新增第二份默认)。不建 `models` 独立表/文件。

*备选*:新建独立 models 配置 —— 否决:与 provider 的 catalog/default_model 重复,
制造第四个真相源,还要解决两套数据的同步问题。

### D2 — 写路径:core 代写,通道 additive 消息

状态库写路径只在 core 进程,detached webui 必须经核心通道。新增 additive 消息:
`ProviderUpsert` / `ProviderDelete` / `ProbeModels` / `SetAgentDefaults`,核心侧
复用 `src/provider.rs` 的既有语义(normalizer、软删清默认、probe 端点推导)落到
`sebas_state` engine。in-process 形态下 webui backend 直接调同一套逻辑,两条路径
共用一个 backend trait 层(`sebas_webui::session_backend` 增 provider 管理方法),
避免 in-process / detached 行为分叉。

probe 的 HTTP 拨号放 core 侧:网络出口统一、detached 侧无需出网权限。

*备选*:webui 直写 SQLite —— 否决:破坏「core 是唯一写权威」,detached 与 core
并发写会踩锁。*备选*:经 gateway admin API —— 否决:gateway 只读 overlay,不持有
状态库写路径。

### D3 — native 模型来源:provider 推导 + env 显式覆盖

`NativeAgentBackend` 装配时读 provider 状态:默认 provider 的 `models` catalog →
`available_models`,`default_model` → 初始模型;两者为空时维持诚实降级(缺凭据
cause 语义不变)。`SEBAS_AGENT_MODELS` / `SEBAS_AGENT_MODEL` 存在时整体覆盖推导值
(优先级:env > provider 推导),保留沙箱/测试的确定性注入通道。取代
wire-webui-sebas-agent-e2e task 2.2 的 `[agent] models` 静态配置路线(该 task 的
其余部分 —— DualSessionBackend 分发 —— 不受影响);apply 时同步改写那个 change 的
task 2.2 描述。

### D4 — UI:Models 分区重做,composer 下拉换源

settings-modal Models 分区改为管理面:provider 卡片(名称、已配置状态、base URL、
models 数、默认标记)+ 预设/自定义表单(字段与飞书表单一致,复用 preset 推导)+
probe / 删除 / 设默认。密钥输入用已有 secret 掩码组件惯例。

composer 模型下拉数据源:backend 级 `available_models`(快照已有字段,来源变为
provider 推导)优先,活跃会话的 configOptions 仅对 ACP 会话生效;下拉随快照/
事件流刷新,免手动 reload。

### D5 — 兼容与安全

- 通道消息 additive,旧客户端/旧核心不发送不消费新消息,行为不变。
- provider list 响应只回 key-configured 布尔 + 掩码,任何路径不回明文 key。
- 非 loopback bind + 鉴权开启时,provider 管理 API 与其余 mutation 同受既有
  session 鉴权门控;不新增匿名写面。

## Risks / Trade-offs

- provider.rs 的表单/normalizer 逻辑原为飞书卡形态,抽公共层时可能带出耦合 →
  抽取时以「语义函数」为单位(apply_preset_defaults、soft-delete、probe 推导),
  不搬卡片渲染。
- detached 下 probe 是同步长操作 → 通道请求带超时,probe 上限复用既有实现。
- D3 改变 native 模型清单的既有观测值(从 env 硬编码变为 provider 推导),依赖
  旧行为的测试需同步更新。
