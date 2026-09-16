## Why

配置面有两处历史包袱：`[router.routes]` 与既有三层兜底路由（provider namespace 直连、`default_provider`、单 provider 隐式默认）高度重合，实际部署从未真正依赖显式路由表；受管服务的配置前缀 `watchdog.*` 与产品语义脱节——Services 页面对外呈现的就是 core / webui / router / im 四个「服务」，配置节却叫 watchdog。简化配置：作废 `[router.routes]`，`[watchdog.{core,webui,router}]` 更名 `[service.*]`。

## What Changes

- **BREAKING** `[router.routes]` 作废：解析时警告忽略（不报错），模型解析完全走 namespace / model alias / `default_provider`（含单 provider 隐式默认）；`RouteGroup`、config-route 匹配、对应校验与管理端点 `routes` 计数随之删除
- **BREAKING** `[watchdog.core]`、`[watchdog.webui]`、`[watchdog.router]` 更名 `[service.core]`、`[service.webui]`、`[service.router]`；解析时旧键警告忽略（stderr + tracing 双通道），行为回落默认值
- `[watchdog.im]`、`[watchdog.upgrade]`、`[watchdog.storage]` 与裸 `max_spawn_failures` 保留 watchdog 前缀不动
- Rust 侧 `Config` 拆出 `service: ServiceConfig { core, webui, router }`（结构体同步更名 `ServiceCoreConfig` / `ServiceWebUiConfig` / `ServiceRouterConfig`），`WatchdogConfig` 瘦身为 `{ im, upgrade, storage, max_spawn_failures }`
- 配套：`config/config.toml` 与 example、README、AGENTS.md、tasks.py、docs/architecture、测试 fixtures 同步新键

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `router-core`: 「Routing resolution order」移除 config-route 匹配层（exact/glob over routes），解析顺序收敛为 namespace → alias → default provider；旧 `[router.routes]` 键警告忽略语义
- `watchdog`: 「Service lifecycle」拉起开关键名 `[watchdog.webui]` → `[service.webui]`；「Managed service table」恒启 core 措辞脱离旧键名；旧键警告忽略
- `webui`: allowed_roots 场景、Local-only binding、非 loopback bind 联动、Watchdog lifecycle ownership、鉴权开关五处键名 `[watchdog.webui]` → `[service.webui]`
- `feishu-option`: 「webui 主控部署形态」键名
- `testsuite-process-e2e`: 「沙箱全隔离」配置清单键名

## Impact

`sebas-router`（config/routing/admin + 4 个测试文件）、`src/config.rs` 及访问链约 40 处（webui_cmd/run/watchdog/services/executor/core_channel/im_cmd/node_link_cmd）、config 两份、README / AGENTS.md / docs/architecture / tasks.py / tests fixtures。前端零涉及（无 config 键引用）。旧配置在警告下继续启动，但 `auth = false` 等旧键被忽略回默认值——部署需迁移到新键名。

## Non-goals

- 不做配置文件自动迁移/改写工具（警告指路即可）
- 不动 `[watchdog.im]`、`[watchdog.upgrade]`、`[watchdog.storage]`、`max_spawn_failures`（保留 watchdog 前缀）
- 不改受管服务名 `core` / `webui` / `router` / `im`（control RPC / REST 层字符串）与 watchdog 进程概念本身
- 不引入任何新路由特性；routes 作废后的解析行为由既有兜底链承接
