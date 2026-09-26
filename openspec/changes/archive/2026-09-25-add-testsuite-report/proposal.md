# Proposal: add-testsuite-report

## Why

三个自动化套件（testsuite-e2e ~68 case、testsuite-acceptance 10 journey、testsuite-webui 45 spec）跑完后只有 cargo/Playwright 的平铺控制台输出，回答不了「这次验收具体测了哪些 case、各自结果与耗时」；QA 只能靠人工翻日志（`.artifacts/verify/*.log`）汇总。需要每次跑套件自动产出一份按树形结构组织、标题简洁的验收报告。

## What Changes

- 新增报告生成器（Python，与 tasks.py 同层）：解析套件输出，把本次实际执行的 case 按树形结构渲染。
- 三个 invoke 套件入口（testsuite-e2e / testsuite-acceptance / testsuite-webui）跑完自动生成报告，无额外手工步骤：
  - Rust 套件（e2e / acceptance）：优先经 nextest 采集（原生 per-case 耗时 + 结构化输出）；环境无 nextest 时降级为 cargo 输出解析（无 per-case 耗时，失败详情从 `failures:` 块归属解析）。
  - webui 套件：经 Playwright 自定义 reporter 采集（原生 per-case 耗时 + describe 树）。
- 报告输出：
  - HTML 自包含单文件，固定路径覆盖写 `.artifacts/verify/report-<suite>.html`；
  - 终端同步打紧凑树（✅/❌ 前缀、分组缩进）；
  - 每 case 耗时；失败 case 附失败摘要与保留的沙箱路径。
- Rust 两个套件的测试文件按功能重组进模块树（树 = 模块路径），**函数名保持不变**——`--case` 过滤器与 COVERAGE.md 证据引用不断链。
- 套件退出码语义不变：报告是附加产物，不改变通过判定。

## Capabilities

### New Capabilities

- `testsuite-report`: 验收报告生成能力——三个套件入口的报告产出契约（采集来源、树形结构来源、HTML/终端双输出、耗时与失败详情、cargo 降级、退出码中性）。

### Modified Capabilities

（无——三个套件既有 requirement（一键入口、沙箱隔离、退出码语义、单用例手动运行）均不变；模块重组是实现细节，不改 observable 行为。）

## Impact

- `tasks.py`：三个套件任务接入报告生成；`.artifacts/` 进 gitignore。
- `tests/testsuite_e2e_test.rs`、`tests/testsuite_acceptance_test.rs`：测试函数归入模块树（搬移，不改名）。
- `tests/testsuite-webui/`：新增采集用 reporter。
- 新增 `scripts/testsuite_report.py`（或 tasks.py 内模块）。
- 开发环境可选依赖：cargo-nextest（缺失自动降级，不装也能跑）。

## Non-goals

- 不覆盖 testsuite-real-agents（真实凭据、operator 手跑，无报告价值）。
- 不做静态用例目录、不做报告历史留存与两次运行对比。
- 不改任何套件的通过判定、沙箱隔离与退出码语义。
- 不引入 nightly 工具链依赖；不重写套件 test harness。
