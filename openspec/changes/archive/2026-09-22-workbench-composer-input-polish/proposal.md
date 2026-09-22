## Why

两处输入框体验积怨：① slash 命令面板把命令名、参数提示、描述全部内联铺开（描述占满整行、claude 广告几十条命令），面板又高又遮挡，选命令像读文档；② claude 会话的右下角模型芯片永远显示「无可用模型」——claude 专用驱动硬编码 `model: None` 并明文拒绝 SetModel，而底层 `cc-agent-sdk` 早已提供 `set_model()` 控制协议，属于「有轮子没接线」，运营者多次点名仍未落地。

## What Changes

- 命令面板行收敛为「`/命令名` + 参数提示」，描述默认不再渲染；描述移入 hover / 键盘高亮同步出现的悬浮气泡：markdown 渲染（复用既有 sanitize 管线）、尺寸有上限、内部可滚动。
- claude 驱动接入 `cc-agent-sdk::set_model()`：内置 claude 模型别名表（default/opus/sonnet/haiku），可被 `[acp.claude] models` 配置覆盖；当前模型从会话帧观察（session_start/assistant 帧的 model 字段）。
- 模型芯片语义补齐：无选项但可观察到当前模型的会话，芯片只读展示当前模型（替代误导性的「无可用模型」）；有别名表的 claude 会话芯片可点开切换，走驱动新通道；通用 ACP 路径（configOptions）不动。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `acp-model-selection`: 「agent 无模型选项 → 模型面缺席」修订为 claude 驱动提供配置可覆盖的别名表与 `set_model` 通道、可观察当前模型的会话芯片只读展示。
- `session-slash-commands`: 「每行展示名称 + 参数提示 + 描述」修订为描述经 hover/高亮气泡呈现（markdown、限尺寸、可滚动、键盘可达同步）。

## Impact

- `sebas-acp/src/claude/driver.rs`：SetModel 接线（替换现役拒绝分支）、别名表与配置覆盖、会话帧 model 观察上报 `AcpModelInfo`。
- `sebas-webui/frontend`：`workbench-composer.ts`（面板行结构、气泡组件、芯片只读态）、`styles`；单测同步。
- 配置：`[acp.claude] models` 新键。
- 前置：`session-slash-commands` change 仍处 Complete 未归档态，其规范需先归档同步进 `openspec/specs/`，本 change 的 delta 才有锚点。

## Non-goals

- 不动通用 ACP 的 configOptions 模型路径与 `session/set_config_option` API。
- 不做模型列表的运行时探测（claude 控制协议无列表能力，别名表 + 配置覆盖已够）。
- 不动面板的两段式补全、过滤、拦截语义。
- 不做 creation 对话框模型预选语义的任何改动（preselect-last-used-model 已定）。
