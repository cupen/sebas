# tasks — converge-webui-e2e-tree

## 1. 骨架：spec 文件两层 `test.describe` 重排

- [x] 1.1 重排 `tests/testsuite-webui/tests/projects.spec.ts` 与 `settings.spec.ts`：核对两层 describe 标签名与新 requirement 子功能对齐；运行 `invoke testsuite-webui --case projects` 与 `--case settings` 各 1 次确认无回归（验证：两次全绿、it() 总数不变）
- [x] 1.2 重排 `auth.spec.ts`：顶层 `test.describe('鉴权闭环', () => { test.describe('错误凭据', () => { ... }); test.describe('登录与登出', () => { ... }); test.describe('深链重定向', () => { ... }); });`；运行 `invoke testsuite-webui --case auth` 验证全绿（验证：3 个 it 全在两层 describe 内、套件通过）
- [x] 1.3 重排 `session-mgmt.spec.ts`：顶层 `test.describe('会话管理', () => { test.describe('close 与 archive', ...); test.describe('深链与退役路径', ...); test.describe('模型面诚实缺省', ...); });`；运行 `invoke testsuite-webui --case session-mgmt` 验证全绿
- [x] 1.4 重排 `sessions.spec.ts`：把双会话切换（2.1）与 archive 写保护（2.2）拆为子功能；运行 `--case sessions` 验证全绿（验证：2 个 it 全在两层 describe 内）
- [x] 1.5 重排 `models.spec.ts`：把无模型诚实拒绝与 settings provider 只读拆为子功能；运行 `--case models` 验证全绿
- [x] 1.6 重排 `dialog.spec.ts`：把同会话多轮（4.1）与输入守卫（4.2）拆为子功能；运行 `--case dialog` 验证全绿
- [x] 1.7 重排 `session-roundtrip.spec.ts` / `streaming.spec.ts`：归入 `agent 对话覆盖` 大功能，子功能分别为「首回合往返 / 重载恢复」「流式分批」；分别运行 `--case` 验证全绿
- [x] 1.8 重排 `errors.spec.ts` / `first-paint.spec.ts`：归入 `agent 对话覆盖`（errors：拒绝存活/崩溃诚实）与 `工作台首屏`（first-paint 单 it 不再硬拆）；分别运行 `--case` 验证全绿
- [x] 1.9 全量收尾：跑 `invoke testsuite-webui` 全量 3 连绿；it() 总数 33 与重排前一致；spec 文件头补 `> 功能：<requirement> / 子功能：<scenario>` 三行注释（验证：3 连绿 + 12 个文件头均有归属注释）

## 2. 账本：requirement 合并与 COVERAGE 树形化

- [x] 2.1 在 `openspec/specs/testsuite-webui-browser/spec.md` 内合并四条「X 核心功能覆盖」requirement 为「项目管理覆盖 / 会话管理覆盖 / 模型管理覆盖 / agent 对话覆盖」下子功能；scenario 名沿用既有；运行 `openspec validate --change converge-webui-e2e-tree --strict` 验证 delta 通过（验证：strict 模式无报错；specs artifact 状态 done）
- [x] 2.2 重写 `tests/acceptance/COVERAGE.md` 的 `testsuite-webui-browser` 一节为树形表格，列为「大功能 / 子功能 / 用例名 / spec 文件 / 锚点」；每行挂一条用例；与 spec scenario 名逐一校对（验证：COVERAGE 行数等于现行 it 总数 33；spec scenario 名与 COVERAGE「锚点」列完全一致）
- [x] 2.3 更新 `tests/testsuite-webui/README.md` 的「旅程与账本」表格：列对齐 COVERAGE 的五列；标注每个 spec 文件对应的大功能与子功能（验证：README 表格与 COVERAGE 同源；无遗漏条目）

## 3. 防线：preflight 拒绝顶层裸 test

- [x] 3.1 在 `tasks.py` 的 `testsuite_webui` 函数内 `_testsuite_webui_preflight(c)` 调用之前新增 `_testsuite_webui_spec_structure(c)`：用 `rg -nE '^[ ]{0,2}test\(' tests/testsuite-webui/tests/*.spec.ts` 检索；任何命中则 `raise SystemExit(1)` 并把命中文件与行号打到 stderr（验证：故意在 `_probe.spec.ts` 加一个顶层 `test('probe', ...)` 调用 preflight，函数报错且 exit code 非 0）
- [x] 3.2 负向验证：在 `tests/testsuite-webui/tests/` 下临时新建 `_probe.spec.ts` 含顶层裸 `test(...)`；运行 `invoke testsuite-webui --case nonexistent` 验证套件在 preflight 阶段拒绝运行并打印命中位置（验证：stderr 出现 `_probe.spec.ts:<line>`；无沙箱装配日志）
- [x] 3.3 删除 `_probe.spec.ts`；跑 `invoke testsuite-webui` 全量 3 连绿确认 preflight 在干净仓库下不误报（验证：3 连绿 + preflight 步骤无任何命中日志）

## 4. 验收：账本闭环

- [x] 4.1 跑 `openspec status --change converge-webui-e2e-tree --json` 验证四个 artifact 全部 `done`（验证：`proposal/specs/design/tasks` 状态均为 done；`isPlanningComplete: true`）
- [x] 4.2 跑 `invoke testsuite-webui` 全量 3 连绿（验证：与 1.9/3.3 一致的稳定性门槛；COVERAGE 行数等于 33 无丢失）
- [x] 4.3 在 `tests/acceptance/COVERAGE.md` 的 `testsuite-webui-browser` 段落末尾追加一行 `骨架收敛（converge-webui-e2e-tree）` 指向本期 commit hash 与本 tasks（验证：账本自身可追溯本次重构）
（实跑证据：commit ba11f6f/8e66cc9 上全量 `invoke testsuite-webui` 3 连绿，每轮 30 main + 3 auth 全过、33 it 无增减；preflight 负向验证以临时 _probe.spec.ts 顶层裸 test 触发拒绝并打印文件行号后删除。）
