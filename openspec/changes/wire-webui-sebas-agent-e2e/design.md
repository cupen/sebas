## Context

工作台与原生内核只在 in-process（`run --webui`）形态下打通：`NativeAgentBackend` 直接持有 `sebas_agent::session::SessionManager`，`DualSessionBackend` 按 key 前缀分发。detached 形态（`sebas webui` → 核心通道）下：

- 核心通道协议（`src/core_channel/protocol.rs`）只有 Spawn / Message / Close / Turns / Subscribe / StateSnapshot / StateMutation / StateSubscribe；`Spawn` 无 backend 字段，客户端丢弃 backend 提示（`client.rs`），模型方法缺失。
- 核心侧通道 server 直接驱动 `RouterHandle`（ACP），不经过 `DualSessionBackend`；原生内核 manager 目前只在 `run.rs` 的 webui 分支构建。
- `DualSessionBackend::reachability` 只返回 ACP 侧状态；原生凭据缺失只在 spawn 时报错。
- 原生会话 `current_model/available_models` 恒为 None；`set_session_model` 对 native key 误转发给 ACP。
- 原生凭据走 `SEBAS_AGENT_PROVIDER_BASE_URL/API_KEY` 或 `SEBAS_AGENT_GATEWAY_URL/AUTH` 环境变量（watchdog 可注入，与 `SEBAS_CORE_SECRET` 同机制）。

## Goals / Non-Goals

**Goals**

- detached 与 in-process 两种形态下，spawn(native)、对话、审批、模型选择行为一致。
- 核心通道协议变化全部 additive，旧客户端不受影响。
- 执行体可用性（含原生缺凭据 cause）如实上报给 composer。

**Non-Goals**

- apply_patch / subagents 等内核能力扩展
- 项目注册表 JSON ↔ SQLite 合流
- gateway provider 配置同步给原生内核
- 飞书路径行为变更

## Decisions

**D1 — 核心常驻双执行体，通道 server 复用 `DualSessionBackend`**
核心进程启动时（`sebas run`）无条件构建 `DualSessionBackend`（ACP + 原生，原生凭据缺失时 manager 仍建、spawn 时报 cause），通道 server 的 Spawn/Message/Close/SetModel 全部委托给它，而不是自己驱动 `RouterHandle`。这使 in-process 与 detached 共用同一条分发逻辑，也顺带消除两处行为漂移的土壤。
*备选*：通道 server 内嵌一份 native 分发逻辑 —— 两份 if-else 必然漂移，否决。

**D2 — Spawn 增加可选 `backend` 字段，默认 `acp`**
`Spawn { prompt, project_dir, model, backend: Option<String>, … }`，serde `#[serde(default)]`，旧客户端线格式不变。`None`/`"acp"` → ACP；`"native"` → 原生内核。通道会话 key 沿用 `agent-{hex}` 命名，与 in-process 一致。

**D3 — 审批走既有订阅流，回答走新请求方法**
原生内核的 PermissionRequest 作为订阅流上的新事件变体（`approval_requested`，含 request_id、工具名、摘要、选项）推送；客户端以新请求 `ApprovalAnswer { request_id, decision, reason }` 回传。不新增第二条订阅生命周期；`request_id` 幂等，迟到/重复的回答返回 typed rejection。无连接客户端时内核 approver 现有的 fail-closed 路径直接生效，核心不做缓冲重放（审批有时效性，重放旧请求比拒绝更危险）。

**D4 — 复用既有 `SetSessionModel` 请求；快照增 `backend` 字段**
实现调查修正：`SetSessionModel { key, model_id }` 已随 add-acp-model-selection 进入协议与 `CoreChannelBackend`，`SessionInfo.current_model` 也已存在 —— 缺的是 server 分发只走 ACP（native key 应路由内核）与快照缺执行体字段。因此本决策收敛为：server 的 SetSessionModel 委托 backend 缝（按 key 分发）；`SessionInfo` 增 `#[serde(default)] backend: Option<String>`，由 `DualSessionBackend` 在 snapshot/事件中转时打标（`acp`/`native`），serde default 保持旧报文兼容。

**D5 — 原生模型选择：环境变量驱动 + 内核会话级 override**
`sebas-agent` 的 `SessionTask` 增会话级模型 override（走既有 mpsc `SessionCmd`，`SessionHandle::set_model`；作用于后续 turn）；可用模型清单来自 `SEBAS_AGENT_MODELS`（逗号分隔，缺省仅含 `SEBAS_AGENT_MODEL` 的默认模型 id）——与内核既有的 `SEBAS_AGENT_*` 全环境变量配置面保持一致（修正：原设计写的是配置文件 `[agent] models`，但内核现有配置面全部走环境变量，`SEBAS_AGENT_MODEL` 本身就是 env，开新配置通道反而制造第二处配置源）。`NativeAgentBackend` 把清单暴露为 `available_models`；`DualSessionBackend::set_session_model`/`spawn_with` 按 key/hint 分发，不再无条件转发 ACP。
*备选*：从 gateway model-aliases 动态拉取 —— 依赖 gateway 在线且跨过 Non-goal，第一版用静态清单。

**D6 — reachability 扩展为按执行体的状态映射**
`reachability` 返回 `{ acp: {ok, cause}, native: {ok, cause} }`（原生：凭据缺失 → cause="no provider credentials"）。HTTP `/api/summary` 原样透传，composer 后端下拉按此渲染（不可用执行体禁选 + cause 标注）。老前端拿到多余字段不受影响。

## Risks / Trade-offs

- [审批请求在客户端重连窗口内到达 → fail-closed 拒绝，操作员意外] → 核心在事件里带明示的过期语义；前端审批卡出现前 turn 已标 rejected，操作员可重发指令。不做核心侧缓冲。
- [`SetModel`/`backend` 对旧核心二进制无效] → additive 变更，旧核心对未知请求回 typed rejection，前端如实显示；不做协议版本协商。
- [原生可用模型靠配置，可能与 gateway 别名漂移] → Non-goal 记录；配置缺省只有一个模型时下拉如实只显示一个。
- [核心常驻原生 manager 增加常驻内存/任务] → 凭据缺失时 manager 不启动 LLM 连接，仅结构体驻留，量级可忽略；沙箱实测确认。

## Migration Plan

纯 additive：合入后 watchdog 无需配置变更；要启用原生内核，watchdog 注入 `SEBAS_AGENT_*` 环境变量即可。回滚 = 回退二进制，新字段被旧核心忽略/typed rejection，无状态迁移。

## Open Questions

- `[agent] models` 配置缺省值是否需要预置多个常用 Anthropic 模型 id，还是严格只暴露已配置项（当前倾向后者，宁可下拉短也不虚列）—— 实现时不影响协议与规格。
