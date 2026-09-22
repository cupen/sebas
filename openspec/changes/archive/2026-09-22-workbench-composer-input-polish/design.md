## Context

- 命令面板现状：`workbench-composer.ts` `renderCommandPalette()` 每行内联 `name + hint + description`（描述 `flex-basis: 100%` 独占一行），`max-height: 260px` 内滚动；claude 从 `get_server_info()` 握手广告几十条带长描述的命令，行高翻倍后面板占掉大半个对话区。两段式补全/过滤/拦截语义（同 spec 其余要求）成熟，不动。
- 模型选择现状：通用 ACP 路径从 `configOptions` 提取 `AcpModelInfo`，工作正常；claude 专用驱动（stream-json + control 协议）`DriverHandle.model` 恒 `None`，`SetModel` 命令命中显式拒绝分支（"当前驱动不支持"）。依赖 `cc-agent-sdk 0.1.7` 已提供 `set_model(Option<&str>)`（control 协议）与 `set_permission_mode` 同款通道。claude wire 的 system `session_start` 帧与 assistant 帧携带 model 名，可观察 current。
- 用户未参与轮次问答，以下决策为默认裁决（按 review 推荐执行，工件评审时可推翻）。

## Goals / Non-Goals

**Goals:** 面板回到「一行一命令」的紧凑密度；claude 会话获得与通用 ACP 对等的模型切换体验；只读 current 展示取代误导性「无可用模型」。

**Non-Goals:** 见 proposal；另加——不改 `AvailableCommandInfo` wire 形状（description 原样在 wire 上，只是呈现层收进气泡）；不动会话头/创建对话框的既有模型入口。

## Decisions

- **D1 气泡 = 面板行内绝对定位的纯 CSS 浮层，不做全局 teleport。** 面板本就是 `.input-wrap` 锚定的浮层（与 model 菜单同配方），气泡作为高亮行的 `position: absolute` 子元素向右弹出即可；`overflow-y: auto` 的滚动容器会剪裁子级浮层——气泡因此挂在面板容器（`.cmd-palette`）而非滚动行上，按高亮行 offsetTop 定位。备选 wa-tooltip 被否：内容需要 markdown + 滚动 + 键盘同步，原生 tooltip 承载不了。
- **D2 气泡参数：`max-width: 360px`、`max-height: 240px`、内部滚动。** 面板行宽已收窄到内容宽度（行收敛后 `min-width` 取消，改自适应 + 上限），右侧空间不足时 CSS 无且优雅的翻转方案——直接固定右弹、容器 `overflow` 剪裁风险用「气泡右缘对齐面板右缘」规避（右对齐而非左锚）。
- **D3 气泡触发 = `:hover` ∨ 键盘高亮，两态共用同一渲染。** 高亮态已有 `.highlighted` 类，气泡随该类渲染即可；a11y 门禁下键盘路径与鼠标路径天然同源。`renderMarkdown()`（marked → DOMPurify 既有管线）直接复用，`unsafeHTML` 注入。
- **D4 claude 别名表内置 + `[acp.claude] models` 覆盖。** 内置 `["default", "opus", "sonnet", "haiku"]`（对齐 claude CLI 自身 /model 词汇；`default` 映射 SDK `set_model(None)`）。claude 专属驱动内含 claude 专属词汇不违反「agent 自广告」原则的实质——该原则针对的是通用路径假装知道某 agent 的命令；claude 驱动本就是逐驱动知识，且给了配置覆盖逃生口。备选「仅配置驱动、无内置表」被否：零配置体验回退到「无可用模型」，等于没修。
- **D5 current 观察：session_start 帧优先，assistant 帧兜底。** 驱动已有两帧的 model 字段解析（usage 统计在用），提为 `observed_model: Option<String>` 共享单元，`AcpModelInfo { current, options }` 在握手成功时由别名表 + 观察值拼装；切换成功本地乐观写 current，后续帧覆盖（自愈错位）。SDK `set_model` 无失败回执语义（control request 发出即成功），agent 不认识的 id 表现为「后续帧仍是旧 model」→ 观察值覆盖即自然纠偏，错误提示不需要额外协议。
- **D6 SetModel 拒绝分支替换而非并存。** 现分支删除，`AcpCommand::SetModel` 走 `client.set_model()`；`"default"` 特判为 `None`。manager 侧 `get_model_info`/快照通道（`AcpModelInfo` 已有）零改动复用。

## Risks / Trade-offs

- [claude CLI 版本差异：老版本不认识 set_model control request] → SDK 层对未知 control response 已有容错；切换后 current 不变的兜底是 D5 的帧观察纠偏，UI 不误报成功后的错值。
- [别名表与真实 model id 不一致（如 `sonnet[1m]` 变体）] → 配置覆盖是官方逃生口；`--model` 全名亦可写进 `[acp.claude] models`。
- [气泡在移动端/窄屏没有 hover] → 键盘高亮路径完整可用（同一渲染）；触摸用户点选行即补全，描述属增强信息。
- [行收敛后 hint 过长仍可能截断] → hint 维持既有 ellipsis 截断，完整文本进 `title`（既有行为）。

## Migration Plan

纯加法（配置键、驱动分支、呈现层）+ 一处替换（SetModel 分支），无数据迁移。core 未升级的 webui 组合下 claude 会话照旧「无可用模型」（字段缺省语义）；反向组合（新 core 旧前端）快照形状不变。回滚 = 回退二进制。

## Open Questions

（无）
