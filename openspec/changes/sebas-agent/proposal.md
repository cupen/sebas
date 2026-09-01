# sebas-agent

## Why

sebas 的 Agent 层完全依赖外部 Claude Code 子进程（`acp-claude`），循环、工具契约、上下文都不可控。蓝图（`add-agent-core-architecture`，f4b51c8）已定稿 Phase 1 目标：in-process 循环 + 六件套工具 + gateway 通道。本 change 立项实现 **Phase 1a：headless 内核并证明它**——webui/飞书接线等 `add-core-session-channel` 落地后作为 1b 跟进，避免与在实现的分支冲突。

## What Changes

- **新增 crate `sebas-agent`**（命名沿 `feat/sebas-crate-prefix` 的 sebas-* 惯例；蓝图中的暂定名 "agent-core" 即此内核，作为能力域名保留）：四模块——`llm/`（Anthropic Messages 流式客户端 + `input_json_delta` 工具参数累积；端点可配置——默认直连 provider，gateway 为可选路径）、`loop/`（turn 状态机：AwaitingModel ⇄ ExecutingTools、取消安全、三重预算）、`tools/`（统一 Tool trait + bash / read / write / edit / glob / grep 六实现）、`session/`（SessionManager：多会话、事件广播、`AcpEvent` 零新增变体）。
- **系统提示词装配**：sebas-agent 身份 + 项目 AGENTS.md / CLAUDE.md 注入（checklist C6）。
- **新增 example `agent-dev`**：headless 冒烟入口（`cargo run --example agent-dev`），事件流打印到 stderr——Phase 1a 唯一的人工验证面，不改 CLI 命令表。
- **FakeLlmClient 脚本化测试**：多步循环（≥5 工具调用）、取消打断、预算耗尽、工具错误自愈四类场景作为验收。
- 不接线任何 UI；`src/`、`router`、`feishu`、`webui`、`gateway`、`acp-claude` 零改动。

## Capabilities

### New Capabilities

- `agent-core`: sebas 原生 coding agent 内核（产品名 sebas-agent）——会话生命周期、in-process turn 循环、六件套工具契约、LLM 通道（直连 provider 或可选 gateway）、`AcpEvent` 事件对齐、取消与预算语义。

### Modified Capabilities

（无——1a 不触及任何现有能力的行为面；example 不改 CLI 命令表，`cli-service` 无 delta。）

## Non-goals

- 不接 webui / 飞书 / CLI 交互面（Phase 1b，等 `add-core-session-channel` 的 `SessionBackend` 缝收敛）；不动 `router` / `acp-claude`。
- 不做权限规则引擎与沙箱（Phase 2）、compaction 与任务清单（Phase 3）、子代理 / MCP / 技能（Phase 4）。
- 不做会话持久化（OQ1 延后；1a 会话为内存态，进程退出即失）。
- 不做协议转换——gateway 纯透传不变，agent-core 是它的新 HTTP 客户端。

## Impact

- 新 crate `sebas-agent`（workspace member）+ 新依赖：tokio（process/fs）、reqwest（stream/json）、eventsource-stream、async-trait、regex、globset、walkdir、futures-util。
- `sebas-agent/tests/` 新增集成测试（FakeLlmClient 在 crate 内，无需起真 gateway）。
- 对现有 crates：零改动。
- 部署不要求 gateway：直连 provider 即可运行（用户决策 2026-09-01）；gateway 降级为可选的路由/计量层。
