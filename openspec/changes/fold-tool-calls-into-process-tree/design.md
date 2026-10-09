## Context

见 `proposal.md` —— Why。本设计只记录塑造做法的现状与约束。

**现状（已核实）**：

- 前端 `transcript-view.ts` 把一次工具调用渲染成**两处**：`📖` 调用条目留在过程折叠内，`✓`
  结果条目被 `isDecidedToolResult`（`:360`）**提升**为顶层 `tool_result` run，且默认展开
  （`:2519` 用 `!== false`）。过程折叠与二级条目本身已默认收起（`:2578` / `:2661` 用 `=== true`）。
- 后端 `tool_entry_title`（`sebas-dispatch/src/engine/events.rs:132`）已按
  `KEY_ARG_PREFERRED` 键序（path → file_path → … → pattern → url → command → query）产出
  `Read · src/main.rs` 形态的结构化标题，200 字符上限。
- `AcpEvent::ToolEnd`（`sebas-acp/src/session.rs:153`）与 `AgentEvent::ToolEnd`
  （`sebas-agent/src/session/mod.rs:45`）**都只有 `tool_name` + `result`，没有 call id**；
  但两侧上游都持有 id 并在用完后丢弃：ACP driver 用 `tool_use_id` 查表拿工具名后丢弃
  （`claude/driver.rs:1140`），native 内核 `calls[i].0` 即 id、`gated_execute` 也收
  `tool_use_id`（`loop_/mod.rs:477`）。
- **两个载体不对称**（本次一并收口）：ACP 走 `TurnEntry::tool(...).with_title(...)`
  （`engine/mod.rs:1495`），而 native 两个投影点写 `TurnEntry::markdown(...)`
  （`native_dispatch_bridge.rs:97/108`、`agent_backend.rs:572/586`）——native 的工具调用
  以正文形态呈现，无 `tool` 类型、无标题、无 id。
- native 内核已把 `request_id == tool_use_id` 写成契约（`session/mod.rs:52`），id 现成可用。
- 历史包袱：`fix-webui-qa-round2` D-C3 曾因「点二级折叠连带折叠父级」导致结果内容不可达，
  才把结果条目提升为顶层默认展开块。本次改成树形嵌套，**该死路不得回归**。

## Goals / Non-Goals

**Goals:**

- 回合内过程呈现为**单棵过程树**：过程折叠（根）→ thinking 小折叠 / 工具调用块（子节点），
  各自默认收起。
- 一次工具调用 = **一个**合并块，标题即带工具名 + 关键参数，收起态即可辨识「读了哪个文件 /
  跑了什么」。
- 两个载体（ACP / native）产出一致的**一等 tool 条目**（`element_type = tool` + `title` +
  `tool_use_id`），使 spec 的行为在两个载体上等价成立。

**Non-Goals:**

- 审批卡（`sebas-review-cards`）不动。
- 不重写 `tool_entry_title` 的偏好键序。
- 飞书卡片侧的折叠形态不动（本期只 webui 转录）。
- 不取消「过程折叠」概念本身。

## Decisions

### D1 — 单棵过程树，取消顶层 tool_result 块

工具调用块与 thinking 小折叠同为过程折叠的子节点；`ToolResultRun` 这一 run 类型从
`AgentRun` 联合中移除，`renderToolResultRun` 合并进 `renderProcessItem`。

**备选与否因**：保留顶层块与过程折叠并列（现状）会让同时存在两套层级语言，且 `✓` 块默认
展开正是刷屏来源。

### D2 — 配对键用 `tool_use_id`，不靠位置

`tool_use_id` 从上游一路补到前端：`AcpEvent::ToolEnd` / `AgentEvent::ToolStart|ToolEnd` 各增
可选字段 → dispatch 写入 `TurnEntry.tool_use_id` → webui view 类型透传 → 前端按 id 配对。

