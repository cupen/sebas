# Proposal: fake-provider-upstream

## Why

进程级 e2e 至今没有可拨的 provider 上游：agent 相关验收要么依赖 fake-claude ACP 桩（完全不经过 router→上游的 HTTP 透传面），要么拨真实 api.anthropic.com——烧 token 且网络间歇抖动（`over_capacity_returns_429` 已实证为该类 flake）。router 透传引擎（header 过滤、上游 key 注入、SSE 透传、usage 结算）今天只有 proxy 单测覆盖，进程级链路零验收。

## What Changes

- 新增 `sebas fake-provider` 子命令：本地 Anthropic `/v1/messages` 线协议上游（流式 + 非流式），监听 127.0.0.1；router 以自定义 provider（`base_url_anthropic` 指向它）零改动接入。
- 内置确定性 agent-loop 规则：请求带 `tools` 且尚无 `tool_result` → 返回首个 tool 的 `tool_use` 块；出现 `tool_result` → 返回终文本——零剧本即可让真实 agent 二进制跑完标准工具环，零 token。
- scenario 文件（可选）：编排多轮响应脚本、错误注入（429/500 + retry-after）、响应延迟、确定性 usage 数值。
- e2e 套件接线三条 journey：provider 透传全链路（header 过滤 / key 注入 / SSE 透传 / usage 落 router-usage.jsonl）、agent-loop journey（真实 claude-code + fake 上游，二进制缺席诚实跳过）、确定性限流/用量用例（fake 秒回消除网络抖动，不再依赖真实上游）。

## Capabilities

### New Capabilities

- `fake-provider-upstream`: fake 上游服务本身——CLI 动词与监听、Anthropic /v1/messages 线协议应答（流式/非流式）、内置 agent-loop 确定性规则、scenario 文件编排、确定性 usage 与错误注入。

### Modified Capabilities

- `cli-service`: Subcommand tree 增加 `fake-provider` 动词。
- `testsuite-process-e2e`: 新增 provider 透传 journey、agent-loop journey 与确定性 quota/metrics 用例的覆盖需求。

## Impact

- 代码：主二进制新增 fake-provider 模块（下沉复用 sebas-router `test_provider` 的响应形状构造）；`tests/testsuite_e2e_test.rs` 与 `tests/support/` 新增用例与 spawn helper。
- router / core 产品代码零改动；tasks.py 不动（fake 上游由 cargo 测试进程 spawn）。

## Non-goals

- OpenAI chat / Responses 协议面（内置 test provider 已在 router 内覆盖 OpenAI 形状，fake 上游面留后续 change）。
- Playwright / testsuite-webui harness 接线（浏览器 journey 消费 fake 上游留后续）。
- router、core 等产品行为变更——本 change 只加测试面工具与套件用例。
- 录制回放真实上游流量（`record`/`replay` 是 ACP stdio 层既有能力，不在此扩展）。
