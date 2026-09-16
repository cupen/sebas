# Design: simplify-service-config

## Context

`router.routes` 是活代码（`RouteTable::match_route` 每请求消费），但其三层兜底（namespace 直连、`default_provider`、单 provider 隐式默认）使显式路由表在真实部署里冗余——操作员拍板作废。`watchdog.*` 前缀六个子表中，操作员点名 core/webui/router 三节更名 `service.*`，im/upgrade/storage 留守（用户决定，混合前缀是接受形态）。旧键姿势：警告忽略（用户拍板）。另见 proposal.md Why。

关键事实：
- 根配置 `[watchdog]` 树**无** `deny_unknown_fields`——直接删字段会让旧键静默失效，必须配 raw-TOML 扫描告警；`warn_deprecated_watchdog_keys`（src/config.rs:677-692）是现成模板（eprintln + tracing 双通道，因 parse 先于 tracing init）
- router 侧 `RouterFile` 只提取 `[router]` + `[provider.*]`，其余节容忍；`[router.routes]` 的废弃扫描挂在 router 解析入口
- 受管服务名（control RPC / services.json 的 `core`/`webui`/`router`/`im`）是独立命名空间，spec watchdog:343 钉死，不随配置前缀改名

## Goals / Non-Goals

- Goals：`[router.routes]` 键面消失（警告忽略）；三节配置键更名 + Rust 结构体对齐；旧配置可启动但告警指路
- Non-Goals：见 proposal Non-goals

## Decisions

- **D1 警告忽略而非硬拒绝**（用户拍板）：旧配置照常启动，偏差只有一行日志。实现上根配置扩展现有 `warn_deprecated_watchdog_keys`：raw TOML 命中 `[watchdog.core|webui|router]` 任一表 → warn；router 侧新增同类扫描（`RouterFile` 解析入口）。Alternative：硬拒绝（acp.claude 先例）——被用户否决。
- **D2 `Config` 拆分**：`service: ServiceConfig { core, webui, router }` + `watchdog: WatchdogConfig { im, upgrade, storage, max_spawn_failures }` 两个节共存；`WatchdogCoreConfig`/`WatchdogWebUiConfig`/`WatchdogRouterConfig` 更名 `Service*`（im/upgrade/storage 结构体保留 Watchdog 名）。Alternative：serde rename 保留 Rust 名——被否，代码读起来会和配置键脱节。
- **D3 routes 作废语义**：`RouteTable` 不再装载 config routes，exact/glob 匹配层删除，解析链收敛 namespace → alias → default（隐式单 provider 默认不变）；`/admin/stats` 的 `routes` 计数字段删除（恒 0 无意义，router-admin-api spec 未钉该字段）。校验报错（`router.routes.* provider 列表不能为空` 等）随之删除。
- **D4 文案同步**：报错/帮助字符串里引用 `[watchdog.webui]` 等字样的（webui_cmd、run、cli）随键名一起改，避免指路指到不存在的键。

## Risks / Trade-offs

- [旧配置 `auth = false` 等键静默回落默认] → warn 双通道可见（stderr 不依赖 tracing init）+ 提案 Migration 指明迁移键名；未正式发布产品，部署面即操作员本人
- [routes 依赖者路由行为变化] → 兜底链承接：namespace `provider/model` 直连、`default_provider`、单 provider 隐式默认；操作员 config 里 deepseek-*/kimi-* 两条与 namespace 形式重合，实际无损
- [混合前缀过渡形态] → im/upgrade/storage 留守是用户明确选择，将来若统一可另立 change

## Migration Plan

警告忽略 = 无需迁移即可启动；仓库内 config/config.toml、example、README 等随本变更同步到新键。回滚 = revert 提交。

## Open Questions

无。
