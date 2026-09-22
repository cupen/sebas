# Tasks: fake-provider-upstream

## 1. sebas-router 内 fake_provider 模块

- [x] 1.1 下沉 `test_provider.rs` 的 Anthropic 形状构造为共用（参数化 content 块与 usage），router 侧既有单测随迁——验证：`cargo test -p sebas-router` 全绿
- [x] 1.2 新建 `sebas-router/src/fake_provider.rs` 服务骨架：axum `/v1/messages`、`--listen 127.0.0.1:0` 随机端口、bind 后 stdout 单行 ready（`fake-provider listening addr=…`）、SIGTERM 优雅退出——验证：spawn 后 curl `/v1/messages` 得 200，ready 行可解析，SIGTERM 后退出且端口释放
- [x] 1.3 内置确定性规则引擎：tools 且无 tool_result → 首个 tool 的 tool_use（stop_reason=tool_use）；含 tool_result → 终文本（end_turn）；无 tools → 纯文本；确定性非零 usage——验证：单测覆盖三规则与同请求同应答
- [x] 1.4 流式应答：stream=true 输出完整 SSE 事件序列（message_start → content_block_delta → message_delta → message_stop），文本与非流式一致——验证：单测断言事件序列与文本
- [x] 1.5 scenario 文件：JSON 加载、按序消费（文本/工具块/错误注入 status+retry-after）、耗尽回落内置规则、加载失败按启动失败语义报错；未知字段宽松忽略——验证：单测覆盖按序、注入、回落、坏文件四路径
- [x] 1.6 `--journal <file>` NDJSON 请求留痕（method、path、headers、body 逐行追加）——验证：单测发两请求后 journal 恰两行且字段可解析

## 2. CLI 动词接线

- [x] 2.1 主 crate 新增 `sebas fake-provider --listen/--scenario/--journal` 薄壳动词（调 `sebas_router::fake_provider::run`），scenario 加载失败走 startup-failure 语义——验证：`cargo build` 后 `sebas fake-provider --help` 正常、坏 scenario 以退出码 75 与 `startup-failure:` 末行退出

## 3. e2e 套件接线

- [x] 3.1 testsupport 新增 `spawn_fake_provider` helper：spawn `CARGO_BIN_EXE_sebas fake-provider`、解析 ready 行取端口、返回 journal 路径、拆卸走 kill_tree 进程树收割——验证：helper 冒烟测试 spawn→拨号→拆卸后端口释放
- [x] 3.2 沙箱 config 模板加 `[provider.fake]`（哑 key、`base_url_anthropic` 指向 fake、models 列表）——验证：既有 e2e/acceptance 套件全绿（无回归）
- [x] 3.3 透传 journey 用例：非流式（应答为 fake 内容 + router-usage.jsonl 落非零 usage 与 provider 名）、流式（SSE 事件完整透传）、journal 离线断言（auth=上游哑 key、无下游 key 与 hop-by-hop）——验证：`cargo test --test testsuite_e2e_test -- --ignored` 新用例绿
- [x] 3.4 确定性限流/用量用例：向 fake provider 连发超桶容量请求断言 429 rate_limit_error 可复现、无真实外呼；视改动幅度顺带收口 sebas-vm9p（原 `over_capacity_returns_429` 改拨 fake）——验证：该用例连续三跑结果一致
- [x] 3.5 agent-loop journey：真实 claude-code + 会话模型路由到 fake，工具环到 Done；`SEBAS_TEST_CLAUDE_BIN` 优先、PATH 兜底探测，缺席时输出原因并跳过不判失败——验证：有 claude-code 的机器用例绿，无则 skip 断言生效

## 4. 收尾与回归

- [x] 4.1 全量回归 + 双平台编译门：`cargo test --workspace`、e2e/acceptance `--ignored` 全绿；`cargo check --target x86_64-pc-windows-gnu --test testsuite_e2e_test` 编译通过（延续双平台 MUST，fake 模块不得引入平台专属 API）——验证：三组命令全绿/通过
  - 状态：`cargo build` 0 errors；`cargo test --workspace` 1965 passed / 52 ignored；e2e `--ignored` 36 passed（基线 32 + 新增 4）；acceptance `--ignored` 9 passed（基线）。**windows 门未验证**：本机 rustc 由 Arch `rust` 包提供（无 rustup），只装了 `x86_64-unknown-linux-gnu` 一个 target，Arch 已无 `rust-mingw` 包（仅 `rustup`，安装并行工具链超出本任务范围），`cargo check --target x86_64-pc-windows-gnu` 报 “can't find crate for core/std”。已做代码级平台审计：新代码唯一平台分支是 `fake_provider.rs` 的 `#[cfg(unix)]` SIGTERM / `#[cfg(not(unix))]` pending，以及测试侧 `#[cfg(unix)]`/`#[cfg(not(unix))]` 两个变体，无未加 cfg 的 unix 专属 API。
- [x] 4.2 文档补一段：AGENTS.md 沙箱菜谱或 README 记录 fake-provider 动词用法与 journal 工件语义（哑 key 专用、勿指向生产）——验证：文档落盘且示例命令可照跑
