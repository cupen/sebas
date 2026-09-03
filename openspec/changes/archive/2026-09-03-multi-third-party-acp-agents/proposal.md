# multi-third-party-acp-agents

## Why

`sebas-acp` 当前只接 Claude Code 一种三方 agent：整个 crate 是 `pub mod claude;`，`AcpConfig` 下只有一个 `claude` 字段，webui 会话创建下拉的三方后端档位只有一个 `acp · Claude Code bridge`。要支持 Codex、Gemini 等更多三方 agent，不能每个都复制一套会话管理——需要在 `sebas-acp` 里抽出一层驱动抽象。调研确认：ACP（Agent Client Protocol）已从 Zed 独立为开源标准（v1 stable，crates.io `agent-client-protocol` 累计 411 万下载），Gemini/Copilot/Cursor 等 30+ 个 agent 原生 ACP；唯 Claude Code 与 Codex 无原生 ACP、靠 adapter 桥接，且是产品上值得专用维护的 T0 级 agent。

## What Changes

- **新增 `AgentDriver` 驱动层**（`sebas-acp` 内）：trait + 两种实现——`ClaudeDriver`（保留 `cc-agent-sdk`，换取 Claude 专有的 `UsageUpdate` token 计数）与 `AcpDriver`（用 `agent-client-protocol` v1 驱动任意原生 ACP agent）。两者都输出统一的 `AcpEvent`/`AcpCommand` 防腐层词表。
- **配置 schema 迁移**：`AcpConfig.claude` → `AcpConfig.agents.<kind>` + `default`；`driver` 字段（`claude`/`acp`）用 serde tag 显式区分。旧 `[acp.claude]` 加载时一次性迁移并 warn。**kind 是开放字符串，不是闭集枚举**——新增原生 ACP agent 只加一条配置、零代码改动。
- **补权限半场**：`InProcessBackend`（ACP 路径）实现 `permission_requests()`/`answer_permission()`，让 Claude 会话的权限请求首次能进 webui 审查卡（此前只能走飞书卡片，`DualSessionBackend` 的 acp 回退是死代码）。
- **权限 request_id 命名空间**：`<kind-slug>:<raw-id>` 防止跨驱动冲突。
- **`sebas agent-kinds list` 子命令**：可达性探测（`command` 存在 + 版本探测），webui 下拉由探测结果驱动。

## Non-goals

- 不实现 native 内核与三方 agent 之间的协议转换。
- 不在本 change 写 Codex 专用驱动——Codex 通过"通用 ACP 驱动 spawn `codex-acp`"覆盖（`driver = "acp"`）。
- 不迁离 `cc-agent-sdk`（保留，风险记入 design R2；迁离作为后续独立 change 候选）。
- 不在 webui 加"会话内切换 kind"UI；kind 绑死在创建时。
- 不引入 `unstable_protocol_v2`（只走 ACP v1 stable 面）。

## Capabilities

### New Capabilities

- `agent-driver`: `sebas-acp` 驱动层——`AgentDriver` trait + `ClaudeDriver`/`AcpDriver` 两类实现，开放 kind 注册表、配置迁移、跨驱动权限路由、可达性探测。

### Modified Capabilities

（无——`webui`、`session-lifecycle`、`router-commands` 等既有 spec 不改 requirement；后端 hint 字符串 `acp` → `acp:<kind>` 是形式扩展，不改可见行为语义。）

## Impact

- 受影响代码：`sebas-acp/`（新增 `agent_driver.rs` + `acp_driver/` 模块；`SessionManager` 改按 kind 查 driver 注册表）、`src/config.rs`（`AcpConfig` 改 schema + 迁移层）、`src/run.rs`/`src/dispatch.rs`/`src/session_boot.rs`（按 kind 选 driver）、`src/agent_backend.rs`（`spawn_with` hint 扩展）、`sebas-webui/src/session_backend.rs`（补 `InProcessBackend` 权限半场）、`sebas-router/`（暴露 ACP 权限广播）、前端 `{sessions,workbench-composer}.ts`（下拉扩为 kind 列表）。
- 受影响外部：用户配置 TOML（`[acp.claude]` → `[acp.agents.claude]` 迁移 + CHANGELOG 说明）。
- 新增依赖：`agent-client-protocol`（`sebas-acp` 内，stable-v1）。
- 不动 `sebas-agent`（native 内核）、`sebas-gateway`、`sebas-feishu` 核心契约。