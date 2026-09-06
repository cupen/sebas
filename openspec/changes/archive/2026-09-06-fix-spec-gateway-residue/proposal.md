## Why

rename-cli-surface 已完成 gateway→router、watchdog→run 的 CLI/配置/控制面改名,但:(1) 主 specs 里有 4 个能力的个别段落仍以 "gateway" 作为**现行术语**出现(按钮文案、场景标题、服务列表、LLM channel 措辞),与 glossary 及实际代码不一致;(2) 改名留下的一整层兼容(旧 env 名回退、配置节迁移 shim、状态值 alias、隐藏 CLI 别名)针对的是从未发布的旧设计——本项目没有正式发布,没有存量用户、配置或 systemd unit 需要兼容,这层兼容是纯负担。

## What Changes

- 主 specs 措辞纠正:`agent-core`、`provider-management`、`webui`、`watchdog` 四个 spec 中作为现行术语的 "gateway" 改为 "router"(与代码 provider_card.rs 按钮、webui 路由、`sebas router` 子进程一致)。
- **BREAKING** 移除旧 env 名支持:`SEBAS_GATEWAY_PROVIDER_OVERLAY`、`SEBAS_AGENT_GATEWAY_URL`、`SEBAS_AGENT_GATEWAY_AUTH` 不再被读取(代码中连回退+告警一起删除),只认 `SEBAS_ROUTER_PROVIDER_OVERLAY` / `SEBAS_AGENT_ROUTER_URL` / `SEBAS_AGENT_ROUTER_AUTH`。
- **BREAKING** 移除配置节迁移 shim:`[gateway]`→`[router]`、`[watchdog.gateway]`→`[watchdog.router]`、`[router]`→`[dispatch]` 的解析前重写与告警删除,未知配置节按既有错误路径处理。
- **BREAKING** 移除 provider 状态文件 `"kind": "gateway"` 的 serde alias,旧值直接解析失败。
- **BREAKING** 移除隐藏 CLI 别名 `sebas gateway` / `sebas watchdog`,调用报未知子命令错误。
- `SEBAS_AGENT_GATEWAY_URL` 在 native 后端错误提示文案中的引用随改。

## Non-goals

- 不改现行命名:`[watchdog.*]` 配置节、watchdog 守护进程概念名、`sebas run`/`sebas router` 子命令名、`status`/`services`/`ctl` 这类**现行**别名(非改名兼容,保留)。
- 不动 webui 退役路径 `/gateway` 的重定向行为(IA-v1 路径退役是现行设计,不是改名兼容)。
- 不改 router-admin-api 等其余能力的 spec。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

- `agent-core`:LLM channel 需求措辞——可选模型路由层的称呼 gateway→router。
- `cli-service`:子命令树移除隐藏兼容别名 `watchdog`/`gateway` 及其理由句;env 变量需求移除 pre-rename 旧名 honored 条款与对应场景,改为旧名不生效。
- `provider-management`:卡片布局按钮文案 Gateway→Router;Mode switching 移除 pre-rename 状态值兼容条款与场景;3 个场景标题 gateway→router。
- `webui`:HTTP route surface 与 Mutation posture 下 3 个场景标题 gateway→router。
- `watchdog`:Control request surface 受管服务列表 gateway→router;Managed service table 一个场景标题随改。

## Impact

- Spec:`openspec/specs/{agent-core,cli-service,provider-management,webui,watchdog}/spec.md`。
- 代码:`src/provider.rs`、`src/agent_backend.rs`(env 回退+错误文案)、`src/config.rs` 与 `sebas-router/src/config.rs`(配置节迁移 shim)、`src/cli.rs`(隐藏别名)、`sebas-dispatch/src/provider_state.rs`(serde alias)及相关测试。
- 文档:`openspec/glossary.md`(去掉"旧名…隐藏别名"描述)、`tests/acceptance/COVERAGE.md`(gateway-* 能力名与 `SEBAS_AGENT_GATEWAY_URL` 备注)。
- 兼容性:未发布,无存量用户受影响。
