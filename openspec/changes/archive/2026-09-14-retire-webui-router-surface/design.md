## Context

`make-core-own-provider-data`（2026-09-11 归档）之后的事实：provider/别名/defaults
数据权威在 core 状态库；webui 的 provider BFF 读经 `SessionBackend.state_snapshot`、
写经 `state_mutate`（detached 走 core channel IPC，内嵌直调同一 seam）；router 经
core channel 订阅（`sebas-router/src/core_channel.rs` NDJSON 长连接）自动热重载，
文件监听只是降级路径。残留在 `sebas-webui`：

- `src/router_client.rs`（149 行）：唯一 webui→router 网络边，只剩
  `POST /admin/reload` 代理一个方法；前端零调用方，全仓引用仅 webui 自身实现与
  `tests/gateway_bff_test.rs`、`src/server.rs` 两处测试。
- `/router/api/*` BFF 前缀：路径名撒谎（是 webui 自有端点、core 承载数据），
  `server.rs` 的鉴权路径匹配（`path.starts_with("/router/api/")` 两处）为其特判。
- `GET /api/router`（静态快照）+ 前端 `client.api.router()`：设置页 3.4 重设计后
  无调用方。
- `webui/spec.md` line 257 仍写「forwards to the router admin API with the control
  secret」——与同 spec line 14 矛盾，属陈旧表述。

## Goals / Non-Goals

Goals：webui 进程部署图收敛为单一出边（core channel IPC）；API 面不再出现 router
概念命名；specs 与实现一致且承载拓扑原则。

Non-goals：见 proposal（router 侧一切不动、probe 仍在 core 执行、不为 provider 面
新增角色执法、`router_listen` 等静态展示字段留待 UI 重设计）。

## Decisions

- **D1 硬切，不留旧路径别名**：前后端同二进制发布（`cargo build` 烘焙
  `frontend/dist`），webui BFF 是 SPA 私有面、无外部消费者，改名即原子生效。
  备选「308 重定向过渡」被否：私有线协议不需要迁移窗口，别名只会延长
  `/router/api` 的尸体腐烂期。
- **D2 新命名空间**：`/api/providers`、`/api/providers/{name}`、
  `/api/providers/{name}/probe`、`/api/provider-presets`、
  `/api/provider-defaults`、`/api/model-aliases`（+`/{alias}`）。presets/defaults
  加 `provider-` 前缀避免与未来其它 preset/defaults 语义撞车；JSON 线形状一律
  不变，前端只改 URL 与方法名。
- **D3 只删 RouterClient 的 secret 读取，不动控制面同名密钥**：
  `SEBAS_CONTROL_SECRET` 在 webui 里有两个互不相干的消费者——RouterClient
  （删）与 watchdog 控制 RPC 的 admin actions（`Admin actions via control plane`
  条款，留）。实现时严禁顺手「清理」后者。
- **D4 reload 触发权交还 router 自有面**：`/admin/reload` 保留在 router 侧作
  ops 后门（router-admin-api spec 的 `External change hot reload` 条款不动）；
  webui 不再代理它。日常生效路径 = core channel 订阅推送，spec 已有 burst 合并
  语义，无功能损失。
- **D5 `GET /api/router` 与 `client.api.router()` 直接删除**；启动快照
  `RouterInfo`（TOML 解析的静态事实，不拨号）保留——`/api/settings` 的
  `router` 段与 `/api/about` 的 `router_listen` 继续作为展示元数据，归属
  Non-goals 留待 UI 重设计。
- **D6 原则落点的双层结构**：行为条款进 `webui` capability（ADDED
  「Router-free API surface and IPC-only server edge」）；术语与拓扑一句话进
  `openspec/glossary.md`（名词表是术语单一事实来源，语义变化先改它）；
  `router-admin-api` 的 Purpose 文案修正（webui 移出消费者名单）是非
  requirement 级修订，随任务直接改主 spec。

## Risks / Trade-offs

- [测试面广：gateway_bff_test、server.rs 内嵌测试、前端 client/settings 测试都钉在旧路径] → 改名与删边分两个任务批次落地，每批次跑 `cargo test -p sebas-webui` 与前端 vitest 收敛后再进下一批。
- [漏改路径守卫导致鉴权口径漂移] → `server.rs` 中 `/router/api/` 前缀特判删除后，新增回归断言：`/router/api/providers` 必须 404 且不因前缀匹配误入鉴权豁免分支。
- [外部脚本/文档若有旧路径引用] → 实现时全仓 grep `/router/api` 兜底（当前仅 spec、测试、AGENTS 沙箱菜谱外无业务引用；AGENTS 菜谱不涉及该路径）。

## Migration Plan

单二进制原子发布，无数据迁移：provider 数据本就在 core 状态库，路径改名不触碰
存储。回滚 = revert 提交重建二进制。

## Open Questions

无——A4（`router_listen` 展示去留）已按 D5 划出本 change 范围。
