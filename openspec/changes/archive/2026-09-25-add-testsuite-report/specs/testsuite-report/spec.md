# testsuite-report Delta

## Purpose

为三个自动化测试套件（进程级 e2e、验收旅程、浏览器级 webui）提供自动生成的运行报告：每次跑套件即产出按树形结构组织、标题简洁的 HTML 报告与终端树，展示本次实际执行的 case、结果与耗时，免除人工翻日志汇总。

## ADDED Requirements

### Requirement: 套件运行自动产出报告

三个套件入口 `invoke testsuite-e2e`、`invoke testsuite-acceptance`、`invoke testsuite-webui` SHALL 在套件运行结束后自动生成该套件的运行报告，无需任何额外手工步骤。报告 SHALL 覆盖本次运行实际执行的 case（含 `--case` 单跑过滤的情形）。testsuite-real-agents 不产出报告。

#### Scenario: 全量运行产出报告

- **WHEN** 操作员执行 `invoke testsuite-e2e`（或另两个入口）且套件跑完
- **THEN** 报告文件被写出、终端打印树形摘要，无需额外命令

#### Scenario: 单用例运行的报告只含被跑的 case

- **WHEN** 以 `--case <name>` 只跑一个用例
- **THEN** 报告只呈现该用例及其分组路径，不列出未运行的 case

### Requirement: 树形结构呈现

报告 SHALL 以树形结构组织 case：Rust 套件（e2e / acceptance）的树来自测试的模块结构，浏览器套件的树来自 Playwright describe 层级。每个 case 行 SHALL 带简洁标题与结果标记（通过/失败/跳过）。模块重组后测试函数名 MUST 保持不变，既有 `--case` 过滤器与 COVERAGE.md 证据引用 MUST 继续可用。

#### Scenario: 按分组浏览通过面

- **WHEN** 查看任一套件的报告
- **THEN** case 按功能分组逐级缩进呈现，每组标题可见，case 行带 ✅/❌/⏭ 标记

#### Scenario: 用例名与过滤器不破链

- **WHEN** 模块重组完成后执行 `invoke testsuite-e2e --case <既有用例名>`
- **THEN** 该用例被定位并运行，COVERAGE.md 中引用的用例名全部仍可解析

### Requirement: 耗时与失败详情

报告 SHALL 为每个 case 附耗时；采集通道拿不到 per-case 耗时（cargo 降级形态）时 MUST 如实缺省而非伪造。失败 case SHALL 附失败摘要（断言/panic 信息）与为事后排查保留的沙箱或现场路径。

#### Scenario: 失败 case 可定位现场

- **WHEN** 套件中某用例失败
- **THEN** 报告中该行带失败摘要与沙箱路径，读者可不经翻日志直接定位排查现场

#### Scenario: 降级形态诚实缺省耗时

- **WHEN** 环境无 nextest、Rust 套件经 cargo 运行
- **THEN** 报告附套件总耗时，case 行耗时如实缺省，报告不因此失败

### Requirement: 双输出形态与落盘

报告 SHALL 同时输出两形态：HTML 自包含单文件（无外部资源依赖），固定路径覆盖写 `.artifacts/verify/report-<suite>.html`；终端同步打印紧凑树。HTML 文件 MUST NOT 被提交进 git。报告产出 MUST NOT 改变套件退出码语义（报告生成失败不掩盖测试失败，测试失败也不因报告存在而反转）。

#### Scenario: HTML 打开即读

- **WHEN** 用浏览器打开 `.artifacts/verify/report-<suite>.html`
- **THEN** 树形报告完整呈现，无需本地服务器或网络资源

#### Scenario: 报告与测试结果解耦

- **WHEN** 报告生成环节自身出错（如写文件失败）
- **THEN** 套件退出码仍如实反映测试通过与否，报告错误单独提示

### Requirement: Rust 套件采集通道自动降级

Rust 两个套件的采集 SHALL 优先使用 nextest（per-case 耗时与结构化结果）；运行环境无 nextest 时自动降级为 cargo 控制台输出解析，套件运行 MUST NOT 因缺少 nextest 而失败。

#### Scenario: 无 nextest 的环境照常出报告

- **WHEN** 在未安装 nextest 的机器执行 `invoke testsuite-e2e`
- **THEN** 套件经 cargo 正常运行并产出报告（无 per-case 耗时），命令不报错
