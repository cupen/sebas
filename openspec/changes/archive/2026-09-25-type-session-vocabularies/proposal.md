# Proposal: type-session-vocabularies

## Why

会话相关的状态词汇今天全是**裸字符串**，且散落在 8 套互不相识的表示里：`SessionInfo.status` 收 `"spawning" | "active" | "dormant"`（派生 `spawn-failed`），节点侧 `phase` 另有一套七值（`spawning/active/idle/waiting_approval/exited/closed/terminated`），webui 的 `SessionStatus` 枚举带 7 个展示变体，另有 `MappingState`、`SessionLifecycle`、`NodeStatus` 各说各话。实证规模：`"active"` 作为字面量出现 **78 次、散在 22 个文件**，`"dormant"` 36 次、`"spawning"` 32 次、`"spawn-failed"` 9 次。审批决策同样有 **5 份并行枚举**（acp / webui / native bridge / agent / node-link），靠 `map_permission_decision` 之类的转换函数桥接；`TurnEntry.element_type` 的 6 个取值由生产者与消费者用字符串约定维持；`desired_mode` 在四层里都是 `String`。

后果是三重的：新增一个状态值不会让任何地方编译失败（只会在运行时漏判）；同一概念的两侧拼写可以静默漂移；`SessionStatus::derive` 靠字符串匹配推导展示状态，错一个拼写就静默降级。`add-domain-layer` 已经把「同一概念唯一定义」立为架构契约，本 change 补上它的**类型面**：定义有了，但定义里全是字符串。

## What Changes

- 会话状态词汇收敛为**单一共享 enum**：一个 `SessionPhase` 覆盖控制面与节点的取值并集（保留各自实际发送的子集），`#[serde(rename = "...")]` 保证线格式与磁盘形状**逐字节不变**；`desired_mode` / `effective_mode` 等字段改用单一 `SessionMode` 类型而非 `String`。
- **未知取值容错**：每个收敛后的 enum 都必须有未知值路径（标签化枚举沿用既有 `#[serde(other)] Unknown` 手法，裸字符串枚举用捕获变体），未知值不得导致连接失败或消息丢弃。
- **审批决策 5 份合 1**：统一为 `agent-driver` spec 已声明的四值（`allow_once` / `allow_session` / `deny` / `escalate`），各边界**保留既有实际发送的变体集**，`escalate → allow_once` 的 ACP 降级语义不变。
- **`element_type` / `kind` 收敛为 enum**，webui / im 的字符串匹配改为 `match`；未知取值照原样传递并渲染为通用块，不丢弃、不报错。
- webui 的展示状态（`SessionStatus`）**保持展示 enum**，但推导改为从 `SessionPhase` 的类型化映射，不再做字符串匹配。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

- `session-lifecycle`: 新增「会话状态词汇共享且封闭」要求——phase 与 mode 的取值集由单一共享定义承载、拼写固定、未知值容错、展示状态由类型化映射派生。
- `agent-driver`: 新增「决策词汇单一共享定义」要求——四值决策集由唯一共享类型承载（不再有 5 份并行枚举），各边界线拼写固定且未知值容错，`escalate` 降级语义不变。
- `core-session-channel`: 新增「回合内容词汇封闭且前向容错」要求——`element_type` / `kind` 取值集封闭、拼写固定，未知取值透传渲染而非丢弃。

## Impact

- **改动**：`sebas-domain`（承载收敛后的 enum）、`sebas-dispatch`（`SessionInfo.status` / `MappingState` / 卡状态映射 / `TurnEntry`）、根 crate（`src/node_link/*`、`agent_backend.rs`、`native_dispatch_bridge.rs`）、`sebas-webui`（`models.rs` 的展示推导、`session_backend.rs` 的决策枚举、`api.rs` 的请求 DTO）、`sebas-im`、`sebas-acp`、`sebas-agent`、`sebas-node`、`sebas-node-link`。
- **不变**：所有 JSON 字段名与取值拼写、NDJSON 帧、SQLite 列与取值、对外 HTTP/WS 契约。验收 = 线格式快照零差异 + `invoke testsuite-e2e` / `testsuite-acceptance` 全绿。
- **风险集中处**：反序列化从「什么字符串都收」收紧为「封闭集 + 未知容错路径」，是本 change 唯一的兼容性回退面，必须每个 enum 都有未知值测试。

## Non-goals

- **不统一控制面与节点的取值子集**——两侧今天发送的值域确实不同（节点有 `idle/waiting_approval/exited/closed/terminated`，控制面有 `dormant/spawn-failed`）。本 change 只让它们**共用同一个类型**，不强行让任一侧多发或少发一个值。
- **不动服务监督域的三个词表**（watchdog 的 `OperationStatus` / `ControlEventKind` / `ServiceState`）——那是服务生命周期而非会话，且今日无复制。
- **不改变任何取值拼写**——`"spawn-failed"` 里的连字符、`"permission_mode_result"` 里的下划线一律照旧；这是零变化基线的一部分。
- **不新增或删除任何状态值**——只把既有取值集合类型化。
- **不合并 `sebas-node-link` 的 `SessionMode` 与 `sebas-domain` 的同名概念**（今日两侧独立发布，归并留给后续评估）。
