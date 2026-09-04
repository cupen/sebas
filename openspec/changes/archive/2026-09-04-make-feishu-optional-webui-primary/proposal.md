## Why

feishu 的功能开关是「查漏即启用」的隐式方式（`app_id`/`app_secret` 同时非空才接入），没有显式的「接通/断开」意图表达；且当前飞书消息**只能驱动 Claude Code 桥**（`Out::SpawnAcp`），原生 sebas-agent 内核（`sebas-agent`）只有 webui 的 `NativeAgentBackend` 走进程内直连可达——feishu 侧永远用不到同一套 agent 能力。用户希望「默认以 webui 为主控端，打通 feishu → sebas agent 的通信链路，让我既可以用 webui 使用 agent 功能，也可以用 feishu」。

## What Changes

- **feishu 显式启用开关**：`[feishu] enabled`（默认 `false`）。在 `run` / `webui_cmd` 启动路径上显式判断，与 `app_id`/`app_secret` 双非空的历史隐式启用并存并互相确认；缺 `enabled` 时按历史行为回退。进程以 webui 主控（无飞书）形态运行时，`enabled` 默认关闭。
- **feishu → 原生 sebas-agent 内核链路**：router dispatch 新增 native 分支——feishu 会话按 manifest/配置选择原生内核（`agent-*` 前缀）或 acp 桥。原生内核的权限请求经审查卡桥接到 webui（而非 feishu 卡片），工具轨迹/文本回包经 webui 事件流与 API 呈现。
- **webui 默认主控验证与文档化**：watchdog 默认 `webui.enabled=true`、`core.enabled=false`（现状即如此），本 change 验证这一默认部署形态，并补齐「feishu 可选」的 spec 表述与部署文档。

## Capabilities

### New Capabilities
- `feishu-option`: 飞书接入作为可选项的显式开关、接入判定、以及「webui 主控 + feishu 辅助」的部署形态下双通道共享同一会话状态的行为面。

### Modified Capabilities
- `agent-workbench`: 原生 sebas-agent 内核从「webui 专属后端」扩展为「webui 与飞书共用执行体」，feishu 会话可选走原生内核，权限审查经 webui 呈现。
- `feishu-bridge`: 入站处理在 chat-type/@bot 过滤之外新增「feishu 启用开关」判定；会话执行体在 acp 桥之外新增原生内核路径（权限请求不再强制渲染 feishu 卡片）。

## Impact

- `src/`：`config.rs`（`FeishuConfig.enabled` 字段 + 校验）、`run.rs`（启用判定）、`dispatch.rs`（native dispatch 分支）。
- `sebas-agent/`：暴露被 router/dispatch 引用的内核句柄（`SessionManager`/`SessionHandle` 已具备）。
- `sebas-webui/`：复用现有 `NativeAgentBackend`（agent-* 会话）与审查卡通道，作为 feishu 原生会话的呈现侧。
- 配置：`config/config.toml` 示例增加 `[feishu] enabled`。
- 无新增 crate 依赖。飞书侧不存在（禁用）时行为与现 webui 主控完全一致。