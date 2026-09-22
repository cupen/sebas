# Design: fake-provider-upstream

## Context

侦查结论（代码证据）：

- router 对自定义 provider 零特殊处理：`[provider.fake]` 配 `base_url_anthropic` + 哑 `api_key` 即被 `proxy.rs` 全链路转发（`filtered_request_headers` 剥下游 key 注入上游 key、SSE 逐 chunk 透传 + `UsageFinalizer` 结算、上游 4xx/5xx 原样回传）——fake 上游是纯增量插入点。
- 内置 debug `test` provider（`sebas-router/src/test_provider.rs`）在 router **内部**自答，不经过拨号路径；已有 Anthropic/OpenAI 两族 JSON 与 SSE 形状构造函数可直接复用。
- 主 crate `Cargo.toml` 已依赖 `sebas-router = { path = "sebas-router" }`（`sebas router` 动词即此模式）。
- `tests/bin/fake-claude.rs` 是 ACP stdio 桩，从不拨 HTTP——agent 流程至今未压过透传面；其 `--journal` NDJSON 留痕模式是套件断言的既有先例。
- 限流是 per-key 令牌桶（`router-auth-rate-limit` spec：连发第 6 个请求 429 `rate_limit_error`）——`over_capacity_returns_429` 的 flake 根因是 provider 名命中 preset 拨了真实 api.anthropic.com，网络延迟让桶在请求间隙回填；fake 秒回即确定性。
- e2e 套件经 `CARGO_BIN_EXE_sebas` 可直接 spawn 主二进制动词（sigterm 用例已解析 `target/debug` 先例）。

## Goals / Non-Goals

**Goals:**

- fake 上游作为产品动词落地，测试 harness（cargo / tasks.py / 人工）都能起。
- 内置规则零剧本驱动标准 agent 工具环；scenario 文件覆盖多轮脚本、错误注入、确定性 usage。
- e2e 三条 journey：透传全链路、真实 claude-code agent-loop（缺席跳过）、确定性限流/用量。

**Non-Goals:**

- OpenAI 协议面、Playwright 接线、产品行为变更（见 proposal Non-goals）。
- 响应录制回放真实上游流量。
- 高保真模型模拟（token 概率、缓存语义、多候选）——确定性优先于逼真。

## Decisions

### D1. 模块落点：`sebas-router` crate 新模块，主二进制薄壳动词

实现放 `sebas-router/src/fake_provider.rs`（与 `test_provider.rs` 同居），把 test_provider 的 Anthropic 形状构造下沉为两处共用；`sebas fake-provider` 动词在主 crate 薄壳调用 `sebas_router::fake_provider::run()`，与 `sebas router` 同模式。

- 备选：主 crate 自实现 → 重复 Anthropic 线协议知识，拒绝。
- 备选：`tests/bin` 测试二进制（仿 fake-claude）→ 不随 release 分发、无法做开箱本地演示，且跨 crate 复用形状更别扭，拒绝。
- 备选：testsupport 进程内 server → tasks.py / 浏览器 journey 用不上，拒绝。

### D2. 应答引擎：规则枚举 + scenario 按序消费，耗尽回落

请求处理统一走 `ResponseRule`：scenario 条目优先按序弹出，耗尽或未配置回落内置确定性规则（tools 且无 tool_result → 首个 tool 的 tool_use；有 tool_result → 终文本；无 tools → 纯文本）。scenario 为 JSON：预置条目（文本 / 工具块 / 错误注入 status + retry-after）+ 可选 usage 覆盖。无运行时控制面。

- 备选：纯 scenario 驱动 → 每个用例都要写剧本，标准工具环本可零成本，拒绝。
- 备选：HTTP 控制面运行时注入 → 部件与复杂度最高，当前无用例需要，拒绝。
- 内置规则的 tool_use `input` 取 tool 名 + 最小合法对象（如 `{"command":"echo ok"}` 形态由 scenario 兜底）——真实 claude-code 对 input 校验若过严，scenario 补足，内置规则保底可跑。

### D3. 留痕与就绪信号：journal NDJSON + stdout ready 行

`--journal <file>` 逐行追加收到的请求（method、path、headers、body），仿 fake-claude `--journal` 先例——透传断言（key 注入、下游 key 不泄漏、hop-by-hop 剥离）全部离线读 journal 完成，fake 自身不做断言。`--listen 127.0.0.1:0` 随机端口，bind 成功后 stdout 打 `fake-provider listening addr=127.0.0.1:<port>`（router 同款），harness 解析该行拿端口。

- journal 明文记录 header 的安全性：fake 只会收到 provider config 里的哑 key（真实上游 key 配在真实 provider 上，不会指向 fake），journal 属测试工件，文档标注不得指向生产。

### D4. e2e 接线：testsupport spawn helper + 沙箱 config 增段

`tests/support` 新增 `spawn_fake_provider(scenario: Option<&Path>) -> FakeUpstream`（CARGO_BIN_EXE_sebas + 动词 + 随机端口，解析 ready 行，落 journal 路径；拆装沿用 kill_tree 进程树收割）；沙箱 config 模板加 `[provider.fake]`（哑 key + `base_url_anthropic` 指向 fake + models 列表）与路由所需段。claude-code 探测：env `SEBAS_TEST_CLAUDE_BIN` 优先，否则 PATH 查找；缺席 → `eprintln!` 原因 + return（skip-not-fail 既有约定）。

- 备选：固定端口 → 并发跑套件易撞，拒绝（沙箱端口探测同款教训）。
- 限流 journey 复用既有 `over_capacity_returns_429` 场景形状但改拨 fake provider；原用例的 preset 命中问题以 issue sebas-vm9p 跟踪是否就地改写。

### D5. 协议面收敛 Anthropic，代码留枚举分派

只实现 `/v1/messages`（流式 + 非流式）；内部按 `WireProtocol` 分派留出 OpenAI 面扩展点但不实现——后续 change 加面不动骨架。

## Risks / Trade-offs

- [真实 claude-code 升级改变 ACP/env 约定 → journey 烂] → 断言保持宽松（Done + 终文本），失败留日志可诊断；`SEBAS_TEST_CLAUDE_BIN` 可钉版本，缺席即跳过。
- [stdout ready 行解析竞态] → bind 后同步 flush 单行，解析失败按用例超时失败（有界），与 router ready 行同款机制。
- [内置规则 tool_use input 不满足真实 agent 校验] → scenario 兜底；journey 断言不依赖具体 input 值。
- [fake 与 test_provider 形状下沉触碰既有单测] → 下沉只搬函数不改行为，router 侧单测随迁。
- [scenario JSON schema 演进] → 未知字段宽松忽略（serde default），加字段不破坏旧剧本。

## Migration Plan

纯增量：新动词 + 新测试用例，无配置/数据迁移。回滚 = revert 对应提交；沙箱 config 里多出的 `[provider.fake]` 段对旧二进制无害（未知 provider 段不解析即忽略——与既有宽松解析一致）。

## Open Questions

（无——scenario 条目字段名等实现细节按 spec 语义在 tasks 内定，不影响规格与拆解。）
