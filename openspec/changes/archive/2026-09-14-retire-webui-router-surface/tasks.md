## 1. 退役 webui→router 网络边

- [x] 1.1 删除 `sebas-webui/src/router_client.rs` 与 `/router/api/reload` BFF 路由（`routes.rs::router_api_reload`、`server.rs` 路由注册及 1339/1352 两处 reload 测试），删除 `router_mutation_guard` 中对已退役路径的引用；验证 `cargo check -p sebas-webui` 通过且全仓 `grep -rn RouterClient` 仅剩历史归档
- [x] 1.2 清理 `RouterClient` 独占的 `SEBAS_CONTROL_SECRET` 读取（保留 watchdog 控制 RPC 与 `/api/env` 遮蔽对同名变量的既有用途，见 design D3）；验证 `grep -rn "SEBAS_CONTROL_SECRET" sebas-webui/src` 仅剩 admin/env 两处合法消费者
- [x] 1.3 `gateway_bff_test.rs` 中 reload 代理用例删除、其余用例迁移；验证 `cargo test -p sebas-webui --test gateway_bff_test` 全绿

## 2. provider 集群改名 `/router/api/*` → `/api/*`

- [x] 2.1 `server.rs` 路由注册改名（providers、providers/{name}、providers/{name}/probe、provider-presets、provider-defaults、model-aliases、model-aliases/{alias}），删除 `/router/api/` 前缀在鉴权匹配（约 324、352 两行）的特判，确认新路径全部落在 `/api/*` 既有鉴权门内；验证 `cargo test -p sebas-webui` 全绿
- [x] 2.2 `routes.rs` / `api.rs` 内 handler 文档注释与函数名（`router_api_*` → 语义名）同步；验证无 `/router/api` 字符串残留于 `sebas-webui/src`
- [x] 2.3 前端 `client.ts` 8 处调用改名（`routerProviders→providers` 等方法名顺带去 router 前缀）、`settings-modal.ts` 相关注释与错误文案、`model-catalog.ts` 线协议注释；验证 `pnpm -C sebas-webui/frontend test`（vitest）全绿
- [x] 2.4 新增回归断言：`/router/api/providers` 与 `GET /api/router` 返回 404 且不落入鉴权豁免分支；验证测试在 `cargo test -p sebas-webui` 中通过

## 3. 死面清理

- [x] 3.1 删除 `GET /api/router` 路由与 `api.rs::router` handler、前端 `client.api.router()` 方法及其测试引用；验证 `grep -rn "api/router\|api\.router()" sebas-webui` 零命中（`/api/about` 的 `router_listen` 字段按 design D5 保留）

## 4. spec 主文档非 delta 修订

- [x] 4.1 `openspec/specs/router-admin-api/spec.md` Purpose 改写：消费者收敛为 ops/CLI 与 router 自身，webui 移出名单，`/admin/reload` 标注为 ops 后门；验证 `openspec validate --specs` 通过
- [x] 4.2 `openspec/glossary.md` 进程角色段补拓扑原则一句（router 仅服务执行体数据面；webui 唯一出边 = core channel IPC；API 面无 router 概念）；验证 glossary 渲染无术语冲突

## 5. 端到端验证

- [x] 5.1 沙箱联调：`invoke testsuite-e2e` 全绿；另起 `invoke testsuite-webui-sandbox` 手工走 provider 新建/改名/删除/probe 旅程，确认 router 侧经订阅自动生效（`/admin/stats` 路由数变化）、`/router/api/*` 404
- [x] 5.2 `invoke testsuite-acceptance` 全绿；`cargo clippy -p sebas-webui` 无新告警后按 conventional commits 提交于 `feat/*` 分支
