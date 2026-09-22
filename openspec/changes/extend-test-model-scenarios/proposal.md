# Proposal: extend-test-model-scenarios

## Why

router 的内置 debug `test` provider 今天只会**一句台词**：固定文本 + 回显最后一条用户消息（`test_provider.rs`，三种协议 × 流式/非流式同形）。这限制了 e2e 验收的纵深：thinking 块如何进转录与卡片、tool_use 如何驱动权限流、多块混排如何在流式事件里保真——这些 Agent 平台最核心的呈现与交互路径，今天要么靠真实凭据拨真上游（烧 token、网络抖动），要么靠 `sebas fake-provider` 独立进程（能测拨号路径，但要多管一个进程）。而 debug `test` provider 是零进程、零凭据、零网络的：它已经在 router 里，`test/<anything>` 的路由也已存在，只差它会演的剧本。

## What Changes

- **场景走 model 名**：`test` 保留现状（echo，逐字不变——AGENTS.md 食谱与既有 e2e 对 `msg_test_debug` 的断言是已文档化契约）。场景集由核心五簇的 LLM 形状相关 requirement **反推**（覆盖矩阵见 design D9），共九个：`test/text`、`test/long`（长文，流式背压/增量同步/折叠）、`test/thinking`、`test/tool-use`（首 tool 环，与 fake-provider 同规范）、`test/tools-parallel`（一回合多 tool_use，驱动并行权限请求）、`test/full`、`test/empty`（零输出 → 通知路径）、`test/error`（LLM 5xx → 失败呈现路径）。由请求 body 的 `model` 字段携带（路由 `test/<anything> → test` 已存在，零新增配置）。
- **块类型升级**：`AnthropicMessage`（test provider 与 fake-provider 共用的线协议事实源）从只构造 text 块扩展到 **thinking / tool_use / text 混排**，非流式 JSON 与流式 SSE 同一事实源。
- **内置确定性 agent-loop 规则**：`tool-use` / `full` 场景与 `fake-provider-upstream` 同规范（tools 非空且无 tool_result → tool_use；有 tool_result → 终文本）；`tools-parallel` 为 test 模型**特有**（一回合发出请求中全部 tools 的 tool_use，确定性 input 各异）——并行权限请求的驱动器，fake-provider 不动。
- **流式保真**：每块类型正确的事件序列（`thinking_delta` / `input_json_delta` / `text_delta`），确定性分块，`message_delta` stop_reason 随场景；`test/error` 以 Anthropic 错误体应答（5xx api_error）。
- **确定性 usage**：每场景固定非零 token 数（bare `test` 保持全零），usage 结算链路有真实形状可断言。
- **e2e journeys**（`testsuite-process-e2e`）：native kernel 工具环经权限流到 Done、并行权限请求、thinking 呈现、混排、**零输出通知**、**长文流式（背压/增量）**、**流式中取消**、**模型切换下一回合生效**、**LLM 错误呈现 + 失败后工作台仍可操作**。真实 claude-code 场景 journey 为可选（缺席诚实跳过）。
- **旅程证明标准**：journey 一律经工作台用户面驱动（HTTP API + WS/SSE）、合计覆盖操作员全路径、代表性 journey 断言重复执行一致性、echo 回显显式证明对话连续性——使全绿等价于「工作台正常、稳定、可用」的证据（design D10）。
- **验收载体定向**（`testsuite-acceptance`）：今后**工作台行为验收的 LLM 载体 = 内置 test 模型**；fake 上游 / 真实上游 journey 保留但只负责拨号透传路径（router 透传验收），不承担工作台行为验收；载体切换**不得降低覆盖口径**——核心五簇账本保持 100%，证据可换、分母不丢。

## Capabilities

### New Capabilities

（无。）

### Modified Capabilities

- `router-core`: 「Debug test provider」更新——保留 echo 契约，新增九场景模型、块类型混排、确定性 agent-loop 规则（含 test 特有的并行 tool_use）、错误应答、流式事件保真、确定性 usage、OpenAI 家族降级规则。
- `testsuite-process-e2e`: 新增「test 模型场景 journey」要求——工具环 + 权限流（含并行）、thinking 呈现、混排、零输出通知、长文流式、错误呈现的进程级覆盖。
- `testsuite-acceptance`: 新增「工作台验收载体 = 内置 test 模型」要求——涉及 agent 回合的验收旅程 SHALL 以 test 模型驱动；账本规则同步（载体切换不计减分母、不降口径）。

## Impact

- **改动**：`sebas-router/src/test_provider.rs`（场景解析 + 规则）、`sebas-router/src/anthropic_wire.rs`（块类型扩展，fake-provider 同受益）、`sebas-router/src/debug.rs`（路由已存在，基本不动）、`tests/testsuite_e2e_test.rs` 与 `tests/support/`（新 journeys）、`AGENTS.md`（沙箱食谱补场景表）。
- **不变**：bare `test` 的 echo 行为逐字不变；`fake-provider` 子命令与 `fake-provider-upstream` capability 不动（它测拨号路径，与本题互补）；`AnthropicMessage` 的既有 text 构造 API 签名不变（新增块构造入口）。
- **验收**：每场景的 JSON/SSE 单测（含与 fake-provider 规则的一致性）+ 新 journeys 全绿 + 既有 `testsuite-e2e` / `testsuite-acceptance` 全绿（bare `test` 断言不改而通过）。
- **排序说明**：usage 断言目标（`router-usage.jsonl` vs `usage.db`）取决于 `persist-router-usage` 的落地顺序——本 change 的 journeys 通过 router 用量记录的**既有断言辅助**取数，不直接绑定文件形态（软依赖，见 design）。

## Non-goals

- **不动 `sebas fake-provider` 独立子命令**——它服务「可拨的真上游」（透传链路验收），本 change 服务「router 内自答」（零进程验收）；两者互补，规则保持同规范但不合并。
- **不改 debug 模式的开关与注入机制**（`--debug` / `[router] debug`），不加新 CLI 或配置键。
- **不做场景文件编排**（多轮剧本、错误注入、延迟）——那是 fake-provider 的 scenario 能力；本 change 的场景集是固定的、编译期的。需要可编排剧本时用 fake-provider。
- **不让 OpenAI 协议族完整对齐**——thinking 在 chat 协议无对应物，降级为纯文本（Responses 档同样只承诺回显级形状）；Anthropic 是 agent 的实际拨号面，一等公民只有它。
- **不改变下游鉴权语义**（debug 模式跳过下游 auth 是既有行为）。
