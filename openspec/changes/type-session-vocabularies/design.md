## Context

动机与范围见 `proposal.md` — Why / What Changes。设计只需要以下约束：

1. **`add-domain-layer` 已提供承载处**：类型化后的 enum 落 `sebas-domain`，原 crate 用 `pub use` 再导出。本 change 依赖它（见其 design D3/D4）。
2. **零变化基线仍然生效**：所有取值拼写、JSON 字段名、NDJSON 帧必须逐字节不变。本 change 只把**既有取值集合类型化**，不新增、不删除、不改拼写。
3. **反序列化形态今天是最宽松的**：`status`、`phase`、`mode`、`element_type` 都是 `String`，什么都能收。收紧为封闭集是本 change **唯一的兼容性回退面**。
4. **`agent-driver` spec 已声明决策词汇**（`allow_once` / `allow_session` / `deny` / `escalate`）与 ACP 降级规则；本 change 不改语义，只让这四值由**唯一类型**承载。今日 5 份并行枚举：`sebas_acp::Decision`（3 值）、`sebas-webui::session_backend::PermissionDecision`（4 值）、`NativeApprovalDecision`（4 值）、`sebas-agent::ApprovalAnswer`（4 值）、`sebas-node-link::ApprovalDecision`（3 值）。
5. **既有容错先例**：`sebas-node-link` 的 `RejectCode` / `SessionRejectCode` 已用 `#[serde(other)] Unknown` 表达前向兼容，本 change 沿用同一手法。

## Goals / Non-Goals

**Goals:**

- 让「新增一个状态值」从**运行时静默漏判**变成**编译期报错**。
- 让「同一概念只定义一次」在词汇层面成立（`add-domain-layer` 立了架构契约，本 change 补上类型面）。
- 在收紧类型的同时**不缩窄实际接受的输入**：未知值必须有一条明确路径。

**Non-Goals:**

- 不统一控制面与节点的**取值子集**（两者值域确实不同，见 proposal Non-goals）。
- 不做取值语义的重新设计（例如把 `dormant` 与节点 `idle` 归一）——那是行为变更。
- 不触碰 watchdog 的三个词表、不触碰展示层 enum 的取值集。

## Decisions

### D1 一个 enum 覆盖控制面与节点的**取值并集**，不做两侧各一个

- **理由**：本 change 的目标是「同一概念唯一定义」。若控制面一套、节点一套，两者之间立刻需要一个转换函数——那正是本 change 要消除的东西（今日 `map_permission_decision`、`summary_of`、`row_of` 就是这么长出来的）。
- **代价**：类型不强制「角色只能发自己的子集」，靠文档与生产端约定。**接受这个代价**，因为没有实例数据表明角色越界发送过。
- **被否备选**：`ControlPlanePhase` + `NodePhase` 两个 enum + 转换表。转换表本身就是漂移面，否决。

### D2 未知值容错按枚举形态选机制，逐枚举用测试钉住

- 已带内部/邻接标签的枚举可沿用既有 `#[serde(other)] Unknown`（`RejectCode` 是仓库内先例）。
- 外部标签的裸字符串枚举（如 `"spawning"` 直出）不能直接用 `other`，需要捕获变体（`Unknown(String)`，保留原值以便回显与诊断）或手写 `Deserialize`。
- **实现时逐枚举选定并在 design 落地处注明**；每个枚举**必须**有一个「喂未知值 → 不失败且可观测」的测试。这是本 change 唯一的硬性防护。
- **被否备选**：全部严格封闭（未知值即报错）。理由：节点与控制面是独立发布的二进制（节点有独立发布节奏），严格封闭会把一次节点升级变成控制面的连接中断——与 `node-session-channel` 既有「前向兼容的拒绝码」设计意图相悖。

### D3 展示层 enum 保持展示层

webui 的 `SessionStatus`（`Starting/Queued/Working/Done/Failed/Dormant/Waiting`）是**派生视图**，不是线值，取值集不变。改变的只是推导方式：从 `SessionStatus::derive` 的字符串匹配（今日匹配 `"spawning" | "dormant" | "spawn-failed"` 与卡片 phase `"OnIt" | "DONE" | "CrossMark"`）改为对 `SessionPhase` 的类型化 `match`。**收益**：新增 phase 时展示映射漏写会编译失败，而不是静默落到默认标签。

