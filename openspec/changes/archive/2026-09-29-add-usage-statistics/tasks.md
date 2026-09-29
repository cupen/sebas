# add-usage-statistics — Tasks

## 1. Router 聚合端点

- [x] 1.1 桶切分纯函数：输入记录集 + granularity + days + tz_offset，输出零填充窗口的桶结构（日期桶 / 当天 0–23 小时桶、按 model 分组含 `(unknown)` 桶、四类 token 求和 + 请求数、token None 不计入）；单测覆盖跨模型分组、UTC 23:00 记录在 +8 落次日桶、None 不计入、单点与全零窗口——`cargo test -p sebas-router`
- [x] 1.2 聚合 SQL（参数化、`ts` 窗口过滤走既有索引）+ `GET /admin/usage/timeseries` 挂载进 `build_admin_router`（复用 `admin_auth`）；测试：非 loopback 无 Bearer 401、双模型日聚合与既有场景一致——`cargo test -p sebas-router`
- [x] 1.3 参数口径：granularity 非法 400、days clamp 1–30 缺省 14（hour 忽略 days）、tz_offset 缺省 0 且 clamp ±840；测试逐条钉住——`cargo test -p sebas-router`

## 2. core 反代链

- [x] 2.1 sebas-ipc 新增 usage timeseries 的 wire 请求/响应类型（新字段全带 serde 默认值、无删改）；跑 `cargo test --test ipc_protocol_contract_test` 确认 golden fixture 零漂移
- [x] 2.2 core 分支：收到该请求后 loopback HTTP 调 router admin——地址取 config `[router] listen`（缺省 127.0.0.1:8787）、Bearer 控制密钥复用 `sebas_ipc::secret` 发现、5s 超时；router 未启用/拒绝/超时映射为结构化 cause；单测用不存在的端口断言 cause 与 promptly 返回——`cargo test`（core 侧测试）
  - 勘误（3a/3c）：Bearer 实读 core 自身 env `SEBAS_CONTROL_SECRET`（router admin 鉴权无 `sebas_ipc::secret` 文件形态，见 `src/router_admin.rs` 注释）；「router 未启用」以 config 是否显式声明 `[router]` 段为准（`declares_router_section`，非 parse 成败）。

## 3. WebUI API 面

- [x] 3.1 `GET /api/usage/timeseries` handler：登录（四角色）即可见、参数透传、聚合载荷与 cause 原样回传；测试：未登录被既有鉴权拦、router 不可达时返回结构化 cause 且其余 API 不受影响——`cargo test -p sebas-webui`

## 4. WebUI 前端

- [x] 4.1 `sebas-line-chart` SVG 组件：「数据→点位/path」纯函数（多系列折线、单点画 marker、全零平线、坐标轴刻度）进 vitest；组件渲染图例与悬停读数（该时点各模型四类明细）——`pnpm test`（frontend）
- [x] 4.2 `/usage` 视图 + 路由 + app-shell 侧栏入口：天/小时粒度切换、token 维度切换（总量/输入/输出/缓存）、窗口汇总数字、「router 不可达」与「无数据」两种空态分开呈现；视图测试覆盖切换与空态——`pnpm test`（frontend）

## 5. 联调与回归

- [x] 5.1 沙箱全链路验证（AGENTS.md 菜谱：`--debug` router + fake-claude 造真实 usage 行）：`/api/usage/timeseries` 天/小时聚合与落库行对账、UI 折线渲染两种粒度、停掉 router 后视图呈「不可达」空态；native 模型场景可用 `test/long` 多跑几回合造量
- [x] 5.2 回归：`cargo test` 全绿 + 前端 `pnpm test` 全绿；`invoke testsuite-e2e` 不红
