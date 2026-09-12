# Design: workbench-agent-identity-and-process-folds

## Context

对话流渲染现状（`transcript-view.ts`）：assistant 回合硬编码 `assistant`
作者 + `AI` 头像；回合内 thinking 按连续 run 平铺成多个 `<details>`
折叠、工具条目收进一个「used N tools」组，组内是逐条 div，无标题摘要。
工具条目的 wire（`sebas-dispatch` `TurnEntry`）只有 markdown `content`
（`📖 **{tool}**\n```json args``` ` 与 `✓ **{tool}**\n{result}`），没有
结构化字段。操作者消息发出后无接收反馈。

## Goals / Non-Goals

Goals：agent 展示名进对话、已收到角标、单过程大折叠 + 二级折叠、工具
条目结构化标题与中间截断。

Non-Goals：头像/图标体系、工具参数编辑与结果 diff、wire 序列语义变更
（只增可选字段）。

## Decisions

### D1: agent 展示名的解析放在前端组装层

`<sebas-transcript-view>` 新增 `agentDisplay: string | null` 属性，由
dashboard（持有聚焦会话与 `/api/agents` 目录）按 `agent_kind` 匹配
`display` 传入；回退链 display → slug → `assistant`。备选：后端在
entry 上带 agent 名——否，agent 目录前端已有，且会话切换时名字随
属性走，不污染持久化数据。

### D2: `TurnEntry` 增可选 `title`，后端构造

- 字段：`#[serde(default, skip_serializing_if = "Option::is_none")] pub
  title: Option<String>`——旧持久化条目反序列化为 None，无需迁移。
- 构造点：`sebas-dispatch/src/engine/mod.rs` 的 ToolStart/ToolEnd 落盘
  处（现有 `format!("📖 **{tool}**…")` 旁），title = `{tool_name} ·
  {key_arg}`；ToolEnd 用 `✓ {tool_name} · {key_arg}` 区分完成态。
- key_arg 提取：args JSON 里按偏好键序取第一个命中的字符串值——
  `path, file_path, absolute_path, file, dir, directory, cwd, pattern,
  url, command, query`；全未命中取第一个字符串值；再未命中则只有
  工具名。长度上限 200 字符（防异常超长），截断只发生在展示层。
- `sebas-webui` 的 `ConversationEntryView` 同步加可选 `title` 透传。

### D3: 回合分块改为「一个过程块」

复用现有 `buildTurnUnits` 分块器，规则改为：回合内 thinking + tool
条目全部汇入**单个** process 块（落在首个过程条目的位置），文本段照旧
按流序留在块外。process 块渲染为 `<details>`（默认收起，summary 概要
如 `process · N entries`），内部逐条二级 `<details>`（thinking 段/
工具条目各一折，默认收起），二级 summary 显示 title（无 title 回退
通用标签，thinking 用稳定通用标签）。条目 DOM 以 `position` 为键保持
身份稳定，避免重渲染丢展开态。

### D4: 已收到角标由视图推导，不加发送方状态机

判定：会话**最后一条** entry 是操作者 prompt，且会话状态为 Working
（输出未开始）→ 该气泡渲染低调「已收到」角标；下一条 agent entry 到达
（不再是 prompt 收尾）→ 条件不成立，角标自然消失。纯派生态，refetch/
重连后自动正确，composer 不需要改 sendMessage 流程。会话状态已由
dashboard 持有，作为属性传入。

### D5: 中间截断在前端做，title 属性保全量

CSS 无原生 middle-ellipsis，用 JS helper：超过 ~64 字符时保留首 28 +
`…` + 尾 28；折叠 summary 上设 `title` 属性携带完整字符串供悬停查看。

## Risks / Trade-offs

- [旧条目无 title] → 回退通用标签（spec 已有场景），不报错。
- [Lit 重渲染重置二级折叠展开态] → 条目 DOM 以 position 为键维持
  身份；`details` 的 open 是 DOM 态，结构不变时 lit 不重建。
- [title 提取启发式漏键] → 偏好键序覆盖主流工具（read/edit/bash/
  glob/grep/web 类）；未命中退化为纯工具名，不阻塞。
- [大折叠把多 run 过程并置一处] → 文本段位置不变，可读性取舍已在
  proposal 记录；后续如需按 run 分组可在块内分段，不影响 wire。

## Migration Plan

wire 只增可选字段（serde 缺省），新旧二进制与旧持久化条目互不破坏；
无部署步骤。回滚即回退二进制（新条目的 title 字段旧代码忽略）。

## Open Questions

- 过程大折叠的 summary 文案（`process · N` vs 中文「过程 · N」）实现
  时随现有 UI 语言风格定，不影响契约。
