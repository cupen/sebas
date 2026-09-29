# add-usage-statistics

## Why

usage.db 里每一笔请求的 token 消耗已经在逐行落库（模型、四类 token 计数、时间戳），但没有任何历史聚合出口：`/admin/stats` 只有进程生命周期内的即时汇总（重启归零），操作员看不到「各模型每天/每小时消耗了多少 token」。本变更把存量数据变成按天/按小时的时序统计，并在 WebUI 以折线图呈现。

## What Changes

- router 新增聚合查询端点 `GET /admin/usage/timeseries`：对既有 `usage_records` 按**天**（默认近 14 天，参数 1–30）与**小时**（仅当天）两种粒度分组聚合，按模型分列，返回 input/output/cache_read/cache_creation 四类明细；天/小时桶按请求下传的时区偏移切分。纯 SQL GROUP BY，**零新表、零后台汇总任务**——「次日后小时数据汇总为一天的总量」是查询口径，历史小时明细不再单独提供。
- core 新增 `GET /api/usage/timeseries`：按需经 loopback HTTP 反代 router 的聚合端点（Bearer 控制密钥，与 admin_auth 既有姿势一致）；router 未启用/不可达时返回结构化 cause，不影响其余 API。
- WebUI 新增独立 `/usage` 视图 + 侧栏入口：天/小时粒度切换、按模型多折线（手写 SVG 组件，零新依赖）、token 维度切换（总量/输入/输出/缓存）、router 不可达的诚实空态。登录即可见（四角色一致）。

## Capabilities

### New Capabilities

- `usage-statistics`: usage 的时序聚合查询面——router 聚合端点的桶切分与分组口径、core 反代的取数路径与降级语义、WebUI 折线图呈现与空态。

### Modified Capabilities

<!-- 无：/metrics、/admin/stats、usage 写入路径与保留期机制全部原样。 -->

## Impact

- `sebas-router`：usage 查询模块（聚合 SQL + admin 路由挂载）；不改 `usage_records` 写入路径与 sink 语义。
- `src`（core）：webui API 新增 usage 反代 handler（HTTP client 至 router admin，携带控制密钥）。
- `sebas-webui/frontend`：新 `/usage` 视图、SVG 折线组件、侧栏入口；无新 npm 依赖。
- `sebas-ipc`：webui→core 的一跳经此边界（新增 `UsageTimeseries` wire 变体，全字段带 serde 默认值）；core→router 是 loopback HTTP admin 面，不经 ipc。

## Non-goals

- 不做 CLI 用量出口（`sebas usage` 留给后续 change）。
- 不做日切 rollup 表与超保留期的历史统计（历史窗口 = 原始行保留期，默认 30 天）。
- 不改 `/metrics`、`/admin/stats` 与保留期清理机制。
- 折线图不做缩放/平移等重交互。
