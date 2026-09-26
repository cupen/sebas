# add-acp-stream-approval-journeys — Tasks

## 1. 桩：并行工具环剧本

- [x] 1.1 `tests/bin/fake-claude.rs` 新增 `parallel` 触发词剧本：单回合两个 tool_use（不同 tool_id/工具名或参数）+ 两个 hook_callback 连发使其同时待批（第二个发出时第一个未决），决定逐一到达后各自落 tool_result（allow → 成功文本，deny → is_error），两个都落定后发环后正文 + result 正常收尾；复用 `wait_hook_decision` 机制，决定乱序到达（先答第二个请求再答第一个）时各自路由到正确 request_id。验证：cargo build 出桩，桩单测/既有桩测试不红
- [x] 1.2 决定组合验证（allow/allow、allow/deny、deny/deny）：桩内按各自 request_id 的决定独立落 tool_result。验证：进程级用例三组合各断言 tool_result 与决定一致
  - 状态备注：1.1/1.2 完成。桩 `parallel` 触发词 + `wait_hook_decisions`（按 request_id 归档、乱序路由）已落地，桩单测 14/14 绿（含并行 3 条）。**实测发现（偏离 design D3）**：cc-agent-sdk 0.1.7 在 hook 回调分发处跨 await 持回调表锁，第二条 hook_callback 要等第一条决定返回才开始——驱动侧一次只泊一条，两卡不同时在读模型。桩侧「连发」wire 契约成立并由 journal 断言钉死。

## 2. 进程级 journey（testsuite-process-e2e）

- [x] 2.1 `tests/testsuite_e2e_test.rs` 新增 ACP 并行权限 journey：经 webui HTTP API + WS 驱动（不绕用户面）——以 `parallel` 触发词会话提交，断言两个不同 request_id 各自泊车（`GET /api/sessions/{key}/approvals` 可见两条）、逐一 POST answer 后回合推进至终态、两工具结果如实呈现。验证：`invoke testsuite-e2e --case <name>` 全绿，全程无真实上游外呼
- [x] 2.2 确定性复核：同一 journey 重复执行两次，断言转录形状与终态一致（id/时间戳除外）。验证：两次输出 diff 结论记入 PR 描述
  - 状态备注：2.1/2.2 完成，三条 journey 均 `invoke testsuite-e2e --case` 单点实跑绿：`parallel_scenario_parks_each_approval_and_settles_the_turn`（0.53s）、`parallel_scenario_decision_combinations_map_to_their_own_tool`（2.31s，三组合）、`parallel_scenario_transcript_shape_is_deterministic_across_runs`（1.05s，两跑形状逐条一致）。**偏离**：受 SDK 持锁限制，「approvals 可见两条」改为「两个 request_id 各自独立泊车、逐一决策」（second card 在第一张决策后出现）；桩侧「连发/第二个发出时第一个仍待批」由 journal 断言（hook_lines[1] < first_decision）钉死。零真实上游：沙箱无 router，回合由桩驱动，journal 记到 `parallel` 触发词。

## 3. 浏览器级 journey（testsuite-webui-browser）

