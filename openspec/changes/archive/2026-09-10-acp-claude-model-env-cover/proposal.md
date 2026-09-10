## Why

Claude Code 子进程的 provider 环境目前只由 sebas 注入 `ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`（Direct/Router 模式），模型与行为类变量（`ANTHROPIC_MODEL` / `ANTHROPIC_DEFAULT_*_MODEL` / `CLAUDE_CODE_*`）sebas 一律不设。于是 claude 会用自己环境/默认配置里的模型与行为参数——接入 DeepSeek 等 Anthropic 兼容上游时（例如 DeepSeek 的 `deepseek-v4-pro[1m]` 模型族），即使 base URL 与凭据指对了，模型也可能是 claude 默认的，行为不可控。需要让 sebas 在 spawn claude 时**全量覆盖**这一组环境变量，确保 claude code 用指定的端点、凭据、模型族与行为设置运行。

## What Changes

- spawn claude 子进程时，由 sebas **全量覆盖**以下模型/行为 env 变量（任何残留值都不得漏入子进程）：
  - 主模型：`ANTHROPIC_MODEL`
  - 各档位默认模型：`ANTHROPIC_DEFAULT_OPUS_MODEL` / `ANTHROPIC_DEFAULT_SONNET_MODEL` / `ANTHROPIC_DEFAULT_HAIKU_MODEL`
  - claude code 行为：`CLAUDE_CODE_SUBAGENT_MODEL` / `CLAUDE_CODE_EFFORT_LEVEL` / `CLAUDE_CODE_AUTO_COMPACT_WINDOW`
- 覆盖语义应用于 **Direct / Router / Off（含隐式 Direct）** 全部模式——当前 Off 模式完全不注入 provider env，无法保证模型；新模式 `Off` 也全量覆盖模型 env。
- 变量值来源：新增独立配置项（不是现有 provider 状态文件），允许单独启用与自定义；可启用 preset（如 DeepSeek 全套），模型族各档可整体沿用 preset 默认或覆盖。
- 覆盖方式：在 spawn 子进程的 env 上**显式覆盖**（注入的值优先于任何父进程残留），而非仅在缺失时补缺省——杜绝 claude 捡到 shell 或用户目录配置里的其它模型。

## Capabilities

### New Capabilities

- `claude-env-cover`: 定义 claude code 子进程 spawn 时全量覆盖的模型/行为环境变量集合（`ANTHROPIC_MODEL`、`ANTHROPIC_DEFAULT_OPUS/SONNET/HAIKU_MODEL`、`CLAUDE_CODE_SUBAGENT_MODEL`、`CLAUDE_CODE_EFFORT_LEVEL`、`CLAUDE_CODE_AUTO_COMPACT_WINDOW`）、来源配置结构、preset 支持与覆盖语义（Off/Direct/Router 全覆盖、注入优先于残留）。

### Modified Capabilities

- `acp-driver`: spawn 时注入的子进程环境从「按模式的 provider env（base_url/auth_token）」扩展为「provider env + 全量模型/行为 env 覆盖」；`extra_env` 合并语义从「补缺省/按模式注入」变为对覆盖集合内键的显式覆盖。
- `provider-management`: spawn-time env 翻译的输入从 `(ProviderMode, DefaultSelection)` 扩展为再加一个独立的模型 env 覆盖源；Off 模式的「子进程用自己的发现配置」语义改为「端点/凭据仍自发现，但模型/行为 env 一律由 sebas 覆盖」。
- `acp-model-selection`: 会话模型选择增加 spawn env 覆盖这一补充通道——`ANTHROPIC_MODEL` 覆盖与 `--model` / `session/set_config_option` 并存时的优先级需要明确（env 是启动时基座，运行时切换仍走既有通道）。

## Impact

- 代码：`sebas-acp/src/claude/agent_driver.rs`（`ClaudeCodeDriver` env 翻译表）、`src/spawn_env.rs`（覆盖源读取与合并）、`src/session_boot.rs`（spawn_overrides 调用点）、`sebas-acp/src/claude/manager.rs` 与 `driver.rs`（extra_env 传递，预计小改或不变）。
- 配置：新增一份模型 env 覆盖配置（独立于 provider 状态文件；preset 化，含 DeepSeek 等）。
- 测试：`src/spawn_env.rs` 单测、`sebas-acp` 单测、沙箱/e2e（用 `SEBAS_*` 覆盖注入变量验证子进程 env 生效）。
- 兼容性：对未配置该覆盖集的既有用户为**可选增量**——不配置时保持现状（不破坏 Off 模式语义）；一旦配置则对覆盖集键显式覆盖。配置项启用会影响所有 claude 会话的默认模型来源（spawn 级），属预期行为变更。

## Non-goals

- 不改动 Router 模式"agent 只见 Anthropic 协议面"的既有架构；模型 env 覆盖**不**向 router 协议层透传。
- 不新增对 OpenAI 协议 agent（`OPENAI_*` 环境族）或原生 ACP agent（非 claude）的覆盖——本 change 只覆盖 claude code 子进程。
- 不把运行时 `SetModel` 切换的通道替换为 env——env 只保证 spawn 时的确定性基线；会话中途切换仍走 `acp-model-selection` 既有通道。
- 不做模型名称自动探测/校验；拼写错误按"claude 端拒绝"处理。
