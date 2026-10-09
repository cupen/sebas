## 1. Wire：两个载体的 call id 出得来（先落底层）

- [x] 1.1 `sebas-acp`：`AcpEvent::ToolEnd` 增可选 `tool_use_id`（`#[serde(default,
  skip_serializing_if = "Option::is_none")]`），driver 在 `User(tool_result)` 分支把已查表用的
  `tool_use_id` 一并带上（当前 `driver.rs:1140` 用完即弃）。验证：codec/driver 单测断言
  `ToolEnd.tool_use_id == "tc-par-1"` 形态，且旧 JSON（无该键）反序列化为 `None`。
- [x] 1.2 `sebas-acp`：`ToolStart` 同样带上 `tool_use_id`（`ContentBlock::ToolUse` 的 `t.id`
  现成）。验证：driver 单测断言 ToolStart 携带 id，与同回合 ToolEnd 的 id 相等。
- [x] 1.3 `sebas-agent`：`AgentEvent::ToolStart` / `ToolEnd` 增可选 `tool_use_id`；
  `TurnEmit::tool_start` / `tool_end` 与 `emit_tool_end` 增 id 参数，取值即内核已有的
  `calls[i].0` / `calls[batch + k].0`（与 `PermissionRequest.request_id == tool_use_id`
  同源契约）。验证：`sebas-agent` 单测断言同一调用的 start/end 事件 id 相等，且并行批次内
  两个调用的 id 互异。

## 2. Wire：`TurnEntry` 字段 + 后端标题（契约闸门）

- [x] 2.1 `sebas-domain`：`TurnEntry` 增 `tool_use_id: Option<String>`（serde 默认值 +
  `skip_serializing_if`，与 `title` 同族写法）。验证：`session.rs` 单测覆盖「带 id 往返一致」
  与「None 不上 wire（JSON 不含该键）」两条。
- [x] 2.2 `sebas-domain`：确认 `golden_session_vocabulary.json` 往返闸门仍绿（样本无该字段 ⇒
  零变化即证明可选字段不破坏旧载荷）。验证：`cargo test -p sebas-domain golden` 通过；若
  样本需补带 id 的条目，则新增一条并保持既有 7 条不变。
- [x] 2.3 `sebas-dispatch`（ACP 路径）：`AcpEvent::ToolStart/ToolEnd` 的 id 写入
  `TurnEntry.tool_use_id`；`ToolStart` 沿用 `tool_entry_title(false, …)`，`ToolEnd` 因 wire
  无 args 仍为 `tool_entry_title(true, tool_name, None)`。验证：`engine/events.rs` 或 engine
  投影单测断言工具条目携带 id 与结构化标题。

## 3. native 载体升为一等 tool 条目

- [x] 3.1 `src/native_dispatch_bridge.rs`：`ToolStart` / `ToolEnd` 分支由
  `TurnEntry::markdown(...)` 改为 `TurnEntry::tool(...)`，补 `.with_title(...)` 与
  `tool_use_id`（复用 `sebas-dispatch` 的标题规则，必要时把 `tool_entry_title` 提升为可复用
  入口而非复制规则）。验证：该模块单测断言投递的条目 `element_type == tool` 且标题形态为
  `Read · <path>`。
- [x] 3.2 `src/agent_backend.rs`：同一投影点做同样改造（`land(...)` 的 element_type 传
  `"tool"` 并带 title/id）。验证：该模块单测断言条目类型与标题；确认
  `turn_visible_output` 记账语义不变（`tool` 与 `markdown` 同在可见输出表内）。
- [x] 3.3 回归确认：跑 native 相关既有单测，确认没有断言依赖「工具痕迹是 markdown」。
  验证：`cargo test` 中 native 模块用例全绿，或逐条说明并更新失效断言。

## 4. 前端：合并块与单棵过程树

- [x] 4.1 `transcript-view.ts`：新增按 `tool_use_id` 配对逻辑——把 `📖` 调用条目与 `✓` 结果
  条目合并为一个 `ToolCallBlock`；配不上的调用自成一块（调用态标题，无 ✓/✗）。验证：前端单测
  覆盖「配对成功合并为一块」「未配对自成一块」「并行同名工具按 id 正确配对」。