- [x] 3.1 `tasks.py` 浏览器沙箱新增 `claude-thinking` agent 装配段（`args = ["--scenario", "thinking", "--slow-ms", "800"]`，惯例对齐 claude-empty/claude-stream）；`parallel` 用例走既有 claude 档 + 触发词，不新增装配。验证：`invoke testsuite-webui-server` 起沙箱，`GET /api/agents` 可见新 agent
- [x] 3.2 新增 `tests/testsuite-webui/tests/parallel-permissions.spec.ts`：并行剧本会话提交 → 两张审批卡片各自独立出现（两个 request_id 独立卡片，不合并）→ 逐一决策 → 回合推进、两工具结果呈现。验证：用例全绿，零固定 sleep，retries: 0
- [x] 3.3 新增 thinking 呈现用例（挂入既有会话页 spec 或新文件）：thinking/正文交替回合 → 过程折叠在场且默认收起、正文独立按序展示、结算后顺序与内容不丢失。验证：用例全绿
- [x] 3.4 流式结算切换断言：增强既有流式用例（claude-stream + drip）——回合进行中 live-tail 纯文本上屏（既有断言），回合结算后同一正文转 markdown 渲染、拼接一致、无重复条目。验证：用例全绿（保留原中途断言与 retries 纪律）
  - 状态备注：3.1–3.4 完成，四条 `invoke testsuite-webui --case` 单点实跑绿（`parallel-permissions` 2.5s / `thinking-process-fold` 1.7s / `conversation-streaming` 1.5s；沙箱自动清理）。**实施实测新增两点事实（已在 spec 注释与账本落脚）**：① 并行泊车次序在 SDK 边界**竞速**（实测先 Read 后 Bash，进程级又见 Bash 先），故浏览器用例按 request_id/工具名动态取卡、不写死顺序；② 首张决策后第二张常在同一批 WS 帧内接替，DOM 计数 1→1 无 0 窗口，故断言「换成另一张 request_id 的独立卡片」而非「计数归零」。另两处测试写法坑：宿主 shadow DOM 的 `textContent` 不含影子树（须用穿透 shadow 的 locator 取文本）；转写屏外子树惰性渲染时 `innerText` 会漏文。

## 4. 账本与收口

- [x] 4.1 `tests/acceptance/COVERAGE.md` 加行：并行审批卡片（permission-flow「并行工具调用独立 request id」锚）、thinking 过程折叠真链路（agent-workbench「过程折叠」锚）、流式结算切换（live-turn-stream「流式分批」锚），逐行标注 journey/浏览器用例来源。验证：账本行锚点格式与既有行一致（`<requirement>「<scenario>」`）
  - 状态备注：4.1 完成。树形账本加 3 行（并行卡片锚 `permission-flow「Parallel tool calls each get their own request id」`；thinking 锚 `agent-workbench「process folds interleave in arrival order」`+「folds stay collapsed with a live summary while streaming」；结算切换锚 `live-turn-stream「text streams during a turn」`），格式与既有行一致（大功能｜子功能｜用例名｜spec 文件｜锚点），并新增脚注 ⁹ 记录本轮 delta、双载体分工与 D3 偏离实测。
- [x] 4.2 全量收口：`invoke testsuite-e2e`、`invoke testsuite-webui`（或既有等价一键入口）整体不红；既有 perm/tool-loop/stream/drip 用例行为不变。验证：两个入口全绿或既有豁免口径如实记录
  - 状态备注：4.2 完成（按「全绿或如实记录」口径）。本 change 的用例在**全量**入口内全绿：`invoke testsuite-e2e` → `scenario_projection` 3 条并行 journey ✅（73✅/1❌，唯一红项 `mode_and_model::test_model_switch_takes_effect_on_the_next_turn` 属 extend-test-model-scenarios 领地）；`invoke testsuite-webui` → `parallel-permissions.spec.ts` ✅、`thinking-process-fold.spec.ts` ✅、增强后的 `conversation-streaming.spec.ts` ✅，且既有 `permission.spec.ts`(3✅)/`first-paint.spec.ts`(2✅)/`streaming.spec.ts`(1✅)/`stop-settle.spec.ts`(2✅) 行为不变（102✅/2❌/2⏭）。
  - **非本 change 的红项（HEAD 既红，本 change 未触碰其文件；`git status` 证明相关源文件干净）**：① `cargo test -p sebas-router` 212✅/1❌（`anthropic_wire::thinking_sse_emits_thinking_delta_then_text_delta_in_order`）；② `cargo test --no-run` 与根 `cargo test` 因 **lib 单测编译错**失败（`src/agent_backend.rs:1952/1989` 把 `TurnElementType` 与 `&str` 比较，E0308 ×2）；③ `testsuite-webui` 的 `skills.spec.ts` K4 同步投影面板（隔离重跑必红，连 retry #1 亦红，属 skills 域）；④ `testsuite-webui` 全量链里 `test-model-scenarios.spec.ts` native `test/empty` 一次 `createSession HTTP 409`（隔离重跑 `--case native` 转绿，判为抖动）。①②的落点均在最近两笔提交（`768e3d9` 会话词汇类型化、`4de17f6` router test 模型与原生思考投影）的领地，需其属主收口。
