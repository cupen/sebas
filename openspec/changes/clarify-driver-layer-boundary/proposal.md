## Why

`agent-driver` 与 `acp-driver` 的边界模糊：agent-driver 的 Purpose 说它抽象「Claude 专用驱动 + 通用 ACP 驱动」并各自 spawn 子进程；acp-driver 的 Purpose 说它「Owns the lifecycle of one Claude Code subprocess per sebas session」，但 acp-driver 的多条 requirement（session/load、config options、model switch）已是通用 ACP 语义。读者无法判断「spawn 一个 ACP 子进程」到底归谁管。实际分工应是：**agent-driver = 驱动抽象/策略层**（AgentDriver trait、kind 注册表、权限跨驱动路由、可达性上报、webui 暴露）；**acp-driver = ACP 子进程运行时层**（单子进程生命周期、事件泵、中断恢复、ACP 协议细节）。

## What Changes

- **`agent-driver` 文本边界重界定**：Purpose 与「AgentDriver abstraction」requirement 明确——本 capability 是驱动抽象/策略层，不承载 ACP 协议与单子进程生命周期细节（那是 acp-driver）；trait 的两个实现把具体 spawn/协议下推给 acp-driver 运行时。
- **`acp-driver` 文本边界重界定**：Purpose 与「One subprocess per session」明确——本 capability 是 ACP 子进程运行时层，向上服务 agent-driver 的 AcpDriver 实现与 router；Claude 专属 vs 通用 ACP 的抽象归属见 agent-driver。
- 纯文本澄清：不改任何 requirement 的 SHALL 语义，不迁移 requirement 归属。

## Capabilities

### New Capabilities

### Modified Capabilities
- `agent-driver`: Purpose + 抽象层边界措辞澄清（MODIFIED）
- `acp-driver`: Purpose + 运行时层边界措辞澄清（MODIFIED）

## Impact

- glossary「执行体」词条补 acp-*/agent-* 分层说明（可选）。
- opencode-agent 引用「acp-driver 的 session/load 路径」语义不变。
- **Non-goals**：不迁移任何 requirement；不合并/拆分两 capability；不改源码；`acp-driver`/`agent-driver` 目录名不变（批次 E 仅清理 `claude-env-cover`/`opencode-agent` 前缀）。