- [x] 4.2 `transcript-view.ts`：移除 `ToolResultRun` run 类型与顶层 `renderToolResultRun`，
  工具调用块与 thinking 同为过程折叠的子节点（单棵树）；`renderToolResultRun` 的
  ✓/✗ outcome 章迁到合并块收起行与过程折叠汇总行。验证：前端单测断言回合内 DOM 只有
  `div.process-fold` 一层树，且不存在与过程折叠并列的顶层结果块。
- [x] 4.3 `transcript-view.ts`：合并块与 thinking 子折叠均**默认收起**（`foldOpen` 未命中即
  收起）；展开体渲染参数段 + 结果段，两段各走既有 `truncateHtml` + 「查看全部」弹层。验证：
  前端单测断言默认态 `aria-expanded="false"`、折叠体不在 DOM；展开后同时含参数与结果文本。
- [x] 4.4 `transcript-view.ts`：`toggleFold` 只写自身 id、不触碰任何祖先（D7 层级不死锁），
  并保留 `data-testid="tool-result-entry"` 与 `data-denied`。验证：前端单测断言「展开子折叠后
  父折叠仍 `aria-expanded="true"`」。
- [x] 4.5 `client.ts`：`ConversationEntryView` 增 `tool_use_id?: string | null` 并透传。
  验证：类型检查通过 + 单测断言 view 映射保留 id。

## 5. 前端既有断言翻新

- [x] 5.1 更新 `transcript-view.test.ts` 中依赖「顶层结果块默认展开」「工具条目留在过程折叠内」
  的既有用例（含 `isDecidedToolResult` 相关 5 条与 lift 到顶层的断言），改为新契约。验证：
  `pnpm test`（vitest）全绿。
- [x] 5.2 更新 `dashboard.test.ts` 等引用 `tool-result-entry` / 过程折叠结构的用例。验证：
  前端测试套件全绿。

## 6. 浏览器验收（native 装配）

- [x] 6.1 `playwright.native.config.ts`：`testMatch` 正则纳入新 spec 文件。验证：配置加载后
  新文件被执行（`--list` 可见用例）。
- [x] 6.2 新增 `tests/testsuite-webui/tests/tool-call-fold.spec.ts`，用 `test/tool-use`：
  断言工具调用块**默认收起**、收起标题含工具名 + 关键参数、点击后展开见参数与结果、过程折叠
  与子块层级不死锁（展开子块后父块仍开）。验证：该 spec 在 9894 装配下通过。
- [x] 6.3 同 spec 增 `test/tools-parallel` 用例：一回合多工具各成一块且互不串扰（若场景复用
  同一 id，则只钉「各自成块」，并在用例注释说明理由）。验证：该用例通过。
- [x] 6.4 确认既有浏览器用例未因口径变更而失实（尤其 `thinking-process-fold.spec.ts` 的过程
  折叠默认收起断言、`conversation.spec.ts` 的折叠旅程）。验证：相关 spec 全绿。

## 7. 集成与端到端收口

- [x] 7.1 `cargo build` + 全量 `cargo test` 通过（含 `ipc_protocol_contract_test.rs` 与
  `golden` 闸门），确认新字段未破坏兼容面。验证：命令全绿。
- [x] 7.2 沙箱联调（AGENTS.md 配方，`SEBAS_HOME` 钉一次性目录）：ACP 载体的 `perm` 回合
  与 native `test/tool-use` 回合各跑一次，经 API 核对工具条目携带 `element_type=tool`、标题与
  `tool_use_id`。验证：`GET /api/sessions/<key>` 的条目字段与预期一致，GUI 呈现为合并块。
- [x] 7.3 刷新持久化一致性：完成一回合后刷新页面，确认合并块仍按 id 正确配对（配对为纯派生、
  无增量状态）。验证：浏览器用例或手测中刷新后形态不变。