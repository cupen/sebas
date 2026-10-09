# add-local-usage-statistics — Design

## Context

usage 三条轨迹互不相通：router 侧已建账（`usage.db` 独占 + `/admin/usage/timeseries` + core 反代 + /usage 折线图）；ACP 会话有 `TurnUsage`（model + 四类 token，`AcpEvent::UsageUpdate`）但只累计进 dispatch 的内存卡片态（feishu footer / webui 会话芯片），不落盘；native 内核两条上游线（provider 直连 / 注入 `SEBAS_AGENT_ROUTER_URL` 走 router）但**回合不解析 usage**（快照恒 None）。类型形状三套：router `UsageRecord`（超集）、ACP `TurnUsage`（缺 ts/status/provider）、channels `AppUsage`（展示态，仅 in/out）。

## Goals / Non-Goals

**Goals:**

- 一套域类型 + 一套聚合实现，router 与 core 两侧共用，同输入同桶形。
- 本地源补全：不经 router 的回合（ACP 全部 + native 直连）落 `usage_local.db`，时序可查。
- `/api/usage/timeseries` 单端点三口径（router/local/all），UI 来源切换。

**Non-Goals:**

- 不改 router 侧端点形状、sink 语义、保留期机制；不迁移内存芯片的展示语义。
- 不做跨库逐条去重（写入侧规则规避双算）；不做 CLI；图表不加缩放。

## Decisions

### D1 「本地」的边界：不经 router 的回合

ACP 会话的 agent 自己拨上游、永不经过 router → **总是本地记**。native 有两条线：注入 router URL 时经 router（router 已记）→ **不本地记**；provider Off 直连 → **本地记**。core 在 spawn 时就知道是否注入了 `SEBAS_AGENT_ROUTER_URL`，规则可在写入侧零歧义执行——一条请求恰好被一个源计数。
- **否案**：全量本地记 + 查询端跨库去重（需要逐条对账，复杂且不可靠）；仅 ACP（native 部署的本地统计继续空谈，双源只剩一半）。

### D2 本地账本：core 独占的 `usage_local.db`

新库进 sebas home 映射表（`StatePath` 族扩充 + `SEBAS_USAGE_LOCAL_DB` 显式覆盖），连接配方/schema 同步/单写 actor 全取自 `sebas-db`——router 的 `open_and_sync` 模式原样复用，core **绝不打开 router 的 usage.db**（单写者红线）。表注册 DDL 落 `src/sebas_state`（工作区唯一手写 DDL 处）。行 struct `LocalUsageRow` 定义在 core 侧消费模块（归属按写入者）。保留期双闸默认与 router 侧同值（30 天 / 20 万行 / 每小时），独立可配（`usage_local_*` 键族），清理计数写日志——与 `persist-router-usage` 的既有取舍对齐。
- **否案**：projects.db 加表（高频追加与低频映射混居、清理策略耦合）；写 router 的 usage.db（违反一个文件一个写入者）。

### D3 中立类型落点：`sebas-domain` 新增 usage 域类型

`UsageRecord` 形状（ts/model/provider/status/四类 token/latency/error）与时序请求/响应类型提升进 `sebas-domain`，`sebas-router` 改为 re-export（wire 形状零变化，`ipc-protocol-home` 兼容面不动）。聚合纯函数（记录集 → 桶结构，含 tz 切桶/零填充/分模型小计）一并下沉 domain，router 与 core 各自的查询端只做 IO 编排。准入检查：≥2 crate（router + core）+ 角色中立 ✓；行 struct 仍归各自写入者 crate（既有规则不破）。
- **备选**：类型留 router 让 core 依赖 sebas-router——方向颠倒且把进程实现耦进域层，否。

### D4 查询口径：单端点 `source` 参数 + 尽力合并

`/api/usage/timeseries?source=router|local|all`（默认 all）。`source=router` 保持既有反代语义（不可达 → 结构化 cause）；`source=local` 纯本地聚合（router 不参与）；`source=all` = 本地聚合 + 尽力反代合并，**router 不可达时仍 200**——本地数据照常返回，响应携带 `router_cause` 如实标注缺席源（写入侧无重叠保证合并即无重算）。桶内每模型行带 per-source 小计（`router`/`local` 两个字段），UI 的来源切换只是换参数，不换组件。
- **否案**：两个端点（复用度差，前端承担合并）；all 模式 router 不可达报整页错误（本地数据被 router 故障连坐，违背「降级不拖垮」）。

### D5 本地落账钩子：逐回合一行，回合完成时写

落账点在 core 的回合结算路径（与 `UsageUpdate` 事件同源）：每个 agent 回合完成 → 一行（ts=回合完成时刻，model=上报模型，provider=best-effort 上游名/留空，status=回合终态，四类 token，latency）。写入走 `usage_local.db` 的单写 actor + 有界通道，满则丢弃 + warn——**统计旁路语义与 router sink 一致，绝不阻塞或失败在途回合**。native 内核新增上游响应 usage 解析（Anthropic `usage` / OpenAI `usage` 两形状），填进与 `TurnUsage` 同构的中立类型随回合事件上报；`usage_reported` 门控语义照旧（从未上报不冒充全零）。

### D6 UI：来源切换器

/usage 视图在粒度切换旁加「全部 / router / 本地」来源切换（默认全部）；`source=all` 且响应带 `router_cause` 时，图表照常渲染本地数据 + 顶部「router 部分不可达」局部警示条——不是整页空态。汇总卡随当前口径出数。

## Risks / Trade-offs

- [ACP usage 上报本身不完整（部分 agent 报 context/cost 非 token）] → 沿用 `usage_reported` 门控：未上报不冒充零；本地行仍记请求数，聚合诚实呈现。
- [双写时序差（回合完成时刻 vs router 完成顺序）导致 all 口径的桶间微差] → 两侧 ts 都取完成时刻，误差为毫秒级边缘桶漂移；写入侧单源规则保证总数不多算。
- [domain 下沉改动 router 的类型来源，碰兼容面] → re-export 保持 wire 逐字不变；golden fixture / 协议契约测试不红为准。
- [usage_local.db 高频追加] → 单写 actor 批量提交 + 保留期双闸，与 router 侧同款容量上限，SQLite 追加负载已被 usage.db 实证可承受。

## Migration Plan

纯增量：新库随 core 首启在 sebas home 内创建；不配 router 的部署从此获得本地统计；既有 router 部署零迁移。回滚 = 移除新端点参数与落账钩子，删除新库文件即彻底清除。

## Open Questions

无——次要口径记为假设：provider 列 best-effort；保留期键族 `usage_local_*` 独立可配；来源切换不做 URL 深链（视图内状态）。
