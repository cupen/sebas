# Design: add-claude-model-slots

## Context

现状三套机制互不相干：(1) `AgentRow.models`（`Vec<String>`，agents 表 JSON 列）只是 claude 会话模型菜单的别名词汇表，Settings 表单未暴露；(2) spawn 时的模型 env 由 provider 模型清单经 `map_to_env`（`sebas-router/src/models.rs`）强→弱自动排位派生，外加 `--model <default_model>` argv（`src/spawn_env.rs`）；(3) new-session 对话框的模型下拉吃 provider 目录（`model-catalog.ts`），与所选 agent 无关。claude 官方档位 2026 年已是 fable/opus/sonnet/haiku 四档，env 键家族含 `ANTHROPIC_DEFAULT_FABLE_MODEL`，现实现缺 fable 键。

## Goals / Non-Goals

**Goals:**

- claude agent 的模型配置收敛为单一显式来源：档位映射（档位 → 目录模型 id + 默认标记），spawn 时整体接管模型 env。
- Settings→Agents 一屏统一目录（一行一 agent + 安装状态 + 齿轮定制），new-session 按 agent 联动档位下拉。
- 移除 provider 派生的模型钉扎双通道（自动排位 env + `--model` argv），模型 env 零 provider 模式特判。

**Non-Goals:**

- 不动 provider 模式系统的凭据半边（`ANTHROPIC_BASE_URL`/`AUTH_TOKEN` 翻译、探测面、provider CRUD）。
- 不做 per-agent 通用 env kv；不为 opencode/pi/custom 增加模型配置。
- 不改 ACP `session/set_config_option` 切换链路（复用现有 SetModel/with_models 机制）。

## Decisions

### D1: models 字段演进为结构化映射，存储面复用现有列

档位是封闭枚举（强弱序固定）：

```rust
// sebas-models / sebas-domain 层级归属：只被 models+config 两方需要，放 sebas-models
pub enum ModelSlot { Fable, Opus, Sonnet, Haiku }   // 序即强弱

pub struct ModelSlotMapping {
    pub slots: BTreeMap<ModelSlot, String>, // 启用档位 → 目录模型 id
    pub default_slot: ModelSlot,            // 校验保证 ∈ slots 键集
}
```

- **存储**：agents 表 `models` 列（JSON 文本）形状从 `Vec<String>` 换成上述结构，列名与 DDL 不变，无 schema 迁移。读取时对旧 `Vec<String>` 形态宽容反序列化为 `None`（视为未配置，warn 一条），写入只写新形状。
- **wire**：`AgentLaunchPayload.models` 从 `Option<Vec<String>>` 改为 `Option<ModelSlotMapping>`（webui HTTP 面，前后端同仓同发布，不走 sebas-ipc golden fixture）。
- **config**：`[acp.agents.<id>]` 的 `models` 键接受 TOML table——键为档位名（`fable`/`opus`/`sonnet`/`haiku`）+ `default` 键；旧裸数组形态解析期报错（serde 类型不匹配自然失败，报错文案点名 `models` 键与期望形态）。
- 否决备选：新开一列（无必要——列本来是自由 JSON）；`Vec<(String,String)>` 平铺（丢掉默认标记的结构性）。

### D2: env 派生收敛到 agent 装配点，spawn_env 只做翻译

`resolve_spawn_overrides` 现在只认 provider state；改为同时接收 spawn 所用 agent 的 `Option<ModelSlotMapping>`（`session_boot.rs` 装配点从 `resolve_agent_config` 拿到后传入）：

- 有映射 → `slot_mapping_env(mapping)` 产出 `ANTHROPIC_MODEL`（默认档值）、各启用档 `ANTHROPIC_DEFAULT_{FABLE,OPUS,SONNET,HAIKU}_MODEL`、`CLAUDE_CODE_SUBAGENT_MODEL`（最弱启用档值）；未启用档不出键。
- 无映射 → 模型键一个都不出（provider 解析只产出凭据键）。
- `--model` argv 产出整体删除：`spawn_env.rs` 不再追加 `--model`，`map_to_env`/`ClaudeModelEnv` 从 `sebas-router` 删除（`spawn_env` 是唯一消费者，删前 grep 确认）。
- 模式特判消亡：不再有「Router 不盖」分支——映射注入与 provider 模式正交。
- 否决备选：逐键合并 provider 自动排位（共识已否——来源混杂难排查）；保留 `--model` 通道（与「唯一来源是映射」冲突，且 flag/env 优先级在 claude 侧不透明）。

