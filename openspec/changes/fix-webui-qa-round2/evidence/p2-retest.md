# 四 P2 GUI 复测记录（task 4.2）

日期：2026-09-29 · 运行环境：testsuite-webui Playwright 场景（chromium，fake-claude 桩）
最终结果：**4/4 PASS**（`_p2-retest.spec.ts`，存档为 p2-retest.spec.ts.bak；正式回归覆盖已并入套件既有 spec——parallel-permissions / unread-badge / settings / projects 均绿）

| QA 编号 | 旅程 | 结果 | 证据 |
|---|---|---|---|
| D-C5 | `parallel` → 两张审批卡**并发可见**（count===2 轮询通过）→ 稳定定位（data-request-id）异判（允许+拒绝）→ 回合 done → 允许侧 ok 文本与拒绝侧 "denied by fake" 并存可读 | PASS | p2-retest-dc5-two-cards.png / p2-retest-dc5-results.png |
| D-C3 | `perm` → 允许 → tool_result "perm done" 可读 + 环后正文 "perm turn finished" 在场 + 「已执行」标识可见 | PASS | p2-retest-dc3.png |
| D-B215 | `crash` → 侧栏点脱离 working（终态）→ **立即**再提交被接受 → 新回合完成、无「回合停滞」卡 | PASS | p2-retest-db215.png |
| D-B11 | 手动路径输入框逐键键入 `C:\definitely\not\existent` → inputValue 逐字符保真（String.raw 基准）→ 提交走诚实拒绝（预检 hint 呈现）| PASS | p2-retest-db11-typed.png / p2-retest-db11-error.png |

## 补充观察（不立项）

- 预检 hint 出现后提交钮的禁用态会随预检重跑短暂回弹（flap）——轻微打磨项，不影响合同（保真/去抖/原因如实均已证）。
- 复测过程中的两个测量层教训已写进 spec 注释：JS 字符串字面量会在解析层吃反斜杠（探针必须 String.raw）；`done` 是短暂态、轮询要用终态集合（done|dormant）。

## 全量回归（同日）

- 浏览器套件 `invoke testsuite-webui`：**105 passed / 1 failed / 1 skipped**——唯一失败 `core-freeze-pending-ack` 为 linux-first 平台限制（win32 抛「需要 POSIX 信号」），非回归。
- 进程级 `invoke testsuite-e2e`：**17/18**——唯一失败 `test_model_tool_loop_runs_with_permission_and_records_usage` 为预存 Windows 平台限制（sebas-agent bash 工具 unix stub，探针目录不创建）。
- 两个上轮确定性红已转绿：unread-badge.spec 2/2（D-R2A 聚焦写锚竞速）、settings.spec 8/8 含 S5a（D-R2B 定位歧义）。
