## Why

一次工具调用在转录里占**两处**且层级语言不一致：📖 调用条目留在过程折叠内，✓ 结果条目被提升为**默认展开**的顶层块——长会话被工具结果内容刷屏，而收起态又看不出「读了哪个文件 / 跑了什么命令」。操作员要求过程统一为**一棵树**，且收起行就自带关键参数。

## What Changes

- **一个工具 = 一个合并块**：📖 调用条目与 ✓ 结果条目按 `tool_use_id` 精确配对后合并，默认收起；标题 = 工具名 + 关键参数（`Read · src/main.rs` / `Grep · TODO` / `Bash · cargo test`）。
- **取消顶层 tool_result 双轨块**：工具合并块与 thinking 小折叠同为过程折叠的子节点，各自默认收起，形成单棵过程树（过程折叠收起行仍给汇总：标签 + 计数 + ✓/✗ 章）。
- **合并块展开体** = 调用参数段 + 结果段，两段都走既有截断 + 「查看全部」弹层口径。
- **未配对的调用**（回合中断 / 历史转录截断）自成一块，保持调用态标题（无 ✓），不丢参数。
- **spec 口径改写**：`permission-flow` 的「无需折叠任何其它条目即可读到结果」改为「标题自带关键参数 + 一次点击可达 + 层级绝不死锁」。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`：过程折叠结构改为单棵过程树（工具合并块取代顶层 tool_result 块）；工具调用/结果按 `tool_use_id` 配对合并；`TurnEntry` 增可选 `tool_use_id` 字段
- `permission-flow`：决策结果可读达口径改写（不再要求零折叠可达，改要求收起标题自带关键参数 + 一次点击可达 + 层级不死锁）

## Impact

- **跨 crate wire 演进（四层）**：`sebas-acp`（`AcpEvent::ToolEnd` 补 `tool_use_id`，driver 已持有该 id）→ `sebas-dispatch`（转录条目写入）→ `sebas-domain`（`TurnEntry` 增字段）→ `sebas-webui`（view 类型 + 渲染）。
- **协议三规则**：新增字段带 serde 默认值 + `skip_serializing_if`（缺口字节形状零变化），同步 golden fixture。
- 前端：`transcript-view.ts` 的 `splitAgentRuns` / `renderProcessRun` / `renderToolResultRun` / `processItemLabel`。
- 验收：新 `tool-call-fold.spec.ts`（native `test/tool-use` / `test/tools-parallel` 装配，需扩 `playwright.native.config.ts` 的 testMatch）。
- 无新增库表；审批卡不动。

## Non-goals

- 不改审批卡呈现（turn 级控件，与本次转录合并无关）。
- 不重写后端 `tool_entry_title` 的偏好键序（沿用现有 `KEY_ARG_PREFERRED`）。
- 不取消「过程折叠」概念，不新增 `test/*` 场景模型。
- 不改飞书卡片侧的折叠形态（仅 webui 转录）。
