# add-usage-statistics — Design

## Context

usage.db 由 router 独占写入（`usage_records` 一行一请求，RFC3339 UTC `ts`、`model`、四类 token 计数；保留期默认 30 天 / 20 万行）。现有一切出口都是**即时汇总**：`/admin/stats` 是进程生命周期计数（重启归零），`/metrics` 是 Prometheus 抓取面——没有任何历史时序查询。拓扑约束（router-metrics spec 明文）：WebUI 唯一服务端出边是 core channel；core↔router 通道是 router 单向订阅 core 状态，core 目前没有向 router 取数的路径。前端 Lit 3 + Web Awesome，无图表库，依赖面刻意极简。

## Goals / Non-Goals

**Goals:**

- 历史 token 消耗可查：按天（默认近 14 天）与按小时（仅当天）两种粒度、按模型分列、四类 token 明细。
- 三段链路各自职责不变：router 聚合自己的库；core 是唯一中转；WebUI 只画图。
- 桶边界按请求时区偏移切分（UTC+8 的「一天」不从早 8 点起算）。

**Non-Goals:**

- 不改 usage_records 写入路径、sink 语义、保留期机制、`/metrics`、`/admin/stats`。
- 不做 CLI 出口、日切 rollup 表、超保留期历史、图表缩放/平移。

## Decisions

### D1 数据路径：router 聚合 → core 反代 → webui 呈现

- **router**：`GET /admin/usage/timeseries`，对既有表纯 SQL GROUP BY；挂在既有 `build_admin_router` 之下，复用 `admin_auth`（Bearer 控制密钥 / 无 secret 时 loopback 放行——两种部署姿势都不用改）。
- **core**：core channel 新增一个 usage 查询请求类型（复用 `sebas-ipc` 的 `CoreChannelRequest` wire 家族，遵守协议演进三规则：新字段带 serde 默认值）；core 分支收到后经 loopback HTTP 调 router admin，Bearer 用控制密钥——core 正是密钥的自动武装方，发现复用 `sebas_ipc::secret` 共享实现（env → core.secret 文件）；router 地址取 core 同读的 config `[router] listen`（缺省 127.0.0.1:8787）。
- **webui**：`GET /api/usage/timeseries` handler（sebas-webui crate，与 `/api/settings` 同模式）把请求经 core channel 转发，原样回传聚合载荷；登录（四角色）即可见。
- 链路形态：`browser → /api/usage (sebas-webui) → core channel RPC (core) → loopback HTTP (router admin)`。webui 进程的服务端出边保持只有 core channel 一条；新增的 core→router HTTP 边落在 core 这个控制面编排者身上（它持有密钥与 config，是唯一合理的主人）。
- **否案**：core 直读 usage.db——破「router 私产」边界（跨 crate 摸表、跨进程读 WAL），且让 core 隐式依赖 router 的存储布局；webui 进程直连 router admin——webui 出边 +1，要改 router-metrics 的拓扑原则；经 core channel 反向问 router——通道方向不支持（服务端是 core）。

### D2 聚合口径：纯查询、零新表、零后台任务

天粒度 = 按请求偏移切日期桶（窗口 `days` 参数 1–30、默认 14）；小时粒度 = 仅当天 0–23 全 24 桶。两粒度都**零填充**返回完整窗口（无数据桶填 0），折线图不需补点逻辑。「次日后汇总为一天的总量」是查询口径：昨日小时明细不再单独提供，由天粒度覆盖——因此不需要 rollup 表、日切 job、清理联动。
- **否案**：日切 rollup 表（新表 + 后台 job + 与保留期清理的联动，复杂度不抵收益；历史窗口 = 原始行保留期，默认 30 天已够观察）。

### D3 时区：`tz_offset`（分钟）随查询下传

浏览器算出自己的偏移经 webui→core→router 透传；router 以该偏移计算桶边界（对 RFC3339 UTC `ts` 加偏移后取日期/小时，SQL 端用 `ts` 索引过滤窗口、应用层或 SQL 表达式分桶）。缺省 0（UTC）。偏移值 clamp 到 ±14h 合法域，非法参数 400。
- **否案**：服务器本地时区（多用户/部署漂移时桶边界与浏览器直觉不符）；UTC（UTC+8 的「一天」从早 8 点起算，违背直觉）。

### D4 分组与指标：按 `model` 分桶、四类明细、未观测不计入

聚合按记录的 `model` 列分组（操作员视角的路由模型名）；`model` 为 NULL 的记录计入 `(unknown)` 桶，保证总量诚实。每桶每模型返回 input/output/cache_read/cache_creation 四类和 + 请求数；token 为 NULL（上游错误、解析失败）自然不计入 token 和、但计入请求数。前端默认画四类之和的「总量」，可切输入/输出/缓存维度——API 一次给全明细，切换零请求。
- 顺带口径：聚合不按 `status` 过滤——成功请求才有 token 计数，失败行自然为零，无需特判。

### D5 呈现：独立 `/usage` 视图 + 侧栏入口

dashboard 是单一会话主面，不塞统计卡；settings 是配置语义。用量观察是独立的高频看数场景，值得一个路由视图（`/usage`，可深链）+ app-shell 侧栏入口。

### D6 折线图：手写 SVG Lit 组件，零新依赖

一个 `sebas-line-chart` 组件：多系列 polyline + 坐标轴 + 图例 + 悬停读数（tooltip 显示该时点各模型四类明细）。「数据 → 点位/path」抽成纯函数进 vitest；组件只做渲染。依赖面保持 lit/marked/highlight.js/dompurify/webawesome 不变。
- **否案**：Chart.js（~70KB gzip 新依赖，对内网工具链偏重）；uPlot（API 偏底层，与 Web Awesome 主题融合费力）。

### D7 降级与超时：结构化 cause、短超时、绝不拖垮面

core→router 的 HTTP 调用设短超时（5s）；router 未启用（config 无 router 段 / 连接拒绝）/ 超时 → `/api/usage` 返回结构化错误（`cause: router_unreachable` 等），webui 视图据此区分「router 不可达」与「无数据」两种空态。聚合查询本身对 usage.db 是只读 SELECT，绝不影响转发路径。

## Risks / Trade-offs

- [router 未随部署启用，usage 数据根本不存在] → 这不是缺陷是事实：视图呈现诚实空态；单测覆盖 handler 的不可达分支。
- [`days`/`tz_offset` 恶意入参] → `days` clamp 1–30、`tz_offset` clamp ±840 分钟，越界 400；SQL 参数化，无注入面。
- [大窗口聚合扫全表] → `ts` 有索引；行数受保留期双闸约束（默认 20 万行上限），GROUP BY 全表在 SQLite 上毫秒级；窗口上限 30 天封顶。
- [core channel 协议新增请求类型碰兼容面] → 遵守协议演进三规则：新 wire 字段全部带 serde 默认值；golden fixture 不动（无删改）。
- [SVG 图表可访问性/边界数据（单点、全零）] → 单点画 marker 不画线；全零返回平线；纯函数单测钉住这些形状。

## Migration Plan

纯增量：无存储迁移、无配置必填项（不配 router 的部署维持现状，只是新视图报不可达）。回滚 = 移除新端点与新视图，无残留状态。

## Open Questions

无——次要口径已记为假设：默认窗口 14 天；tooltip 展示时点四类明细；图表无缩放平移。
