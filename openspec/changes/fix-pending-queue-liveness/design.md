## Context

排队机制（workbench-turn-queue，已归档）的骨架保留：卡片 FSM WORKING → `enqueue_turn`（back-pressure）；终态（DONE/FAILED）事件到达时 `drain_queue_if_terminal` 弹出队头。关键事实（本次 review 实测代码得出）：

- `submit_turn` 的 in-flight 判定与 drain 只在 `sebas-dispatch/src/engine/mod.rs` 一处，判定源是 `card_states.status_emoji`。
- 泊车审批（permission 泊车、review-card）期间卡片仍 WORKING，但 webui 呈现层把它改名 `waiting`（`models.rs` `with_parked_approvals`）；前端 `turnInFlight = status_slug === 'working'`（`dashboard.ts:852`）——呈现词与后端判据脱节。
- pending 管理的类型化拒绝（`PendingOpError`：Unknown/AlreadyStarted/OutOfRange/PriorityConflict → 404/409/400/409）链路完好，前端 `pending-stack.ts` 对所有失败统一 `reconcileSilently()`（原 design D8 决策），用户零感知。
- claude 驱动自带 hang 升级链（5m 静默 → interrupt ×3 → SIGTERM → terminal Error），但只在子进程维度；引擎维度对「事件流断了但子进程活着」「泊车审批无人应答」两类停滞无兜底。

用户未参与轮次问答，以下决策为默认裁决（按 review 推荐执行，工件评审时可推翻）。

## Goals / Non-Goals

**Goals:**

- 队列前进不再单一依赖终态事件：停滞可检测、可自愈、可禁用、有通知。
- 「turn 在飞」在呈现层与后端判据合一，waiting/starting 态提交不再伪装成直接发送。
- pending 操作的确定性拒绝可见，竞态竞输保持静默。

**Non-Goals:** 见 proposal；另加——不重构卡片 FSM、不动 `PendingOpError` 类型化拒绝的 API 形状。

## Decisions

- **D1 停滞看门狗放引擎层，不放驱动层。** 驱动（claude）已有自己的 hang 升级链，但通用 ACP 驱动与事件路由层（channel 断连、引擎 bug）不在驱动职责内；引擎是唯一同时看得到「卡片态、泊车态、事件到达」的位置。实现为每个活跃 WORKING 会话记录 `last_event_unix`，引擎已有周期 tick（复用 watchdog/状态巡检节奏）扫描，不动事件热路径。备选「驱动各自上报心跳」被否：三处驱动各写一遍、事件路由断线场景覆盖不到。
- **D2 泊车豁免按引擎事实判，不按呈现词判。** 引擎知道「有无泊车中的审批请求」（`parked approvals` 已在快照里），泊车中不计时；解除（批复或取消）后重新计时。备选「泊车也计时但阈值放宽」被否：无人值守过夜场景必然误伤。
- **D3 呈现层「在飞」改为消费结构化事实，不猜 slug。** 会话快照加一个直白字段（如 `turn_engaged: bool`，WORKING ∨ 泊车 ∨ spawn 窗口），dashboard 直接透传给 composer；`waiting` 等呈现词继续服务徽标配色。备选「前端把 waiting 也算在飞」被否：slug 语义是展示词表，下次再加一个词（如 degraded）又会漏——呈现词表不该承担状态判定。
- **D4 拒绝反馈用两级判据，不用异常类型单判。** 前端操作后已拿回服务端全量 pending（既有对账通道）：条目已不在 → 竞态竞输，静默；条目仍在或请求 4xx/网络失败 → 确定性拒绝，走 notice-layer 低档（info/warn）就地短暂呈现，点名条目文本与原因词。备选「所有拒绝都通知」被否：快速连点时的并发竞态会刷屏。
- **D5 停滞收尾走既有非终端 Error 的收尾通道。** 引擎把停滞回合按「回合异常结束、会话存活」处理（对齐 `AcpEvent::Error { terminal: false }` 臂的既有语义：SEED/WORKING → DONE + drain），再发 warn 通知。不引入新事件类型。
- **D6 阈值默认 600s、`[dispatch] turn_stall_timeout` 可配、0 关闭。** claude hang 链的 5m 静默先于看门狗触发（子进程真挂时轮不到看门狗），看门狗兜的是驱动判定不了的场景，10 分钟是「真在跑但完全无事件」的保守上界；流式回合事件密集，不会误伤。

## Risks / Trade-offs

- [看门狗误杀超长无事件回合（如 agent 侧超长本地计算不上报）] → 无事件 10 分钟在 ACP 语义里已属异常（工具调用都会出帧）；日志会记收尾前最后事件时间便于归因；阈值可调/可关。
- [泊车豁免被滥用（agent 永远泊着一个审批不响）] → 泊车本身在 transcript 有 review-card 可见，停止按钮在泊车态可达（本 change），操作者有出口；不算看门狗职责。
- [turn_engaged 字段增加快照形状] → 纯加法，旧前端忽略新字段不破（对齐既有 legacy 载荷兼容惯例）。
- [两级判据在「服务端已收敛但条目恰好被别的操作删掉」场景下静默] → 服务端真相为准是既有对账语义，符合预期。

## Migration Plan

纯加法 + 行为修正，无数据迁移。看门狗默认开启（600s），升级即生效；回滚 = 配 0 或回退二进制。前端在 core 未升级组合下读不到 `turn_engaged` 时回退旧 `status_slug === 'working'` 判定（字段缺省 = 旧行为）。

## Open Questions

（无——诊断性任务（钉死用户实例具体触发路径）列入 tasks 首项，但不阻塞本设计：修复对三类停滞根因均有效。）
