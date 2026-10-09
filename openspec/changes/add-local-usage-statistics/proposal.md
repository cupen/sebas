# add-local-usage-statistics

## Why

usage 统计今天只有 router 单源：`usage.db` 只记「经 router 的请求」。不经 router 的回合——ACP 会话（claude-code 等自己拨上游）与 native 直连（provider Off）——的消耗要么只在内存（ACP 的 `TurnUsage` 累计进会话芯片，重启即失、无时序），要么根本没被解析（native 内核快照 `usage` 恒 None，上游响应里的 usage 字段被丢弃）。三处类型形状各异（router `UsageRecord` / ACP `TurnUsage` / 展示态 `AppUsage`），无法同一口径出数。

## What Changes

- **中立 usage 域类型**（`sebas-domain`）：统一回合用量记录（ts/model/provider/status/四类 token/时延）与时序查询请求/响应形状，router 与 core 共用；各写入者的行 struct 仍归各自 crate（既有归属规则不变）。
- **本地落账**（新能力 `local-usage-capture`）：core 新建独占写入的 `usage_local.db`（sebas home 映射表收编，env 可覆盖，保留期双闸默认与 router 侧同值）；ACP 回合的 `UsageUpdate` 逐回合落一行；native 内核本期解析上游响应的 usage 字段，同形状落账；**双算规避**——native 经 router（注入了 `SEBAS_AGENT_ROUTER_URL`）的回合不本地记账（router 已记）。
- **查询面双源化**（修改 `usage-statistics`）：`/api/usage/timeseries` 加 `source` 参数（`router|local|all`，默认 all 合计），桶内每模型行带 source 细分小计；core 聚合本地库 + 按需反代 router 合并（router 不可达时本地源照常出数）；一套聚合实现跑两个库。WebUI /usage 视图加来源切换（全部/router/本地）。

## Capabilities

### New Capabilities

- `local-usage-capture`: 不经 router 的 agent 回合用量落账——ACP UsageUpdate 持久化、native usage 解析、双算规避规则、本地库归属（core 独占）与保留期。

### Modified Capabilities

- `usage-statistics`: 查询面从 router 单源扩为双源——`/api/usage/timeseries` 加 source 维度与合并语义；core 从「反代 router」扩为「本地聚合 + 反代合并」；WebUI 视图加来源切换。

## Impact

- `sebas-domain`：新增 usage 域类型（两 crate 共用的准入已满足）。
- `sebas-acp` / `src`（core）：UsageUpdate 挂落账钩子；`sebas-agent`：native 回合解析上游 usage；`src/sebas_state`：usage_local 表注册（手写 DDL 唯一处）。
- `sebas-router`：聚合实现泛化供 core 复用（或 domain 承载聚合纯函数），`/admin/usage/timeseries` 语义不变。
- `sebas-webui/frontend`：/usage 来源切换。

## Non-goals

- 不改 router 侧既有端点形状、sink 语义与保留期机制。
- 不做跨库逐条去重（双算以「经 router 的回合不本地记账」规则在写入侧规避）。
- 不做 CLI 出口、图表缩放/平移。
- 不迁移既有内存累计的展示语义（feishu footer、会话芯片原样）。
