# Tasks: add-testsuite-report

## 1. 报告器本体

- [x] 1.1 建 `scripts/testsuite_report.py`：数据模型（suite → 分组树 → case：name/title/status/duration/failure_excerpt/sandbox_path）+ HTML 渲染（自包含内联 CSS）+ 终端紧凑树渲染（✅/❌/⏭ 前缀 + 缩进 + 秒级耗时），单测覆盖渲染与截断。验证：`python3 -m pytest scripts/` 或等价单测绿；样例数据生成的 HTML 浏览器打开即读。
  - 状态：完成。`python3 scripts/testsuite_report.py --selftest` 40/40 绿（数据模型/树计数/终端树/HTML 自包含与转义/截断/cargo 解析/JUnit 解析/Playwright JSON 解析/webui 分片归并/沙箱路径/退出码中性）。pytest 未安装故用内嵌 unittest。
- [x] 1.2 `.artifacts/` 进 `.gitignore`，建 `.artifacts/verify/` 目录约定（无文件不提交）。验证：`git status` 不再显示 `.artifacts/`。
  - 状态：完成。`/.artifacts/` 已加；`git check-ignore -v .artifacts/verify/probe.log` 命中，`git status` 不显示。

## 2. webui 通道（Playwright）

- [x] 2.1 新增 `tests/testsuite-webui/tests/reporters/collect-json.ts`：与 `keep-on-fail.ts` 并列进 reporter 数组，把 Playwright JSON 结果（describe 链、标题、duration、error message、附件路径）落标准输出/文件供 Python 侧读。验证：`invoke testsuite-webui --case <smoke>` 跑完产出含全部 case 的 JSON。
  - 状态：完成。reporter 已挂进**全部六个** config 的 reporter 数组（与 keep-on-fail 并列，互不干扰）。分片命名 `webui-results.json` / `webui-results-<stem>.json`，与 tasks.py `_WEBUI_SHARDS` 逐一对齐（已实测比对）。`tsc --noEmit` 对本文件零报错。未跑真实 Playwright（留给 review 阶段）。
- [x] 2.2 tasks.py `testsuite_webui` 接报告器：捕获 JSON → 树形 HTML 写 `.artifacts/verify/report-webui.html` → 终端打树。验证：跑后文件存在且树与本次运行的 spec/describe 一致，退出码语义不变。
  - 状态：完成。多配置聚合方案：六配置各自写分片，tasks.py 跑前清分片、跑后 `--shards` 归并成一棵树（选择理由记于 design.md D2 补充）。已用样例分片离线验证归并出的树与终端输出（分片→分组树→每 case 耗时/失败摘要）。真实 Playwright 跑留给 review 阶段。

## 3. Rust 套件模块重组（纯搬移）

