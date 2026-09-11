## Why

规格声称「core 状态库是 provider 数据的唯一事实来源，router 只读」，但实现不是：
router 进程自己拥有一整套写路径（`POST/PUT/DELETE /admin/providers*`、
`/admin/model-aliases*`、probe 的 `apply`），core 通道不可用时**直接回退写
`providers.json`**；`defaults.json` 更是只由 router 写、core 完全不消费；而
`sebas-webui` 没有 core 通道客户端，WebUI 只能经 HTTP 打到 router 进程做 provider
管理。结果是「谁拥有 provider 数据」在实现层面含糊，router 事实上能改数据，与
「models 数据由 core 管理、router 只是使用者」的定位相悖。

## What Changes

- **BREAKING**：router 失去全部 provider/model 写路径——`/admin/providers*` 与
  `/admin/model-aliases*` 的变更接口、probe 的 `apply`、`defaults` 写入，以及
  `providers.json` 回退写。router 只保留只读消费（core 快照 + 变更通知 + 只读视图）。
- core 成为 provider / model / alias / defaults 的唯一管理者：管理经既有 core 状态库
  通道（`providers` / `aliases` / `settings` 域，defaults 并入 `settings` 域），飞书
  与 WebUI 都走它。
- WebUI 后端的 provider 管理不再代理 router 进程，改由 core 通道承载；`/router/api/*`
  下的 provider 变更簇从「代理 router」改为「读写 core 状态库」，对外路径名与鉴权
  姿态不变。
- `defaults` 从 router 独占的 `defaults.json` 迁入 core 状态库，与 provider 数据同源、
  同事务。
- 修掉 spawn 期 `read_overlay_item` 优先读 legacy `providers.json` 的行为：旧文件会
  盖住库里的 `default_model`，必须让库成为权威。
- legacy `providers.json` 不再被写、不再作为权威；仅在无 core 状态库时按现有降级路径
  读取并如实报「数据源不可用」。

## Capabilities

### New Capabilities

- 无。

### Modified Capabilities

- `provider-management`：新增「core 拥有 provider 与 model 数据」——core 是唯一写者，
  router 只读消费。
- `core-session-channel`：状态库通道面扩展为 provider / model / alias / defaults 的
  管理入口（defaults 并入 `settings` 域）。
- `router-admin-api`：provider CRUD、model alias CRUD、model probe 三个变更面从 router
  移除；`Configuration source` 与 `Write-then-apply semantics` 改为 core 是写者。
- `webui`：HTTP 路由面中 provider 管理的承载从 router 代理改为 core 状态库。

## Impact

Rust：`sebas-router/src/admin.rs`（删写路径与 `channel_write` 文件回退）、
`config.rs`（不再接受文件写 / 不再以文件为权威）、`hot_reload.rs`（仅外部只读）、
`src/core_channel/server.rs`（defaults 并入 settings 域）、`src/sebas_state/repo.rs`
（defaults 持久化）、`src/spawn_env.rs`（去掉 legacy 优先）、
`sebas-webui/src/routes.rs` + `router_client.rs`（改 core 通道承载）、
`sebas-webui/Cargo.toml`（core 通道依赖）。前端：`api/client.ts` 的管理端点指向不变，
但错误语义随 core 承载调整。测试：`sebas-router/tests/admin_test.rs`（写路径用例删除
或改为只读）、`tests/state_*`、`sebas-webui/tests/gateway_bff_test.rs`、
Playwright `settings.spec.ts` / `models.spec.ts`。

**BREAKING**：`/admin/providers*`、`/admin/model-aliases*`、`/admin/defaults` 的变更
接口下线；`/admin/providers/{name}/probe` 的 `apply` 下线（抓取能力在 `add-fetch-models`
中于 core 侧重做）。

## Non-goals

- 不改 provider 的字段形状，也不动模型条目的数据结构（结构化 model 条目留给
  `redesign-provider-models-settings`）。
- 不删除 legacy `providers.json` / `state.json` 的读取兼容，只是不再写、不再当权威。
- 不为 core 新开独立 HTTP 管理端口：管理面走既有 core 通道。
- 不重命名对外路径（`/router/api/*` 的名字会名不副实，重命名另开 change）。
- 不迁移已存在于 `defaults.json` 的历史值以外的数据；provider 数据本就在库里。
