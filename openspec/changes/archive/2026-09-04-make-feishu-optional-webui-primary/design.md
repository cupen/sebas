## Context

See `proposal.md` — Why. Established groundwork this design builds on:

- **feishu 已是可选**（sebas-2ty）：`FeishuConfig::enabled()` 以 app_id/app_secret 双非空判定，run.rs 据此接不接飞书（WS 循环、token、出站泵、hello/test）。
- **webui 默认主控**：watchdog 默认 `webui.enabled=true`、`core.enabled=false`；webui 是独立进程 `sebas webui`，作为 core session channel 客户端，跨 core 重启存活。
- **会话状态已共享**：feishu 与 webui 都汇聚到同一个 `RouterHandle`；会话 key 是 `SessionKey { chat_id, thread_id }`（webui 用 `web-*`、原生内核用 `agent-*` 前缀）。feishu 入站→`RouterEventHandler`→`router.dispatch(FeishuIn)`。
- **原生内核当前仅 webui 可达**：`NativeAgentBackend`（`agent-*` 会话直连 `sebas_agent::session::SessionManager`）只挂在 webui 的 `DualSessionBackend` 上；router dispatch 的 `Out::SpawnAcp`/`WebSpawn` 全部走 acp 桥。飞书消息到不了原生内核。
- **出站分叉已存在**：`dispatch_out`（feishu 卡片渲染）vs `dispatch_out_without_feishu`（web 会话无卡片）两条路径。

## Goals / Non-Goals

**Goals:**
- 显式飞书开关（`[feishu] enabled`），向后兼容隐式判定。
- feishu 会话可选走原生内核：router dispatch 增加 native 分支，原生会话的权限请求经 webui 呈现、文本回包经 webui 读取。
- 验证并文档化「webui 主控 + feishu 可选」的部署形态。

**Non-Goals:**
- 不改 core session channel 协议、不动会话持久化。
- 不做飞书侧的「原生内核命令」（如 `/native` 切换)的 UI——本期只有配置/默认路由。
- 不做原生内核的飞书卡片渲染（原生会话在飞书侧静默）。
- 不动 acp 桥行为（默认执行体仍是 acp）。

## Decisions

### D1: 显式开关以 `enabled` 字段 + 缺省回退隐式判定

`FeishuConfig` 增加 `#[serde(default)] enabled: Option<bool>`；`fn is_enabled(&self)` 计算 `enabled.unwrap_or_else(|| !app_id.is_empty() && !app_secret.is_empty())`。校验：显式 `enabled=true` 但凭据不完整 → 配置错误拒绝启动；`enabled=false` 但凭据齐全 → 以显式值为准（不接入，日志提示）。`enabled()`（旧）保留为 `is_enabled()` 的别名，避免改调用点。

**Why over 纯凭据判定**：显式开关让「想关但留着凭据」和「想开但少填一个字段」两种意图可区分；`Option<bool>` 让缺省行为零破坏。

### D2: router dispatch 增加 native 执行体路由

`RouterHandle` 持有一个可选的 native 执行桥 trait（进程内指向 `sebas_agent::session::SessionManager` 句柄）。`dispatch(FeishuIn)` 走文本事件时：若该会话已是 `agent-*` 前缀（原生）→ 直接 `native.prompt()`；若是新会话且配置/默认路由为 native → 创建原生会话（`agent-*` key）并 `prompt()`，**不** emit `Out::SpawnAcp`、不渲染飞书卡片。acp 会话（默认）完全保持现状。

**Why**：复用 router 作为唯一会话权威（快照/事件广播/`agent-*` key 已贯通），feishu 与 webui 共享同一 `SessionManager` 直连，避免第二套状态。
**Alternative considered（否决）**：新增 `Out::NativeSpawn` 变体走 out 泵 → dispatch 层再调 native。缺点：给本就承载「副作用」语义的 `Out` 增加执行体选择，且 dispatch 层当前没有 native 句柄——需要把 `SessionManager` 注入 dispatch，不如在 router 决策层直接持有。

### D3: 原生会话的出站呈现全走 webui

原生会话**不**经由 feishu 出站泵：不渲染卡片、不 reactions、不 send_text。文本/工具轨迹经 `SessionEvent` 广播 + `turn_log` 由 webui（`agent-*` 会话）呈现；权限请求经 `perm_events` → webui 审查卡，决策经 `answer_permission` 回 `ApproverHub`（fail-closed）。飞书侧仅保留「已收到」回执（可选）。

**Why**：复用 `agent-*` 前缀已有的 webui 呈现路径（`NativeAgentBackend` 已实现 transcript + 审查卡 + 决策回填），零新增呈现面；也符合「飞书是辅助入口、webui 是主控」的部署语义。
**风险**：飞书用户看不到原生会话输出 → 本期接受（Non-Goals），文档写明「原生会话用 webui 看」。

### D4: 部署形态默认 webui 主控

保持 watchdog 默认（webui on / core off / gateway off）不变，作为「webui 主控」的规范默认。`run --webui`（内嵌）与 `sebas webui`（独立进程）继续共存；feishu 会话若在独立进程部署下存在，走 core 侧 router，webui 呈现。

**Why**：现状已满足，无需改默认；本 change 只验证与文档化。

## Risks / Trade-offs

- **原生会话对飞书不可见**（用户从飞书发消息却只看得到回执）→ 用 webui 查看；本期 non-goal，文档与 spec 场景写明。
- **router 持有 native 句柄引入模块耦合**（router → sebas-agent 依赖）→ 以 trait 隔离（`NativeSessionBridge`），core 无原生内核时该桥为 None，行为零变化。
- **`enabled` 缺省回退隐式判定可能造成误配**（用户以为关了但凭据还在 → 仍接入）→ 校验 + 启动日志提示当前接入状态；spec 明确「显式优先、缺省回退」。
- **双通道共享状态下 webui 会话对飞书不可操作**（已存在语义，非新引入）→ 维持现状，不新造回复目标。

## Migration Plan

- 配置：`[feishu] enabled` 为纯增量，缺省即历史行为，无需迁移。
- 部署：现有 webui 主控部署零改动；开启飞书 = 填凭据 + 显式 `enabled = true` +（watchdog 下）服务页启用 core。
- 回滚：去掉 `enabled` 字段即回退历史行为；native 路由默认关，不触发即无影响。

## Open Questions

- **OQ1**：原生会话的「已收到」回执是否要发（回执是纯飞书 outbound，不影响会话状态）——实现期按可选项处理，缺省不发。
- **OQ2**：飞书侧要不要提供「查看原生会话输出」的跳转入口（如回复 webui URL）——本期 non-goal，留待后续。
- **OQ3**：`sebas-agent` 的 LLM 凭据注入（`SEBAS_AGENT_PROVIDER_*` env）在 core 进程里如何配置——沿用 run.rs 既有 env 读取，不新增配置面。