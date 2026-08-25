# Proposal: gateway-admin-api-and-model-aliases

## Why

gateway 目前是「配置只读」的服务：providers 只能在 config.toml 手改，改完必须重启进程；webui 的 gateway 页是启动时注入的静态快照（`GatewayInfo`），无法反映运行态；没有 admin API，没有 metrics。更糟的是存在一条断链——飞书 `/provider` 卡片的编辑写入 `~/.sebas/state.json` v2（spec 2026-08-17 §2.6 迁移后 `providers.json` 已被删除），而 gateway 仍读 `~/.sebas/providers.json`——卡片编辑的 provider 根本到不了 gateway。本 change 建立可管理、可观测的 gateway 管理面，并顺手修复这条断链。

## What Changes

- gateway 新增同端口 admin HTTP 面（`/admin/*`）：watchdog 场景用 `SEBAS_CONTROL_SECRET` 做 Bearer 鉴权；standalone（无 secret）回退 loopback 免鉴权 + 启动 warn。`/healthz` 与 LLM 透传流量鉴权语义不变。
- Admin API 提供 providers CRUD（key 恒脱敏、空提交保留旧值）、模型别名 CRUD、provider model 列表探测、`/admin/stats` JSON 摘要、手动 `POST /admin/reload`。
- 新增 Prometheus 格式 `/metrics`：requests 计数（provider/model/protocol/status 标签）、latency 直方图、tokens 计数、429/上游错误计数。
- 配置热生效：admin 写入走「校验 → 原子写 `~/.sebas/providers.json` → 内存重建 RouteTable/api_keys」，写前校验失败即拒绝；`notify` 监控 providers.json，飞书卡片/手改等外部变更实时热加载（debounce）。在途请求不受热重建影响。
- 模型别名成为第一类实体：`{别名, provider, upstream_model?}` 持久化在 providers.json 新段 `model_aliases`，gateway 内部编译为「精确路由 + model_map 改名」，路由引擎零改动。
- webui gateway 页重构：webui 后端作为 BFF 代理 gateway admin API（control secret 不出后端），provider/别名 CRUD 表单 + stats 数字卡片（沿用 htmx），mutation 沿用现有 admin 鉴权姿态。

- provider 数据回归单一真源 `~/.sebas/providers.json`（含 providers/deleted/model_aliases 段）：router `state_store` 把 providers/deleted 段从 state.json 拆回 providers.json（mode/default_selection 留 state.json），已迁移机器自动搬出——修复「卡片写 state.json 而 gateway 读 providers.json」的断链。

## Capabilities

### New Capabilities

- `gateway-admin-api`: gateway 管理 HTTP 面——鉴权、providers/别名 CRUD、探测、热重载写路径与外部变更加载、stats 摘要。
- `gateway-model-aliases`: 模型别名实体语义——wire 格式、解析优先级、上游改名、校验与自愈。
- `gateway-metrics`: Prometheus `/metrics` 指标族与 `/admin/stats` JSON 内容契约。

### Modified Capabilities

- `gateway-core`: Endpoint surface 增补 admin/metrics 路由共存与鉴权分层；Routing resolution order 增补别名优先级链。
- `webui`: HTTP route surface 增补 gateway 编辑路由（BFF 代理）；Mutation posture 覆盖 gateway 变更类操作。

## Impact

- `gateway` crate：新增 `admin.rs`/`metrics.rs`；`config.rs` 支持 `model_aliases` + 热重载解析；`AppState` 从启动期只读改为 `RwLock` 共享可换内核；新依赖 `notify`。
- `webui` crate：gateway 页重构 + gateway admin HTTP 客户端。
- `router` crate：`state_store` providers/deleted 段拆回 providers.json + 一次性反向迁移；卡片（`crud::FileStore`）写路径随之切回 providers.json。
- 跨进程共享文件：core（卡片）与 gateway（admin API）两进程均写 providers.json，采用 entry 级 read-modify-write + 原子 rename，最后写者胜。

## Non-goals

- 不做路由故障转移（routes provider 数组仍只取第一个）。
- webui 不做时序图表，图表留给 Prometheus/Grafana。
- 模型别名不进 Direct/Off spawn 路径（别名只影响 gateway 路由）。
- 不做 per-token 差异化限流配置。
- 飞书 `/provider` 卡片不迁移到 admin API（保持文件写入路径，与 admin API 双写者共用 providers.json）。
