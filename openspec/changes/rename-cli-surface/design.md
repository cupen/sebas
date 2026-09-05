## Context

三层命名互相纠缠：CLI `run`（核心服务）/`watchdog`（监督守护）/`gateway`（模型路由）；crate `sebas-router`（会话分发）与 `sebas-gateway`（模型路由）；配置节 `[router]`（会话映射持久化）与 `[gateway]`（模型路由）。用户定名目标态：`core`/`run`/`router` 三命令 + `sebas-dispatch`/`sebas-router` 两 crate。改名是纯机械动作，但**双向互换**（gateway→router 且 router→dispatch）带来了撞名风险与顺序约束。

## Decisions

### D1. 双向改名的执行顺序（关键）

必须**先让位、后入位**：

1. `sebas-router` → `sebas-dispatch`（crate 目录、Cargo name、`sebas_router::`→`sebas_dispatch::`、`RouterHandle`→`DispatchHandle`、`[router]`→`[dispatch]`、`native_router_bridge.rs`→`native_dispatch_bridge.rs`）
2. `sebas-gateway` → `sebas-router`（crate 目录、`sebas_gateway::`→`sebas_router::`、`GatewayConfig`→`RouterConfig`、`GatewayAction`→`RouterAction`、gateway 子命令→`router`、`[gateway]`→`[router]`）
3. run/watchdog CLI 面（与 1/2 无耦合，可并行）

倒序执行会让步骤 2 的新 `sebas_router` 名字撞上尚未改走的旧 crate。

### D2. 兼容别名（零成本保旧）

- `Cmd::Run` 加 `#[command(alias = "watchdog")]`：**硬兼容**——已装 systemd unit 的 `ExecStart` 烘的是 `watchdog --config`，unit 只在重装时重渲染，无此别名升级后旧 unit 重启即失败。隐藏别名（非 visible），help 不展示。
- `Cmd::Router` 加 `#[command(alias = "gateway")]`：软兼容——旧脚本/肌肉记忆。
- 旧 `run`（core 语义）**不设别名**：与新 `run` 冲突，且所有调用方在仓库内可控（Dockerfile/tests/scripts/docs 同步改）。

### D3. 配置与数据的旧值兼容（一个发布窗口，告警不阻断）

- `[gateway]`/`[router]` 旧节：解析时识别 → 应用 + `warn!` 告警提示新名（config.rs 已有 `deprecated_watchdog_upgrade_hits` 先例）。
- env：新名优先，回退读旧名并告警（`SEBAS_GATEWAY_PROVIDER_OVERLAY`、`SEBAS_AGENT_GATEWAY_URL`、`SEBAS_AGENT_GATEWAY_AUTH`）。
- 持久化 `ProviderMode`：serde `#[serde(rename = "router", alias = "gateway")]`——写新值、双读，state.json 无需迁移。
- **不兼容**：控制面服务名 `"gateway"`→`"router"` 是 wire 变更，不做双读——两端同二进制发布，风险窗口仅自升级重启瞬间，不值得为它加映射层。

### D4. spawn/log 锚点全部收敛到常量

`lib.rs` 三个子命令常量：`CORE_SUBCOMMAND="core"`、`RUN_SUBCOMMAND="run"`、`ROUTER_SUBCOMMAND="router"`。watchdog 拉生 core 从硬编码 `.arg("run")` 改为引用 `CORE_SUBCOMMAND`（现状与常量靠人肉同步，注释明说）；service.rs ExecStart、spawn_aux argv 同样引用常量。日志行 `"router started (core --router)"` 是 AGENTS.md 菜谱的 grep 锚点，与文档同改。

### D5. spec 能力目录改名的 delta 建模

openspec 无「能力目录 RENAMED」操作：新路径 delta 写 `## ADDED Requirements` 全量（措辞已换新名），旧路径 delta 写 `## REMOVED Requirements` 清单；归档后旧 spec 目录清空，手工 `git rm` 收尾（tasks 列明）。需求级改名用 `## RENAMED Requirements`（`- FROM:`/`- TO:`，如 "Env-only bootstrap for run"→"...for core"）。

**工具链约束**：MODIFIED 块内的 scenario 标签被 validator 钉死（archive 拒绝丢失既有 scenario，同块 REMOVED+ADDED 同名又会在应用顺序 ADDED→REMOVED 下自删）。因此只改 scenario 的**内容**措辞，**标签**保留旧名（如 "gateway mode write" 描述的是点击 `Router`）——标签是稳定锚点，非规范内容。

## Risks

- 漏改字符串面：以清零扫描兜底（`sebas_gateway|GatewayConfig|GatewayAction|SEBAS_GATEWAY|sebas_router\b|RouterHandle`，排除别名定义/archive/历史 log）。
- `pgrep -f 'run --config'` 类脚本 pattern 改名后语义漂移（会错匹配新 `run`=watchdog）——逐个人工核过。
- 前端 fixtures（app-shell.test.ts 等）与 API 字段名强耦合——前后端同仓同提交，编译+测试兜底。
