# Proposal: add-claude-model-slots

## Why

claude agent 的多模型使用目前依赖 provider 模型清单的强→弱自动排位生成 spawn env——哪个档位用哪个模型是算法猜的，不可控，且 fable 档键（`ANTHROPIC_DEFAULT_FABLE_MODEL`）缺失；Settings→Agents 表单完全没有模型配置入口，页面还是「已装列表 + 安装配方区」两区割裂布局。用户需要按 agent 显式配置模型档位，并在创建会话时直接选择。

## What Changes

- claude agent 新增**模型档位映射**配置：固定四档 `fable/opus/sonnet/haiku`，各档从模型目录（provider 模型 + router 别名同权）选一个模型 ID，单选标记默认档。
- spawn 时按映射注入 env：`ANTHROPIC_MODEL` = 默认档值、`ANTHROPIC_DEFAULT_{FABLE,OPUS,SONNET,HAIKU}_MODEL` = 各启用档值、`CLAUDE_CODE_SUBAGENT_MODEL` = 最弱启用档值；未启用档不注入；未配映射则不注入任何模型键。
- **BREAKING** 移除 provider 自动排位机制（`map_to_env` 强→弱模型 cover）：模型 env 唯一来源变为 agent 映射；凭据/接入键（`ANTHROPIC_BASE_URL` 等）不受影响。
- **BREAKING** 一并退役 provider 派生的 `--model` argv 通道（`default_selection.model` / provider `default_model` 不再钉扎 claude 子进程）——spawn 期模型钉扎只剩映射派生的 `ANTHROPIC_MODEL`；provider 的 `default_model` 保留为目录/探测元数据。
- agent 映射对 provider 模式（Direct/Router/Off）零特判，一致生效——router 就是普通 provider，目录里同权可选。
- **BREAKING** `models` 字段形状从 `Vec<String>` 演进为结构化映射（wire `AgentLaunchPayload` 与 config.toml `[acp.agents.<name>]` 同步）；config 旧裸列表形态解析期报错；存量 `Vec` 值视为未配置。
- Settings→Agents 重构为**统一目录**：一行一 agent（内置 native、已装、未装配方同列），行内显示安装状态；未装行给「安装」按钮，已装行给齿轮定制按钮；配方区作为独立区块消失。
- new-session 联动：选中 claude agent → 模型下拉切换为该 agent 档位列表（`default` + 启用档），初始值 = agent 默认档；其他 agent 维持现有 provider 目录下拉。
- claude 内置回退表 `builtin_claude_models()` 升级为 `default/fable/opus/sonnet/haiku`（对齐 2026 官方档位阵容）。

## Capabilities

### New Capabilities

（无——本变更全部落在既有能力的需求变更上）

### Modified Capabilities

- `acp-model-selection`: 会话模型列表语义改为 agent 档位映射（`default` + 启用档）；内置回退表升级含 fable；创建会话时 claude agent 的模型下拉与预选规则（agent 默认档开局，不记上次选择）。
- `claude-env-cover`: spawn 模型 env 的来源从 provider 自动排位改为 agent 档位映射；自动排位相关要求全部移除，能力语义整体改写。
- `agent-settings`: Settings→Agents 统一目录形态（一行一 agent + 安装状态 + 齿轮定制）；表单按形态渲染模型档位段（仅 claude）；模型档位映射作为 claude agent 设置的存储/校验/种子语义。
- `provider-management`: Off 模式解析不再构造 `--model` 旗标；「Model flag precedence」要求整体移除（provider 元数据不再钉扎 claude 模型）。

## Impact

- Rust：`sebas-models/src/agent.rs`（AgentRow/AgentDefinition models 形状）、`src/config.rs`（AcpClaudeConfig）、`src/agent_store.rs`（装配 glue）、`src/spawn_env.rs`（删自动排位 + 新映射注入）、`sebas-router/src/models.rs`（map_to_env 移除）、`sebas-acp/src/claude/driver.rs`（内置表 + fable）、`sebas-dispatch/src/state_store.rs`（mutation 校验）。
- WebUI：`sebas-webui/src/api.rs`（wire）、frontend `settings-modal.ts`（统一目录 UI）、`new-session-dialog.ts`（下拉联动）、`api/client.ts`（类型）。
- 兼容性：wire 与 config 的 models 形状变更为破坏性（旧形态明确拒绝而非静默容忍）。
- 移除的既有行为：provider 模型清单自动生成模型 env（原 claude-env-cover 核心要求），依赖该行为的部署需迁移到 agent 映射。

## Non-goals

- 不重建 provider 模式系统（Direct/Router/Off 的凭据注入半边不动）。
- 不做通用 per-agent 环境变量 kv 字段。
- 不代 opencode/pi 配置模型（opencode 模型由其自身 ACP configOptions 广告）。
- 不开放自定义档位名（claude CLI 只认内置别名，自定义名选了解析不了）。
