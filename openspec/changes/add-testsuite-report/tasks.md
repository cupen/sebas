# Tasks: add-testsuite-report

## 1. 报告器本体

- [ ] 1.1 建 `scripts/testsuite_report.py`：数据模型（suite → 分组树 → case：name/title/status/duration/failure_excerpt/sandbox_path）+ HTML 渲染（自包含内联 CSS）+ 终端紧凑树渲染（✅/❌/⏭ 前缀 + 缩进 + 秒级耗时），单测覆盖渲染与截断。验证：`python3 -m pytest scripts/` 或等价单测绿；样例数据生成的 HTML 浏览器打开即读。
- [ ] 1.2 `.artifacts/` 进 `.gitignore`，建 `.artifacts/verify/` 目录约定（无文件不提交）。验证：`git status` 不再显示 `.artifacts/`。

## 2. webui 通道（Playwright）

- [ ] 2.1 新增 `tests/testsuite-webui/tests/reporters/collect-json.ts`：与 `keep-on-fail.ts` 并列进 reporter 数组，把 Playwright JSON 结果（describe 链、标题、duration、error message、附件路径）落标准输出/文件供 Python 侧读。验证：`invoke testsuite-webui --case <smoke>` 跑完产出含全部 case 的 JSON。
- [ ] 2.2 tasks.py `testsuite_webui` 接报告器：捕获 JSON → 树形 HTML 写 `.artifacts/verify/report-webui.html` → 终端打树。验证：跑后文件存在且树与本次运行的 spec/describe 一致，退出码语义不变。

## 3. Rust 套件模块重组（纯搬移）

- [ ] 3.1 `tests/testsuite_e2e_test.rs` 测试函数归入文件内 `mod` 树（按功能分组：会话生命周期 / 通道与监督 / router 与 provider / mode 与模型 / pending 与队列 / 流式与转录 / 沙箱与状态目录等），函数名/断言/属性不变，use 路径修正。验证：`cargo test --test testsuite_e2e_test -- --ignored` 全量绿；`cargo test --test testsuite_e2e_test <既有用例名> -- --ignored` 过滤器仍命中。
- [ ] 3.2 `tests/testsuite_acceptance_test.rs` 同法归入 journey 分组 mod。验证：`invoke testsuite-acceptance` 全量绿，journey 名与 COVERAGE.md 引用逐一对应。
- [ ] 3.3 COVERAGE.md / spec 文档中引用的用例名抽查回归（grep 引用 vs `cargo test -- --list`）。验证：引用零断链。

## 4. Rust 采集双通道

- [ ] 4.1 tasks.py `testsuite_e2e` / `testsuite_acceptance` 接采集：检测 `cargo nextest --version`，有则 `cargo nextest run --message-format junit`（JUnit XML → per-case status/耗时/失败文本），无则 cargo 控制台行解析 + `failures:` 块归属（宽容解析，未知行忽略）。单测覆盖两种解析器（样例 JUnit XML / 样例 cargo 输出）。验证：单测绿；两种通道各实跑一次均产出报告。
- [ ] 4.2 套件总耗时计时 + 失败沙箱路径捕获（沿用失败保留现场输出约定）写入报告。验证：人为弄红一个用例，报告该行带失败摘要与沙箱路径，退出码仍非零。

## 5. 端到端验收

- [ ] 5.1 三入口各实跑一遍（或全量太慢时 e2e `--case` 单跑 + acceptance `--case` 单跑 + webui 单 spec），核对：HTML 树与终端树一致、只含本次运行的 case、`--case` 单跑报告只含该用例、无 nextest 降级路径不报错。验证：三份 report-*.html 存在且内容与运行一致。
- [ ] 5.2 报告器故障注入（输出重定向到不可写路径）确认退出码中性。验证：测试通过时退出码 0 不受报告错误影响，报告错误以 warning 呈现。
