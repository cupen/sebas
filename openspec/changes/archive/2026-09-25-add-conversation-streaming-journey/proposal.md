## Why

对话流的"实时上屏"从未被真正验收。唯一的流式浏览器用例 `streaming.spec.ts` 以**服务端 API** 为主 oracle（轮询 5 个 chunk 条目），只在回合结束后断言 DOM 里有一个合并气泡，并把 `working` 瞬态**降级为 annotation、不做硬断言**——前端哪怕只在回合结束时整块上屏，该用例照样绿。

同时，"回合进行中"这个窗口今天**无法被确定性构造**：fake-claude 的 `--slow-ms` 是"收尾前一次性停顿"（`tests/bin/fake-claude.rs:844`），而 driver 的 watchdog 探测超时只有 **1.5s**（`tasks.py` 沙箱配置注释），超过即被判 hang 杀灭。

后果已实证：本会话在真实 claude 拓扑下复现工作台静默卡在"排队中"（`active/queued` → `dormant`，0 entry、无 error），而全部浏览器/进程级旅程仍判绿；操作员直接反馈"前端对话流一直没有实现实时流式更新"。

## What Changes

- 新增一条**专用对话流 e2e 用例**：回合进行中（会话仍为 `working`）断言 DOM 已出现增量正文，随后收敛 `done`；零固定 sleep、不依赖 retry 兜底。
- 扩展 `fake-claude` 桩：新增"按时间间隔发出 delta"的能力，使"回合进行中"窗口可在 watchdog 预算内确定性构造。
- 收紧行为契约：对话视图的流式增量 SHALL 在回合进行中即渲染进该回合正文，SHALL NOT 延迟到回合结束整块上屏。
- 收紧套件口径：`testsuite-webui-browser` 的「流式分批」子功能从"中途存在瞬态"改为"必须在 DOM 观察到中途上屏"。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`：对话视图的流式实时性——回合进行中增量上屏，而非结束时整块渲染。
- `testsuite-webui-browser`：「agent 对话覆盖」的流式子功能与 scenario 收紧为可确定性断言的中途上屏。

## Non-goals

- 不修"回合起不来静默排队"（本会话已定位到 `src/dispatch.rs` / `sebas-acp` 的 resume 回落路径；另立 change）。
- 不做"单项目多会话、各自发消息"的 UI 全程用例，不修 `expandAllProjects`（`helpers/pages/workbench.ts:163`）的点击竞态。
- 不把旅程接进 CI 门禁（现有 101 浏览器 + 57 进程级 + 10 验收用例均不在 CI）。
- 不引入真实凭据；不触碰真实实例 9797 与 `~/.sebas`。

## Impact

- `tests/testsuite-webui/tests/streaming.spec.ts`（改造为专用对话流旅程）
- `tests/bin/fake-claude.rs`（新增间隔发 delta 的 flag/scenario）
- `tests/acceptance/COVERAGE.md`（补对应行）
- `openspec/specs/agent-workbench/spec.md`、`openspec/specs/testsuite-webui-browser/spec.md`
- 仅当用例证伪实时性时：`sebas-webui/frontend/src/views/transcript-view.ts`
