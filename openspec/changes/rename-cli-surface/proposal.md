## Why

CLI 与 crate 命名已与语义脱节：`sebas run` 实际是核心服务（systemd 真正运行的是 `watchdog`），`gateway` 将按产品语义定名为模型路由 `router`，而 `router` 一词又被会话路由 crate（`sebas-router`/`RouterHandle`/`[router]` 配置节）占用。一次把三层名字理顺，消除后续所有文档与代码的歧义成本。

## What Changes

- **BREAKING** `sebas run` → `sebas core`（核心长驻服务；旧 `run` 语义废止）
- **BREAKING** `sebas watchdog` → `sebas run`（监督守护；保留隐藏别名 `watchdog`，已装 systemd unit 的 `ExecStart` 不重装也能继续启动）
- **BREAKING** `sebas gateway` → `sebas router`（模型路由；保留隐藏别名 `gateway`）；内嵌 flag `--gateway` → `--router`
- **BREAKING** crate 改名：`sebas-gateway` → `sebas-router`；`sebas-router` → `sebas-dispatch`（`RouterHandle`→`DispatchHandle`，`GatewayAction`→`RouterAction`）
- **BREAKING** 配置节：`[gateway]` → `[router]`；`[router]` → `[dispatch]`；env `SEBAS_GATEWAY_PROVIDER_OVERLAY` → `SEBAS_ROUTER_PROVIDER_OVERLAY`、`SEBAS_AGENT_GATEWAY_URL/AUTH` → `SEBAS_AGENT_ROUTER_URL/AUTH`——旧键/旧 env 一个发布窗口内仍生效并告警；持久化 `ProviderMode::{"kind":"gateway"}` 改写为 `"router"`（serde alias 兼容读旧值）
- 控制面服务名 `"gateway"` → `"router"`（services RPC/展示）；webui API `/api/gateway`→`/api/router`、代理挂载 `/gateway/api/*`→`/router/api/*`、`GatewayInfo`→`RouterInfo`
- 5 个 `gateway-*` spec 能力目录改名 `router-*`；`router-commands` 改名 `dispatch-commands`

## Capabilities

### New Capabilities

（无——本变更只改名既有能力，不引入新行为面）

### Modified Capabilities

- `cli-service`: 子命令树改为 core/run/router/webui 系（含两隐藏别名）；`ExecStart` 烘 `run --config`；env-bootstrap/配置节的命令提法随迁
- `watchdog`: 核心子进程拉生 argv 改为 `core --config`；辅助服务表 gateway→router 提法
- `webui`: `/api/gateway`→`/api/router`、`/gateway/api/*`→`/router/api/*`、`run --webui` 提法→`core --webui`
- `feishu-option`: 默认部署场景的命令提法
- `replay-debug`: `sebas run --dump-inbound` → `sebas core --dump-inbound`
- `process-e2e-suite`: detached 拓扑与 debug provider 的命令/组件提法
- `provider-management`: provider mode `gateway` 枚举值改 `router`（兼容读旧值）、spawn env 提法
- `acceptance-suite`: `SEBAS_AGENT_GATEWAY_URL`→`SEBAS_AGENT_ROUTER_URL`、`gateway-model-aliases` 能力引用随迁
- `gateway-core` → `router-core`（能力目录改名，内容措辞 gateway→router）
- `gateway-admin-api` → `router-admin-api`（同上）
- `gateway-auth-rate-limit` → `router-auth-rate-limit`（同上）
- `gateway-metrics` → `router-metrics`（同上）
- `gateway-model-aliases` → `router-model-aliases`（同上）
- `router-commands` → `dispatch-commands`（能力目录改名；内容为会话命令语法，仅目录与自引用改名）

## Impact

- 代码：workspace 两个 crate 目录与全部 `sebas_router::`/`sebas_gateway::` use 路径（src/ 18+7 个文件）；cli.rs/main.rs/lib.rs/watchdog.rs/service.rs/config.rs；webui 后端 API + 前端 client.ts/fixtures；watchdog spawn argv 与控制面服务名
- 数据兼容：state.json 的 provider mode 旧值 `"gateway"` 仍可读；已装 systemd unit 经 `watchdog` 别名兼容，重装后 ExecStart 更新为 `run`
- 文档：README、docs/architecture.md、docs/frontend-dev.md、AGENTS.md（沙箱菜谱与 env 三件套）、glossary.md（新增三义消解：CLI `router`=模型路由 / `sebas-dispatch`=会话分发 / 前端 `router.ts`=URL 路由）
- 脚本：test_watchdog_debug_upgrade.sh（spawn 与 pgrep pattern）、test_webui_sandbox.sh

## Non-goals

- 不拆 `sebas-dispatch` 内部逻辑（卡片层下沉 sebas-feishu、provider 持久化独立——留待后续 change）
- 不动 `[watchdog.*]` 配置命名空间与 watchdog 概念命名（模块名 `src/watchdog.rs` 等保留）
- 不动控制面 `core`/`webui` 服务名与 `/services` 行名 `watchdog`
- 不做 `ctl`/`status`/`services` 简写与 `service`/`update` 命令的整理（另行 change）
- 不改 `docs/acp-opencode-accept-*.md` 历史 log 与 `openspec/changes/archive/**`
