# add-acp-stream-approval-journeys

## Why

ACP 驱动通路的「会话实时流式更新 + 权限审批」验收存在三处真实缺口：① permission-flow 规范已有「并行工具调用各自独立 request id」契约，但 fake-claude 桩没有并行 tool_use 剧本，任何测试都无法让一个 ACP 回合产生两个待批请求——浏览器级并行审批卡片旅程无从驱动（COVERAGE.md 无条目）；② thinking 呈现（过程折叠组）有前端单测与桩剧本，但浏览器级真链路无用例、账本无条目；③ 流式「显示正确」缺一条显式断言：live-tail 纯文本在回合结算后切换为 markdown 渲染的时刻无人钉住。

## What Changes

- fake-claude 桩新增 `parallel` 触发词剧本：单回合并发两个 tool_use、连发两个 hook_callback 重叠泊车，逐一应答后各自 tool_result 成功、终文本收尾（复用 perm/tool-loop 的 hook 等待机制）。
- 进程级 e2e journey（testsuite-process-e2e）：ACP 并行权限环——两个独立 request_id 各自泊车、各自决策、回合推进。
- 浏览器级 Playwright journey（testsuite-webui-browser）：并行审批卡片各自独立呈现与逐一决策；thinking 呈现真链路用例；live-tail→markdown 结算切换断言。
- COVERAGE.md 账本加行，锚定 permission-flow / agent-workbench / live-turn-stream 既有场景。

## Capabilities

### New Capabilities

（无——验收载体与呈现契约均已归既有 capability，本 change 只补测试缺口。）

### Modified Capabilities

- `testsuite-process-e2e`: ADDED——ACP 桩并行 tool_use 剧本契约 + 并行权限 journey（对应 permission-flow「并行工具调用独立 request id」）。
- `testsuite-webui-browser`: ADDED——ACP 桩驱动的浏览器呈现覆盖：并行审批卡片独立呈现、thinking 过程折叠真链路、live-tail→markdown 结算切换。

## Impact

- `tests/bin/fake-claude.rs`（新增触发词剧本，不改既有场景行为）。
- `tasks.py`（浏览器沙箱按既有 claude-empty/claude-stream 惯例新增 thinking/parallel agent 装配段）。
- `tests/testsuite_e2e_test.rs`、`tests/testsuite-webui/tests/*.spec.ts`、`tests/acceptance/COVERAGE.md`。
- 不碰 router/core/webui 生产代码；与 extend-test-model-scenarios 按**驱动通路**分工（本 change = ACP/桩侧，它 = router test provider/native 侧）。浏览器呈现面在两条通路上各有独立用例属**有意双载体**：事件生产者与链路不同（ACP 子进程→驱动解析→hook 泊车 对比 native 内核直投），互不替代、互不豁免（分工理由见 specs/testsuite-webui-browser 增量）。

## Non-goals

- 不实现 router 内置 test provider 九场景与 native 通路 journey——归 extend-test-model-scenarios（其中「并行审批卡片」「UI 取消」的 native 侧对应用例归它，本 change 不代劳也不豁免）。
- 不重复立项「流式中经 UI 取消」——stop-settle.spec.ts 已覆盖（账本 ✅）。
- 不改审批/流式的生产行为；规范契约（permission-flow、agent-workbench、live-turn-stream）视为已定，本 change 只补驱动与验收。
