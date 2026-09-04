# design — add-acp-model-selection

## Context

动机见 proposal.md——opencode 等原生 ACP agent 通过标准 `configOptions` 暴露模型列表、`session/set_config_option {configId:"model"}` 切换，sebas 接上即可免 opencode 专属代码获得会话级模型选择。已核实源码 + 真实探针：opencode 的 `session/new` 响应带 `configOptions[0] = {id:"model", category:"model", type:"select", currentValue, options:[...]}`；`SetSessionConfigOptionRequest(session_id, config_id, value)` 与 `SessionConfigOption`（`kind` flatten 的 select）在同版 `agent-client-protocol` schema 中现成可用。

关键结构事实（已核实）：
- `AcpCommand`（`sebas-acp/src/session.rs`）目前只有 Create/Continue/PermissionReply/Cancel——新增 `SetModel` 走命令通道 `cx.send_request(...)` 即可，无需新 hook。
- `DriverHandle` 的 `handshake` 信号（`add-opencode-acp` 引入）只上抛 `(routing_id, resumed)`；`configOptions` 目前被驱动丢弃。
- `CreateSessionRequest`（webui）只有 `prompt/project_dir/backend`；新建时无模型字段。
- ACP 的 `session/new`/load 响应里 `configOptions` 是**会话建立时**才有的（initialize 不带）。

## Goals / Non-Goals

**Goals:**
- 会话级模型列表（来自 agent 的 configOptions）暴露给 webui 下拉
- `SetModel` 命令 → `session/set_config_option`，失败显式报错
- create-with-model 在会话建立后、首个 prompt 前应用

**Non-Goals:**
- 不做非 ACP 驱动的模型选择（Claude 专用驱动不动）
- 不暴露 mode/effort 等其他 config id 的 UI（driver 只处理 model；其他 id 的透传留待需要时）
- 不做模型别名 / 路由（gateway-model-aliases 不动）

## Decisions

### D1 模型列表：configOptions 解析进 spawn outcome（不难而干净）

`AcpDriver` 建立会话后，把响应的 `configOptions` 里 `category == model`（或 `id == "model"`）的 select 选项解析为一个 `AcpModelList { current: String, options: Vec<String> }`，随握手信号一并上抛（`handshake` 从 `(String, bool)` 扩为 `(String, bool, Option<AcpModelInfo>)`，或并入 `SpawnOutcome`）。来源是 agent 的响应，**不硬编码**；无 model 选项 → `None`（webui 不显示下拉，不报错）。

> 与 `add-acp-session-id-mapping` 的扩展方向一致（那个 change 把 handshake 扩为 `(routing_id, resumed, acp_session_id)`）——本 change 可能与之触碰同一信号结构，实施时注意 merge 顺序；若映射 change 先归档，模型信息随 `acp_session_id` 一起扩展即可，互不冲突。

### D2 `SetModel` 走 AcpCommand 命令通道

`AcpCommand::SetModel { session_id, model_id }` 进驱动主循环匹配分支，发 `SetSessionConfigOptionRequest::new(acp_session_id, "model", model_id)`（value 用 value_id 语义）。返回结果通过现有命令通道语义：
- 成功：driver 更新本地 current model 并（可选）发 `AcpEvent::ModelChanged` 通知
- 失败（RpcError / 无效模型 / agent 无此能力）：`mgr.send` 返回错误，webui 显示"模型不可用/无效"

**为什么走命令通道而非 spawn 时参数**：create-with-model 需要"会话建立后、首个 prompt 前"应用模型——命令通道天然支持时序；且中程切换与创建共用一条路径。

### D3 create-with-model：建会话后立即 SetModel

`POST /api/sessions` 增加 `model: Option<String>`；`acp_spawn_and_activate` 成功后、首 prompt 前，若请求带 model 则 `mgr.send(SetModel)`。实现为：spawn 完成后先 flush SetModel（若有），再走既有 prompt 流程。失败 => 会话仍可对话（模型未变），webui 报非致命错误。

### D4 webui 数据流

- **快照/详情**：会话建立的 `AcpModelInfo` 存进 session 记录的 model 字段（`MappingDto` 或内存 SessionMeta），快照 API 暴露 `current_model` + `available_models`。
- **表单**：创建会话下拉数据源 = 快照里的 `available_models`（首次可在建立后回读，或由 spawn outcome 直接填充）；中程切换 = 对话界面模型选择器 → `SetModel`。
- **无模型选项的 agent（如 Claude 驱动）**：不显示模型 UI。

## Risks / Trade-offs

- [handshake 信号被两个 change 同时扩展] → 实施顺序错开（映射 change 先把 acp_session_id 加进去，模型 change 在其上加 AcpModelInfo）；或本 change 直接在最新主干上扩展，merge 时统一
- [opencode 的 set_config_option 需真实 session id] → 与映射 change 的 `acp_session_id` 联动：模型命令用真实 ACP 会话 id（而非路由 id）；映射未落地前对 uuid 会话的 set 可能失败——如实报错
- [configOptions 大列表性能] → 仅模型 select 的 options（几十条），内存/JSON 开销可忽略
- [authMethods 干扰：没登录时 opencode initialize 带 auth 方法] → 非模型路径，本期不处理（沿用现有 spawn 行为）

## Migration Plan

- 无数据迁移、无破坏性变更：新增字段全部 `Option`/`#[serde(default)]`；模型命令只对支持 config 的 ACP 会话生效
- 回滚：旧二进制忽略未知字段；`AcpCommand` 新变体对旧驱动是未知枚举项——序列化走 serde tag，旧端读到未知 tag 会反序列化失败——**与 `add-acp-session-id-mapping` 同批归档部署，避免单一回滚**

## Open Questions

- opencode 的 `set_config_option(model)` 返回值/currentValue 刷新语义：实施时用真实 opencode 冒烟确认（沿用 `docs/acp-opencode-smoke.md` 新增步骤）
- model 选择是否要按会话持久化（restart 后恢复上次模型）：与 `add-state-store` 的 sessions 表字段联动，本期先在内存层，持久化留待 state-store 落地