- [x] 3.1 `tests/testsuite_e2e_test.rs` 测试函数归入文件内 `mod` 树（按功能分组：会话生命周期 / 通道与监督 / router 与 provider / mode 与模型 / pending 与队列 / 流式与转录 / 沙箱与状态目录等），函数名/断言/属性不变，use 路径修正。验证：`cargo test --test testsuite_e2e_test -- --ignored` 全量绿；`cargo test --test testsuite_e2e_test <既有用例名> -- --ignored` 过滤器仍命中。
  - 状态：完成（63 个测试函数归入 8 个 `mod`：`session_lifecycle` 11 / `channel_and_supervision` 9 / `router_and_provider` 8 / `mode_and_model` 4 / `pending_and_queue` 6 / `streaming_and_transcript` 15 / `scenario_projection` 5 / `sandbox_and_state_dir` 5）。38 个 helper/const 提到文件顶部共享区（`mod` 内 `use super::*;` 取用）。**纯搬移的机械证明**：行多重集 diff 显示原文件 0 行丢失、仅新增 32 行脚手架（8×`mod X {`、8×`use super::*;`、8×`}`、4 空行 + 4 行注释横幅）——体不做二次缩进，多行 `\` 续行字符串字面量逐字保留。裸函数名集合 before/after diff 为空（63/63），63 个 `<name>` 过滤器逐一验证各命中 1 个用例。`cargo test --test testsuite_e2e_test --no-run` 绿。
  - 注意：`--list` 输出按设计 D1 变为 `mod::fn`（design.md D1 已预置该形态，`scripts/testsuite_report.py:_group_and_name` 已按 `suite::mod::case` 解析）——故 raw `--list` diff 非空，**属预期**；铁律检验落在裸函数名集合（为空）。全量 `--ignored` 实跑属组 5。
- [x] 3.2 `tests/testsuite_acceptance_test.rs` 同法归入 journey 分组 mod。验证：`invoke testsuite-acceptance` 全量绿，journey 名与 COVERAGE.md 引用逐一对应。
  - 状态：完成（10 个 journey 归入 4 个 `mod`：`session_and_turn` 2 / `workbench_and_projects` 3 / `model_and_provider` 3 / `remote_node` 2）。3 个 helper（`create_session` / `wait_turn_done` / `wait_bootstrap_token` / `wait_node_status` / `wait_session_row`）留在顶部共享区。**纯搬移机械证明**：原文件 0 行丢失、仅新增 21 行脚手架；裸 journey 名集合 before/after diff 为空（10/10）；10 个 `<name>` 过滤器逐一各命中 1 个用例；`cargo test --test testsuite_acceptance_test --no-run` 绿。`tests/acceptance/COVERAGE.md` 引用的是裸 journey 名（不含 mod 前缀），故引用零断链。全量实跑属组 5。
- [x] 3.3 COVERAGE.md / spec 文档中引用的用例名抽查回归（grep 引用 vs `cargo test -- --list`）。验证：引用零断链。
  - 状态：完成。**引用零断链**：(A) `tests/acceptance/COVERAGE.md` 的 10 个 `J:` journey 锚点全部逐字命中 acceptance 套件（10/10，且作为过滤器各命中 1 个用例）；(B) 全仓 markdown（openspec/ + tests/acceptance/）中出现的 37 个 e2e 用例名，**重组前后集合完全一致**，且逐一作为 `<name>` 过滤器各命中 1 个用例（37/37）；(C) `E \`…\`` 锚点中 21 条字面名全部命中，另 3 条为文档简写（`startup_failure_*`、`startup_failure_core/run`、`permission_loop_allow_once/deny/allow_session_over_core_channel`）——含 `*`/`/` 故从来不是可执行过滤器，其展开的真实用例名（`startup_failure_core/run_exits_75_with_summary`、`permission_loop_allow_once/deny/allow_session_switches_auto_over_core_channel`）全部在场，且该状态重组前后无变化（非本次引入）。COVERAGE.md 引用的是裸名，不含 mod 前缀，故不受影响。
  - 集成确认：`scripts/testsuite_report.py:_group_and_name` 对新的 `mod::fn` 名正确切分为（分组路径, 裸名）——`session_lifecycle::cancel_…` → `(('session_lifecycle',), 'cancel_…')`，与 design.md D1 预置形态一致，组 4/5 采集零额外映射。

## 4. Rust 采集双通道

