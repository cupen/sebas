# Proposal: session-slash-commands

## Why

操作员在 webui composer 里无法使用 agent 的斜杠命令（`/goal`、`/compact` 等）：没有命令面板，不知道有什么可用，也不知道 agent 是否支持。调研已证实（源码级）：claude code 的 ACP/SDK 层把 `/` 开头的 prompt 文本原样当命令执行（`/goal <condition>`、`/compact` 均为真内建），opencode 同样解释 `/` 前缀（`/compact` 硬编码映射到 summarize），但 opencode 对**未广告**命令静默吞掉（返回空转 `end_turn`，无报错）——不加前端防护时，敲错命令表现为一次「假装完成」的空回合。两侧 agent 均已实现 ACP 命令发现（`available_commands_update`），sebas 却没有消费。

## What Changes

- **命令发现**：driver 侧新增 `AcpEvent::AvailableCommands`——claude 路径取自 `cc-agent-sdk` `get_server_info()` 初始化握手返回的 `commands`；通用 ACP 路径解析 `SessionUpdate::AvailableCommandsUpdate`（`agent-client-protocol` 2.1.0 已内建该变体）。引擎把命令表物化进会话快照（沿 ModelChanged/ModeChanged 模板），经既有会话载荷/WS 更新到达 webui，不新增 API 端点。
- **composer 命令面板**：输入首字符 `/` 触发浮层（经典命令面板：命令名 + 参数提示 + 说明），按前缀增量实时过滤；↑↓ 选择、Esc 关闭、Enter/Tab 两段式（先补全命令+空格、焦点留在输入框继续打参数，再次 Enter 才提交）。仅接 workbench composer。
- **原样透传**：命令文本即普通消息，走既有提交链路（含 busy/queued），sebas 不做翻译层——agent 自行解释。
- **未支持拦截**：手输未被会话广告且不在通用集（`compact`）的命令时，composer 内联提示「该会话的 agent 不支持此命令」并阻止提交、令其重新输入——专治 opencode 静默吞陷阱。`/goal` 因此只对 claude 会话可用（opencode 无此命令，源码核实）。
- **诚实退化**：无发现能力的会话（native 引擎等）不渲染面板，`/` 前缀按普通文本放行。

## Capabilities

### New Capabilities

- `session-slash-commands`: 会话斜杠命令全链——agent 侧命令发现（claude SDK / 通用 ACP 两条 intake）、命令表物化进会话快照、composer 面板交互（增量过滤/两段式提交）、原样透传与未支持拦截。

### Modified Capabilities

（无——AcpEvent 增变体、SessionInfo 增可选字段均为加法；composer 既有投递承诺不变。）

## Impact

- 后端：`sebas-acp`（AcpEvent 枚举 + claude driver 取命令表 + acp_driver codec 新 match arm）、`sebas-dispatch`（引擎物化命令表进会话状态/SessionInfo）、`sebas-webui` crate（会话载荷透出新字段）。
- 前端：`workbench-composer.ts`（面板渲染、过滤、键盘、拦截）及其测试。
- 依赖：无新增 crate——`cc-agent-sdk` 0.1.7 与 `agent-client-protocol` 2.1.0 的现有 API 即够。

## Non-goals

- 不做命令的 sebas 侧翻译/改写（无参数模板扩展、无本地执行）。
- 不接新建会话对话框（初始 prompt 无命令语义）。
- 不做 `_session/goal`、`_session/steering` 等 ACP 扩展协商（claude 适配器的目标控制扩展），只用标准命令透传。
- 不在飞书/IM 面接命令面板（IM 消息同样原样透传，属既有行为）。
- 不改 native 引擎（AgentEvent 词汇不动；native 会话无命令发现即诚实不渲染）。
