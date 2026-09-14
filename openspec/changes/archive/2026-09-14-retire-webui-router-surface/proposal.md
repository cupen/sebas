## Why

设计原则是：**WebUI 的 HTTP API 面不出现「router」概念；服务端进程间通信一律走 IPC（core channel）**。`make-core-own-provider-data` 之后 provider 数据已归 core 状态库、router 经 core channel 订阅自动热重载，但 WebUI 仍残留一条 webui→router 的 HTTP 直连边（reload 代理，前端零调用方），且整组 provider BFF 路径仍挂在 `/router/api/*` 这个名不副实的前缀下；`webui/spec.md` 内部还存在自相矛盾的两处表述（line 14「绝不代理 router 进程」 vs line 257「forwards to the router admin API with the control secret」）。原则必须落进 spec，否则会按代码现状反复长回枝节。

## What Changes

- **BREAKING** 退役 webui→router 唯一网络边：删除 `RouterClient`、`POST /router/api/reload` BFF 路由及 webui 侧 `SEBAS_CONTROL_SECRET` 读取。功能无损失：router 热重载由 core channel 订阅自动覆盖。
- **BREAKING** provider 管理集群改名 `/router/api/*` → `/api/*` 命名空间（`/api/providers`、`/api/providers/{name}`、`/api/providers/{name}/probe`、`/api/provider-presets`、`/api/provider-defaults`、`/api/model-aliases`）。线协议 JSON 形状不变；前后端同二进制发布，无外部消费者，不做旧路径兼容。
- 退役死面：`GET /api/router` 端点与前端 `api.router()` 方法（3.4 重设计后无调用方）。
- spec 修正：`webui` 重写 provider 面条款并新增拓扑原则条款、删除陈旧的「转发 router admin API」表述；`webui-user-management` 角色执法豁免路径跟随改名；`testsuite-webui-browser` 真值来源随动（Router 卡片归 `/api/admin/services`，退役 `/api/router` 真值）；`router-admin-api` 的 Purpose 文案（webui 移出 admin 消费者名单）与 `openspec/glossary.md` 拓扑原则一句为非 requirement 级修订，随任务直接改主 spec。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`: provider 集群路径改名为 `/api/*`；新增「API 面无 router 概念、唯一出边 core channel IPC」规范条款；删除 reload 代理与 `GET /api/router`；修正变异守卫条款中陈旧的转发表述。
- `webui-user-management`: provider 管理面角色执法豁免条款的路径跟随改名。
- `testsuite-webui-browser`: 设置面只读/写覆盖两条款的真值来源与降级成因随本 change 修正。

## Impact

- 代码：`sebas-webui`（router_client.rs 删除、routes.rs / server.rs 路由与守卫、api.rs 死端点）、前端 `client.ts` / `settings-modal.ts` 及对应测试、`gateway_bff_test`。
- 行为：webui 进程部署图收敛为单一出边（webui→core channel IPC）；router 侧一切不动（数据面、`/admin/*`、热重载订阅）。

## Non-goals

- 不动 router 侧任何端点：`/admin/reload` 保留为 ops/CLI 后门。
- 不动 provider 数据权威归属（core 状态库）与 router 的 core channel 订阅热重载机制。
- 不动 probe 执行位置（仍由 core 出网抓取，webui 只转发）。
- 不动 agent 执行体 → router 的数据面推理流量。
- 不为 provider 面新增角色执法（维持仅登录门 + 自身守卫的现状，仅路径改名）。
- 不在本 change 处理 `/api/settings` / About 页中 router 静态事实字段（`router_listen` 等）的展示去留——它们来自启动快照（TOML 解析，不拨号），留待 UI 重设计另议。