**备选与否因**：位置 FIFO 在并行同名工具（Read A + Read B）下会错配；纯相邻配对在转录截断时
退化丢标题。上游契约明确以 `tool_use_id` 为唯一关联键（[Parallel tool use](https://platform.claude.com/docs/en/agents-and-tools/tool-use/parallel-tool-use)），
driver 已持有该 id，补齐近乎零成本。

### D3 — wire 演进遵守协议三规则

`TurnEntry` 新增 `tool_use_id: Option<String>`，带 `#[serde(default, skip_serializing_if =
"Option::is_none")]`——与既有 `title` / `failure_class` 同族写法。旧持久化条目与该字段缺省时
不上 wire，缺口字节形状零变化；`golden_session_vocabulary.json` 的往返闸门继续通过（样本里
没有该字段即证明零变化），另有新增用例覆盖带 id 的往返。

### D4 — 标题规则沿用既有 `KEY_ARG_PREFERRED`

合并块标题 = 调用条目的结构化 `title`（后端已产出）。展开体 = 调用参数段（`📖` 条目的
markdown + json）＋ 结果段（`✓` 条目的内容），两段各走既有 `truncateHtml` + 「查看全部」弹层。

**备选与否因**：按工具名定制显示字段（bash 优先 description）需要第二份规则表，新工具即退化；
现键序已覆盖 read/edit/write、glob/grep、web fetch/search、bash 的主流参数形态。

### D5 — 未配对的调用自成一块（调用态标题）

结果未到（仍在跑）/ 转录被截断 / 回合中断时，`📖` 调用仍渲染为独立合并块，标题为调用态
（无 `✓`），展开可见参数。不丢任何调用与参数。

### D6 — native 载体升为一等 tool 条目（本次纳入）

两个 native 投影点改为 `TurnEntry::tool(...).with_title(tool_entry_title(...))` 并带上
`tool_use_id`；`TurnEmit::tool_start/tool_end` 与 `emit_tool_end` 增 id 参数，id 取内核已有的
`calls[i].0`（`ToolStart`）与 `calls[batch+k].0`（`ToolEnd`）——即 `request_id == tool_use_id`
的同一值。

**备选与否因**：不升级则 native 会话不产出工具块，spec 的载体中立行为在 native 上不成立，且
已定的 native Playwright 验收无事可验（native 现有浏览器用例也确实从不断言过程折叠）。

### D7 — 层级不死锁（D-C3 死路不得回归）

子折叠的开合**绝不能**连带父折叠。现实现是 `button` 兄弟节点 + 条件渲染（非原生 `details`），
本就不冒泡；本次把它写成显式断言（`toggleFold` 只写自己的 id），并加浏览器用例钉住
「展开子节点后父节点仍展开」。

### D8 — 验收走 native 装配 + 新 spec

新增 `tests/testsuite-webui/tests/tool-call-fold.spec.ts`，用 `playwright.native.config.ts`
（需扩其 `testMatch` 正则）与 `test/tool-use` / `test/tools-parallel` 场景模型；断言合并块
默认收起、标题带关键参数、展开见参数+结果、并行各成一块、未配对退化。ACP 载体侧由既有
`thinking-process-fold.spec.ts` 的同类口径兜底。

## Risks / Trade-offs

- **[跨四 crate 的 wire 演进，改动面大]** → 分层落地：先 `sebas-acp` / `sebas-agent` 出 id，
  再 `sebas-dispatch` / `sebas-webui` 投影，最后前端渲染；每层各自单测，最后浏览器验收。
- **[native 投影从 markdown 改成 tool 会改变现有 native 会话的呈现形态]** → 这是有意的口径
  收口，但需在 tasks 里显式覆盖既有 native 断言（含 `agent_backend.rs` 的
  `turn_visible_output` 记账口径——`tool` 与 `markdown` 都在可见输出表内，语义不变）。
- **[改默认展开为收起会与 `permission-flow` 现有明文相抵触]** → 本 change 同时提
  `permission-flow` 的 MODIFIED delta 改写口径（收起标题自带关键参数 + 一次点击可达 +
  层级不死锁），不留悬空矛盾。
- **[前端已有测试引用 `tool-result-entry` 选择器]** → 保留该 testid 与 `data-denied`，只换内容
  结构与新增子节点选择器，避免大面积改断言。
- **[`TurnEntry` 是不可变转录的一部分，配对依赖两条都在场]** → 配对是纯派生（每次渲染都从
  完整条目序列重算），不做增量状态机，刷新后自然一致。

## Migration Plan

无库表迁移：`tool_use_id` 是可选字段，旧转录条目缺省即 `None`，前端退化为「未配对调用自成
一块」形态（D5），不报错、不丢内容。回滚即还原渲染分支（wire 上的可选字段可长期留存，不构成
兼容负担）。

## Open Questions

- `test/tools-parallel` 场景在 native 下是否对每个 tool_use 都发独立 id（而非复用）——实现时
  读场景模型确认；若复用，则该用例只钉「各自成块」不钉「按 id 精确区分」，精确区分由 ACP
  侧的并行用例承担。这不改变 spec 或任务划分。