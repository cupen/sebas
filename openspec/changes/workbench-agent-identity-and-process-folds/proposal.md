## Why

工作台对话流目前把 agent 回合渲染成通用的「assistant / AI」身份，看不出
这条会话绑的是哪个 agent（Claude Code、Codex 还是内置内核）；思考与
工具条目散落在多个平铺折叠里、折叠标题只有「thinking / used N tools」，
无工具名与参数摘要；操作者发出消息后到流式回复开始前没有任何「已收到」
反馈。需要一轮对话流的可读性打磨。

## What Changes

- **Agent 身份**：assistant 回合的作者标识 SHALL 显示会话绑定 agent 的
  展示名（如 `Claude Code`），数据取 agent 目录（`/api/agents` 的
  `display`，按会话 `agent_kind` 匹配；缺失回退 slug）。头像图形暂缓，
  维持文本形态（可用展示名首字母）。
- **已收到角标**：操作者消息被服务端接受（条目进入会话载荷）且该回合
  agent 输出尚未开始流式返回期间，消息气泡上显示低调的「已收到」角标；
  agent 回合开始输出后角标消失。
- **过程大折叠**：agent 回合内的全部 thinking + tool 条目收进**一个**
  默认折叠的大折叠（替代现在按 run 平铺的多个 thinking 折叠 + 工具组），
  文本段仍按流序留在大折叠外；展开后是**第二层**逐条折叠（每个思考段、
  每次工具调用一折），二级折叠也默认收起。
- **折叠标题摘要**：工具条目由后端补充结构化标题（工具名 + 关键参数，
  如 `read · src/main.rs`），二级折叠收起时显示该标题；过长时中间
  省略（首…尾截断）。历史条目无标题字段时回退通用标签。
  wire 变更：`TurnEntry` 增可选 `title` 字段（serde 缺省 None，
  旧持久化条目兼容）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities
- `agent-workbench`: 「Workbench renders the focused session as a
  conversation」的折叠结构改为「单个过程大折叠 + 二级逐条折叠、默认
  全收起」；新增需求——assistant 回合显示绑定 agent 展示名、操作者
  消息的已收到角标、工具条目结构化标题与中间截断展示。

## Impact

- `sebas-dispatch`：`TurnEntry` 增 `title: Option<String>`；工具
  ToolStart/ToolEnd 落盘时构造标题（工具名 + 路径类参数提取）。
- `sebas-webui`：`models.rs`/`api.rs` 透传 `title`；
  `transcript-view.ts` 重构回合分块（过程大折叠 + 二级折叠 + 标题 +
  中间截断）、作者名取 agent 目录；composer/dashboard 增已收到角标态。
- 测试：dispatch 单测（标题构造、旧条目 None）、transcript-view /
  composer 前端测试、e2e 冒烟。

## Non-goals

- 不做头像图形/图标体系（文本形态先行，后续单独做）。
- 不做工具调用的结构化参数编辑或结果 diff 视图（折叠内仍渲染现有
  markdown 内容）。
- 不改条目的 wire 序列语义（position/kind/element_type 不动，仅增
  可选字段）。
