## Why

`tests/testsuite-webui/` 的浏览器级 e2e 套件已用 12 个 spec、33 个 it 覆盖 9 大功能区，但 spec 文件、requirement 命名、COVERAGE 账本三处对“按功能收敛”的对齐不一致：requirement 同时存在「X 旅程」和「X 核心功能覆盖」两套命名；同一项目增删旅程既挂在「工作台/项目/会话管理面」又挂在「项目管理核心功能覆盖」下；只有 projects/settings 走两层 `test.describe`，其余 10 个 spec 把 1–4 个用例摊在一级。这次把骨架拉齐：spec 改成纯功能树（功能 → 子功能 → 用例），spec 文件按子功能两层 `test.describe`，COVERAGE 写成树形视图，新加用例必须在现有子功能下，禁止“另起一摊功能覆盖”。

## What Changes

- **新增**：spec 文件内部统一两层 `test.describe` 结构——顶层对应大功能（与 requirement 一致），二层对应子功能（与 COVERAGE 一致）。未达到该结构的 spec 重排到两层 describe。
- **新增**：requirement 与 spec 文件的双向索引——每个 spec 文件头加注释 `> 功能：<requirement 名> / 子功能：<scenario 名>`，COVERAGE.md 同步成树形表格。
- **新增**：`tests/acceptance/COVERAGE.md` 的 `testsuite-webui-browser` 一节从二维表改写为“大功能 → 子功能 → 用例（spec 文件 / 锚点）”的树形结构；既有旅程条目按新结构重排。
- **修改**：spec 中 `### Requirement: 项目/会话/模型/agent 对话 核心功能覆盖` 四条 requirement 的命名与条款——移除“核心功能覆盖”并列写法，统一为大功能主条目下的子功能场景；同主题 requirement 合并去重。
- **修改**：spec 末尾“渐进补用例”纪律由“无需改 spec 即可接纳”改为“落在现有子功能下、需在 COVERAGE 加行，无需改主 spec”，并在 tasks.py 的 `testsuite_webui` 入口加 preflight 检查（不允许 spec 文件顶层直接出现 `test(...)`，必须包在 `test.describe` 内）。
- **非破坏**：本期不动任何用例语义、不删任何 it，只调文件结构与命名；新结构稳定后再按子功能补用例。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `testsuite-webui-browser`: 「项目管理/会话管理/模型管理/agent 对话 核心功能覆盖」四条 requirement 合并为「大功能 → 子功能」结构；现有旅程与用例映射随之重整；为 spec 文件两层 `test.describe` 结构、`invoke testsuite-webui` 入口 preflight、COVERAGE.md 树形视图建立规约。

## Impact

- 受影响代码：`tests/testsuite-webui/tests/*.spec.ts`（12 个文件重排为两层 describe）、`tests/testsuite-webui/README.md`（旅程表格同步）、`tests/acceptance/COVERAGE.md`（webui 段重写为树形）、`tasks.py` 的 `testsuite_webui`（加 preflight 检查 spec 文件顶层无裸 `test`）。
- 受影响 spec：`openspec/specs/testsuite-webui-browser/spec.md` 的四条 requirement 与三条「工作台/项目/会话管理面旅程」scenario 锚点。
- 不影响：被测 webui 后端（无产品改动）、vitest 单测（不在本次范围）、CI workflow（CI 仍按 `--case`/全量入口运行，无 CLI 变化）。

## Non-goals

- 不动现有 it() 的断言与触发词（任何用例语义变更需另立 change）。
- 不动 vitest 单测层（前端单测有自己的工程化债，跟浏览器 e2e 是两套工程）。
- 不动选择器策略（CSS 深定位 vs `getByRole` 收敛另立 change；本次只做骨架与命名）。
- 不动 CI workflow；本 change 不要求 vitest/Playwright 进 CI（属于搁置的 CI 工作）。
- 不解决既有缺口清单（detached 审批、项目分支端点、watchdog 进程级旅程、record/replay 旅程）——这些已记录在 COVERAGE「缺口清单」，由各自专项推进。