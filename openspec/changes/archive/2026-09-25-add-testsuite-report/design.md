# Design: add-testsuite-report

## Context

三个套件入口已固化在 `tasks.py`：`testsuite-e2e` / `testsuite-acceptance` 走 `cargo test --test … -- --ignored`，`testsuite-webui` 走 Playwright（自有多 config 形态与 `keep-on-fail.ts` 自定义 reporter 先例）。Rust 侧 e2e ~68 case、acceptance 10 journey 全部平铺在单一集成测试文件顶层。本机实测：stable cargo 1.98.1、无 nightly/rustup/nextest——libtest JSON 输出与 `--report-time` 均不可用；Playwright 自带 JSON reporter 可直接用。`.artifacts/` 当前 untracked。动机见 proposal.md，行为契约见 specs/testsuite-report。

## Goals / Non-Goals

**Goals:**
- 一次运行 → 一份树形 HTML + 终端树，case 粒度到「模块分组 / describe 分组 + 用例标题 + 结果 + 耗时 + 失败摘要」。
- 采集通道对环境宽容：有 nextest 用 nextest，没有降级 cargo；webui 用 Playwright JSON。任一形态都能出报告。

**Non-Goals:**
- 不做报告历史/对比（固定路径覆盖）；不动退出码语义；不覆盖 real-agents；不改 harness（libtest 保留）。

## Decisions

### D1 树来源 = Rust 模块结构（测试文件重组）
把 `testsuite_e2e_test.rs` / `testsuite_acceptance_test.rs` 顶层测试函数搬进文件内 `mod` 树（如 `mod channel_and_supervision { … }`、`mod session_lifecycle { … }`），cargo/nextest 输出的用例名天然携带模块路径（`channel_and_supervision::secret_rotation…`），解析器零额外映射。
**否决备选**：命名前缀推导（受制于既有命名、树质量差）；集中注册表（与代码双份、必漂移）；外部 crate 重组为多文件模块（git 历史冲突面更大——最终取文件内 `mod`，单文件内搬移，diff 可审）。
**代价与约束**：函数名、断言、support 引用全部保持原样，只动归属；`#[ignore]` 属性随函数走；搬移后跑全量套件回归验证。

### D2 采集 = 外层解析，双通道自适应
- **nextest 通道**（优先）：`cargo nextest run --test … --message-format junit`（结构化 per-case status + 耗时）。tasks.py 检测 `cargo nextest --version`。
- **cargo 降级通道**：正则解析 libtest 控制台行（`test <name> ... ok/FAILED/ignored`）；失败详情按 `failures:` 块内 case 名归档归属（libtest 保证并行下失败输出按名隔离）；套件总耗时由 tasks.py 计时兜底。
- **webui 通道**：Playwright `--reporter=json` 输出经管道喂自定义 reporter，produce per-case 树（describe 链 + 标题 + duration + error message）；与既有 `keep-on-fail.ts` 并列配置，不替换。
- 解析器与报告器统一为 Python（`scripts/testsuite_report.py`），tasks.py 只做「跑 + 喂数据」，零新依赖（标准库 xml/etree 解析 JUnit、json、re）。

### D3 输出 = HTML 单文件覆盖 + 终端树
- HTML：自包含（内联 CSS，无 JS 依赖），固定写 `.artifacts/verify/report-<suite>.html`（e2e / acceptance / webui 各一份），每次运行覆盖。`.artifacts/` 加入 gitignore。
- 终端：报告器同时打紧凑缩进树（✅/❌/⏭ + 标题 + 秒级耗时），失败行附沙箱路径。invoke 输出已有构建日志，树打在套件结果之后。
- **否决备选**：时间戳留历史（目录膨胀、无清理约定，被否）；最新+N 滚动（实现重，先不做，固定路径已满足即看即用）。

### D4 耗时与失败详情
- nextest/JUnit：per-case `time` 原生带；Playwright JSON：原生带。
- cargo 降级：per-case 耗时拿不到 → case 行耗时缺省、总耗时兜底（如实标注「cargo 模式无 per-case 耗时」）。
- 失败详情：JUnit `failure` 节点文本 / cargo failures 块文本，截断至 ~2000 字符入报告；沙箱路径沿用既有约定（失败保留现场打印的路径，从输出捕获）。

### D5 退出码中性
报告生成放在套件命令之后、退出判定之外：先按测试结果定 exit code，再尝试生成报告；报告失败仅打 warning 不改码。`--case` 单跑时报告只含被过滤的运行（数据源即本次真实输出，天然满足）。

## Risks / Trade-offs

- [模块重组动 ~80 个函数归属] → 纯搬移不改内容；分两个 commit（e2e / acceptance）便于审；重组后全量复跑两套件回归。
- [cargo 控制台输出格式随 cargo 版本漂移] → 解析器对未知行宽容（只认 `test … ok/FAILED/ignored` 行锚），解析失败时报告如实报「0 case 解析」而非伪造；nextest 存在时该通道非主路径。
- [HTML 覆盖写丢历史] → 接受（用户已确认固定路径覆盖为默认）；终端树 + 套件日志仍可回溯。
- [报告器与 keep-on-fail reporter 并存] → Playwright reporter 数组并列，互不干扰；JSON reporter 的输出由 tasks.py 捕获，不进操作员终端。
- [webui 是六次 Playwright 运行而非一次] → 实施期发现：主配置用 `testIgnore` 把 auth / auth-setup / users-admin / deployment / detached / dead-core 等 spec 排除，交给另外五个 config 跑，`testsuite_webui` 全量时是 5 个 config 的 `&&` 链（`--case` 时是单 config）。**取「六配置各写分片 + 生成器归并」**：`collect-json.ts` 挂进全部六个 config，分片按 config 名区分（`webui-results.json` / `webui-results-<stem>.json`），tasks.py 跑前清分片（避免上一次运行的残留混进本次报告，尤其链中途中止时）、跑后一次性 `--shards` 归并成一棵树。**否决备选**：在 tasks.py 里按配置逐个采集——`&&` 链的每条命令要各自注入不同的输出路径并逐次解析，采集逻辑与 Playwright 调用顺序耦合，且链中途失败时前面几次的采集结果要额外暂存，比"配置自报分片"更脆；任一次运行单独重跑时（`--case auth`）分片机制天然只覆盖它自己的切片。

## Migration Plan

1. 先落报告器 + webui 通道（不动 Rust 侧，立即有第三套件报告）。
2. Rust 侧重组模块（两 commit），跑全量回归确认用例名映射不断链（COVERAGE.md 引用抽查）。
3. 接入 nextest 优先 / cargo 降级双通道。
4. 回滚策略：报告器为纯附加产物，任一环节异常可退回原入口命令（报告失败不影响测试运行本身）。

## Open Questions

（无——耗时路线（nextest 优先 + cargo 降级）与落盘策略（固定路径覆盖）已由拷问收口或用户默认确认。）
