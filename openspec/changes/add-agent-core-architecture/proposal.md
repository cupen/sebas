# add-agent-core-architecture

## Why

sebas 目前只是"桥"而不是 agent：`acp-claude` 拉起外部 Claude Code 子进程，sebas 自身没有 agent 内核，无法脱离 Claude Code 独立执行编码任务。要回答"如何实现一个专业的 coding agent"（像 Claude Code、Codex、DeepSeek-Harness 那样），需要先有一份系统的架构调研与设计蓝图作为决策依据，而不是直接动手写代码。同时 webui 控制台（`redesign-webui-console`、`add-project-workbench`）已具备承载 agent 交互的地基，原生 agent 以 webui 为第一入口是顺理成章的下一步。

## What Changes

- **新增架构调研文档**：拆解 Claude Code、Codex、DeepSeek-Harness（及代表性开源 coding agent）的核心机制——agent loop 结构、工具系统与工具契约、上下文管理、权限模型、流式交互——提炼可迁移到 sebas 的设计模式与反模式。
- **新增 agent-core 架构设计文档**：定义 sebas 自研原生 coding agent 内核的目标架构——in-process agent loop + 基础工具集（bash / read / write / edit / glob / grep），LLM 通道复用 `gateway` 双协议网关，交互入口 webui 优先；并给出与 `acp-claude` 桥接并存的模块边界与演进路线（核心循环 → 权限沙箱 → 上下文管理 → 子 agent / MCP）。
- **纯文档交付**：不改任何代码，不新增依赖。

## Capabilities

### New Capabilities

（无——本 change 为纯文档交付，已在 `.openspec.yaml` 声明 `skip_specs: true`。原生 agent 的行为规格留给未来的实现 change，从设计文档中派生。）

### Modified Capabilities

（无）

## Non-goals

- 不实现任何代码：不新增 crate，不改 `webui` / `router` / `gateway` / `acp-claude`。
- 不在本期给出权限沙箱、上下文 compaction、子 agent、技能 / MCP 的完整设计——仅在路线图中作为分期提及。
- 不改飞书侧任何行为；飞书与 CLI 接入 agent-core 留待后续 change。
- 不做模型选型评测、benchmark 或成本对比。

## Impact

- 新增 `docs/superpowers/specs/2026-08-29-agent-core-architecture-design.md`（调研 + 架构设计，沿用现有设计文档命名惯例）。
- 不触及任何 Rust 代码、API、依赖与配置；对现有 spec 无影响。
