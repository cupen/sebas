## Why

WebUI 工作台（项目树 + composer + turn 流）与原生内核 sebas-agent 都已可用，但两者只在 `core --webui`（本文写作时为 `run --webui`，rename-cli-surface 后更名）in-process 形态下端到端打通。watchdog 托管的正式形态（`sebas webui` 经核心通道）断在四处：核心通道 Spawn 协议没有执行体（backend）提示、没有审批消息、没有模型方法，且原生内核缺凭据不反映在 reachability 上。结果：正式部署下选 native 会静默落到 ACP 或提交时才报错，审批卡片不出现，模型下拉为空。"跑通"= 两种部署形态下 workbench → sebas agent 的创建、对话、审批、模型选择全部真实可用。

## What Changes

- 核心通道 Spawn 请求携带可选执行体提示 `backend`（`native` / `acp`，默认 `acp`），核心据此把会话路由到原生内核或 ACP 子进程，对旧客户端向后兼容
- 核心通道新增审批面：原生内核的 gated tool call 经通道推给 WebUI review card，操作员决定（allow-once / allow-session / deny）回传内核 approver，fail-closed 语义保持
- 核心通道新增会话模型方法（set model），原生内核补会话级模型选择面（available_models / current_model），composer 模型下拉对 native 会话有数据源且 set_model 不再误路由到 ACP
- `DualSessionBackend::reachability` 汇总两个执行体的状态：原生内核缺凭据/不可用时如实上报 cause，composer 门控不再只看 ACP 侧
- composer 后端下拉对不可用的执行体如实标注（不可选或注明 cause），而不是"选了提交才报错"

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `core-session-channel`：Spawn 请求增加 backend 执行体提示；新增审批请求/应答消息；新增会话模型设置方法
- `agent-workbench`：composer 可用性按执行体如实反映（含原生内核凭据缺失的 cause）；模型选择面覆盖原生会话；审批往返在 detached 形态下同样成立

## Impact

- 代码：`src/core_channel/{protocol,client,server}.rs`、`sebas-webui/src/session_backend.rs`（CoreChannelBackend）、`src/agent_backend.rs`（DualSessionBackend / NativeAgentBackend）、`sebas-agent`（会话级模型）、`sebas-webui/frontend/src/views/workbench-composer.ts`
- 协议：核心通道消息集为 additive 变化（Spawn 增可选字段，新增消息类型），旧客户端不受影响
- 部署：watchdog 注入 `SEBAS_AGENT_*` 环境变量即启用原生内核（沿用现有机制，不新增凭据通道）

## Non-goals

- 原生内核新工具能力（apply_patch、subagents）
- 项目注册表 JSON ↔ SQLite state-store 合流（另行立项）
- gateway provider 管理面配置同步给原生内核（凭据继续走 `SEBAS_AGENT_*` 环境变量注入）
- 飞书路由路径的行为变更
