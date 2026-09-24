# add-acp-stream-approval-journeys — Tasks

## 1. 桩：并行工具环剧本

- [ ] 1.1 `tests/bin/fake-claude.rs` 新增 `parallel` 触发词剧本：单回合两个 tool_use（不同 tool_id/工具名或参数）+ 两个 hook_callback 连发使其同时待批（第二个发出时第一个未决），决定逐一到达后各自落 tool_result（allow → 成功文本，deny → is_error），两个都落定后发环后正文 + result 正常收尾；复用 `wait_hook_decision` 机制，决定乱序到达（先答第二个请求再答第一个）时各自路由到正确 request_id。验证：cargo build 出桩，桩单测/既有桩测试不红
- [ ] 1.2 决定组合验证（allow/allow、allow/deny、deny/deny）：桩内按各自 request_id 的决定独立落 tool_result。验证：进程级用例三组合各断言 tool_result 与决定一致

## 2. 进程级 journey（testsuite-process-e2e）

- [ ] 2.1 `tests/testsuite_e2e_test.rs` 新增 ACP 并行权限 journey：经 webui HTTP API + WS 驱动（不绕用户面）——以 `parallel` 触发词会话提交，断言两个不同 request_id 各自泊车（`GET /api/sessions/{key}/approvals` 可见两条）、逐一 POST answer 后回合推进至终态、两工具结果如实呈现。验证：`invoke testsuite-e2e --case <name>` 全绿，全程无真实上游外呼
- [ ] 2.2 确定性复核：同一 journey 重复执行两次，断言转录形状与终态一致（id/时间戳除外）。验证：两次输出 diff 结论记入 PR 描述

## 3. 浏览器级 journey（testsuite-webui-browser）

- [ ] 3.1 `tasks.py` 浏览器沙箱新增 `claude-thinking` agent 装配段（`args = ["--scenario", "thinking", "--slow-ms", "800"]`，惯例对齐 claude-empty/claude-stream）；`parallel` 用例走既有 claude 档 + 触发词，不新增装配。验证：`invoke testsuite-webui-server` 起沙箱，`GET /api/agents` 可见新 agent
- [ ] 3.2 新增 `tests/testsuite-webui/tests/parallel-permissions.spec.ts`：并行剧本会话提交 → 两张审批卡片各自独立出现（两个 request_id 独立卡片，不合并）→ 逐一决策 → 回合推进、两工具结果呈现。验证：用例全绿，零固定 sleep，retries: 0
- [ ] 3.3 新增 thinking 呈现用例（挂入既有会话页 spec 或新文件）：thinking/正文交替回合 → 过程折叠在场且默认收起、正文独立按序展示、结算后顺序与内容不丢失。验证：用例全绿
- [ ] 3.4 流式结算切换断言：增强既有流式用例（claude-stream + drip）——回合进行中 live-tail 纯文本上屏（既有断言），回合结算后同一正文转 markdown 渲染、拼接一致、无重复条目。验证：用例全绿（保留原中途断言与 retries 纪律）

## 4. 账本与收口

- [ ] 4.1 `tests/acceptance/COVERAGE.md` 加行：并行审批卡片（permission-flow「并行工具调用独立 request id」锚）、thinking 过程折叠真链路（agent-workbench「过程折叠」锚）、流式结算切换（live-turn-stream「流式分批」锚），逐行标注 journey/浏览器用例来源。验证：账本行锚点格式与既有行一致（`<requirement>「<scenario>」`）
- [ ] 4.2 全量收口：`invoke testsuite-e2e`、`invoke testsuite-webui`（或既有等价一键入口）整体不红；既有 perm/tool-loop/stream/drip 用例行为不变。验证：两个入口全绿或既有豁免口径如实记录