### D4 `SessionMode` 定义移入 `sebas-domain`，`sebas-node-link` 原位再导出

`SessionMode` 今日已是枚举，但住在 `sebas-node-link`（链路契约 crate）。domain 不该依赖链路契约（依赖方向反了），故定义移入 domain，node-link `pub use`——与 `add-domain-layer` 的 D3 战术一致，node-link 的公开 API 不变。根 crate 与 webui 的 `desired_mode` / `effective_mode` / `SetSessionModeRequest.mode` 从 `String` 改为该类型。

### D5 决策词汇合一**不改变任何边界实际发送的变体集**

5 份枚举的值域确实不同（ACP 侧 3 值、节点侧 3 值、webui/native/agent 4 值）。合一为 4 值类型后，**发送集不动**：控制面发给节点/ACP 的仍只是对方今天能收的值，`escalate` 到 ACP 仍降级为 `allow_once`（`agent-driver` 既有规则）。**风险与防护**：加一个测试断言各边界的序列化输出集合与合一前一致——否则统一类型会静默扩大发送面，而接收侧旧二进制会因此反序列化失败。

### D6 不新增、不删除、不改拼写任何取值

零变化基线的直接推论。实现中发现某个取值只在一侧存在、语义可疑时，**记录进本文件而不删**。特别注意连字符与下划线：`"spawn-failed"`、`"permission_mode_result"` 必须逐字保留。

### D7 排除服务监督域词表

watchdog 的 `OperationStatus` / `ControlEventKind` / `ServiceState` 属于服务生命周期而非会话，且今日各只有一份定义（无复制），不在本 change 射程。**触发条件**：若未来 watchdog 与会话词表出现共享需求（例如会话状态派生自服务状态），另立 change 评估。

## Risks / Trade-offs

- **[反序列化收紧导致兼容性回退]**（本 change 头号风险）→ 每个 enum 必须有未知值路径与对应测试；黄金样本取自「今日代码里出现过的全部字面量 + 历史上线上出现过的取值」，逐值回归。禁止任何 enum 出现「未知值 → 报错/丢帧」。
- **[统一决策类型静默扩大发送面]** → D5 的发送集不变测试；PR 描述里附合一前后各边界的序列化输出对比。
- **[拼写漏字]**（`spawn-failed` 写成 `spawn_failed`、`permission_mode_result` 写错）→ 字面量清单先审计（tasks 1.1），每个取值一个钉测试；线格式快照做整体闸门。
- **[`Unknown(String)` 保留原值带来 `PartialEq`/`Hash` 语义变化]** → 保留原值的变体在比较时按「原值相等」处理，不做归一；需要归一的调用点显式处理。实现时用测试钉住。
- **[改动面广（22 个文件含 78 处 `"active"` 字面量）]** → 编译器是主闸门：类型变了，未改的调用点直接编译失败，不存在漏改；分枚举、分 crate 提交。

## Migration Plan

无数据迁移（取值拼写不变，磁盘里的值照旧可读）。顺序：

1. 审计取值清单与字面量分布，产出黄金样本（tasks 1）。
2. 逐个 enum 落地：先定义（含未知值路径与单测）→ 再切换生产端 → 再切换消费端 → 删字符串常量。同一 enum 一个提交。
3. 决策词汇：先合一类型并保留各边界转换（发送集不变），再删 4 份并行定义。
4. 回合内容词汇：`element_type` / `kind` 同法。
5. 全量回归：线格式快照 + `invoke testsuite-e2e` + `invoke testsuite-acceptance`。

**回滚**：逐 enum 独立提交，回滚 = revert 该 enum 的提交；无持久化状态需要回滚（磁盘存的是拼写，未变）。

## Open Questions

- `Unknown` 变体是否需要保留原始字符串（`Unknown(String)`）取决于是否需要把未知值回显给操作员与写进日志。今日无此需求，但保留成本极低——实现时按「先保留」处理，若造成 `PartialEq` 复杂度超出收益，再退化为无载荷的 `Unknown`。这不改变 spec（要求只说「显式标记的未知值」）与任务分解。