### D3: 会话档位词汇与内置回退表

`builtin_claude_models()` 升级为 `["default","fable","opus","sonnet","haiku"]`。`resolved_models` 语义变为：有映射 → `["default"] + 启用档（按强弱序）`；无映射 → 内置表。`ClaudeDriver::with_models` 注入链路不变，composer chip 与创建时 `SetModel` 自动吃到新词汇——选 `default` 经 `ANTHROPIC_MODEL` 解析，选档位经对应 `ANTHROPIC_DEFAULT_*` 解析。

### D4: Settings 统一目录 = 现有两区数据源的并集渲染

`renderAgents` 重构为单一行模型：行数据 = `/api/agents`（native + store 行，含 reachability）∪ 安装配方条目（现有 recipes 数据源）。行内：安装状态徽标（`内置`/`已安装`+可达性/`未安装`）、未装行「安装」按钮（沿用 `POST /api/agents/install`）、已装行齿轮（打开现有编辑对话框）与删除。档位映射段只在 claude 形态分支渲染：四档位行（启用 checkbox + 目录模型选择器 + default radio），目录来自 `loadModelCatalog`（provider 模型 + router 别名同权平铺）。编辑对话框预填存量映射。

### D5: new-session 联动是前端分支，不动后端创建协议

`new-session-dialog.ts` 在 agent 选择变化时判 driver：claude → 模型下拉切为该 agent 档位列表（`default` + 启用档，无映射用内置表），预选 = agent 默认档，**跳过**「上次 (provider,model)」记忆规则；非 claude → 现有 provider 目录 + 三级预选原样。确认载荷不变（`model` 字段传档位别名），后端 `session_boot` 的 spawn 后 `SetModel` 链路原样复用。

### D6: mutation 校验收紧在 state_store

agents 域 mutation：`models` 仅 claude driver 可携带（其他 driver 携带即拒）；slot 名 ∈ 四档；启用集非空时 default 必须落在启用档上；空映射等价 None。config 种子导入走同一校验（`definition_of` glue 转换后过 mutation 面，或种子路径共享校验函数——实现取共享函数，避免两套口径）。

## Risks / Trade-offs

- [存量部署依赖自动排位] Direct + provider 模型清单的部署升级后模型 env 消失，claude 回落自身默认 → 在 proposal 声明 BREAKING 与迁移路径（配 agent 映射）；发布说明点名。
- [DB 存量 `Vec` 值] 旧形态宽容读为未配置 + warn——用户不会因升级丢配置之外的东西（旧值只是词汇表，从未驱动 env）。
- [fable 键与旧 claude CLI] 旧 CLI 不识别 `ANTHROPIC_DEFAULT_FABLE_MODEL` 时无害（多一个未知 env 键不致错）；档位词汇含 fable 在旧 CLI 上选了回落其内置解析，行为可解释。
- [目录为空] provider 未配时档位选择器无可选项——表单段显示引导文案（去 Settings→Models 配 provider），允许保存无映射状态（映射是可选的）。
- [wire 形状变更] `AgentLaunchPayload.models` 破坏性变更限定 webui HTTP 面，前后端同仓同发布；e2e/testsuite 同步改。

## Migration Plan

1. 实现按 tasks 顺序：Rust 域模型与校验 → spawn env 替换 → 前端 UI/联动 → 测试面替换。
2. 部署：升级二进制即可；config 旧 `models` 裸数组在解析期显式报错（改配置后启动），DB 旧值静默降级为未配置。
3. 回滚：git revert 单 change 即可——无不可逆 schema 操作（`models` 列形状由 JSON 内容区分）。

## Open Questions

（无——TOML 内联 vs 子表等纯排版细节实现时定）