- [x] 4.1 tasks.py `testsuite_e2e` / `testsuite_acceptance` 接采集：检测 `cargo nextest --version`，有则 `cargo nextest run --message-format junit`（JUnit XML → per-case status/耗时/失败文本），无则 cargo 控制台行解析 + `failures:` 块归属（宽容解析，未知行忽略）。单测覆盖两种解析器（样例 JUnit XML / 样例 cargo 输出）。验证：单测绿；两种通道各实跑一次均产出报告。
  - 实测：单测绿（`--selftest` 40 tests OK，含 JUnit / cargo 两解析器）。**cargo 通道实跑**：`invoke testsuite-e2e --case graceful_exit_removes_channel_socket` → 1 passed，probe 打印 `cargo-nextest not found — falling back to cargo output parsing`，报告树与 `.artifacts/verify/report-e2e.html` 只含该用例（`67 filtered out`）。**JUnit 通道**：本机**未装** cargo-nextest（`cargo nextest --version` → no such command），无法以真实二进制实跑；改以 nextest 形状的 JUnit fixture 经 CLI 驱动（`--channel junit --input fixture.xml`）端到端验证：模块路径正确成树（`channel_and_supervision/`、`streaming_and_transcript/`）、per-case 耗时、失败摘要含 message + panic 行、HTML 成文。**残留缺口**：真实 nextest 二进制下的 `--run-ignored all --junit-path` 组合未经实跑（装 nextest 即可补，属一次性操作，未擅自安装工具链）。
- [x] 4.2 套件总耗时计时 + 失败沙箱路径捕获（沿用失败保留现场输出约定）写入报告。验证：人为弄红一个用例，报告该行带失败摘要与沙箱路径，退出码仍非零。
  - 实测：总耗时写入各报告（`✅ e2e — 1 passed · 9.1s [cargo]`、`✅ webui — 2 passed · 1.5s`）。失败摘要经 `failures:` 块归属验证（喂入红样例，报告该行带 excerpt）。沙箱路径捕获：以真实保留措辞与路径命名喂入，报告 HTML 含 `/tmp/sbtestsuite.abc123`——命中 `sbtestsuite.*` 路径规则。**软缺口**：`tests/support/mod.rs` 的保留措辞是 `[sandbox] kept for diagnosis (logs inside): <path>`，不匹配 `_SANDBOX_PATTERNS` 的 `sandbox scene kept at:` 字面；当前靠目录命名规则兜住，若日后沙箱目录改名会静默退化（见汇报）。退出码非零：走 `_report_rust_suite` 返回 `not passed` → 调用方 `raise SystemExit(1)`，与原实现同路径；报告侧 `warn=True` 不影响判定。

## 5. 端到端验收

- [x] 5.1 三入口各实跑一遍（或全量太慢时 e2e `--case` 单跑 + acceptance `--case` 单跑 + webui 单 spec），核对：HTML 树与终端树一致、只含本次运行的 case、`--case` 单跑报告只含该用例、无 nextest 降级路径不报错。验证：三份 report-*.html 存在且内容与运行一致。
  - 实测（三入口各实跑，均 `--case` 单跑形态）：e2e `graceful_exit_removes_channel_socket` → 1 passed / 67 filtered，树 `channel_and_supervision/`；acceptance `session_lifecycle_journey` → 1 passed / 14 filtered，树 `session_and_turn/`；webui `first-paint` → 2 passed，树 `chromium/ → first-paint.spec.ts/ → 工作台首屏/ → 首屏结构与 reachability/` 两级 describe 保留，per-case 耗时在。三份 `.artifacts/verify/report-{e2e,acceptance,webui}.html` 均成文；降级路径（无 nextest）不报错。**首跑暴露真缺陷**：webui 报告曾渲染 0 case（`collect-json.ts` 的 `REPO_ROOT` 少算一级，分片落到 `tests/.artifacts/`）——已修（`../../..` → `../../../..`）并让 tasks.py 显式钉 `TESTSUITE_REPORT_JSON`；复跑得 2 passed。
- [x] 5.2 报告器故障注入（输出重定向到不可写路径）确认退出码中性。验证：测试通过时退出码 0 不受报告错误影响，报告错误以 warning 呈现。
  - 实测：`--out /proc/definitely-not-writable/r.html` → 终端仍打印正常树，输出 `[report] WARNING: could not write report: [Errno 2] ...`，**exit 0**；tasks.py 侧 `_emit_report` 亦为 `warn=True` 且早返回。判定与报告解耦，中性成立。
