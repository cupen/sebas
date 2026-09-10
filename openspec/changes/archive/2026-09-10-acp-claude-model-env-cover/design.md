## Context

Spawn-time env 目前只有一条链：`session_boot.rs::spawn_overrides` → `spawn_env.rs::resolve_spawn_overrides` → `ClaudeCodeDriver::resolve_env`，产出 provider 端点 env（Direct/Router 模式注入 `ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN`；Off 返回空）→ 经 `SessionManager` 的 `extra_env` → SDK 的 `options.env` → `.envs()` 增量合入子进程（无 `env_clear`，SDK 全量继承 OS env）。模型走 `--model`（`resolve_args`）+ 会话建立后 `SetModel`（`session_boot.rs:154`）。

结果：claude 子进程的 `ANTHROPIC_MODEL` 等模型/行为变量若在 OS env 或用户 Claude 设置里存在，会一路漏进子进程；sebas 从不设置、也从不覆盖它们。模型与端点来源分叉，无法保证 claude 用指定模型。动机见 proposal.md - Why，行为契约见 specs/claude-env-cover + 三个 MODIFIED delta。

## Goals / Non-Goals

**Goals:**
- spawn claude 时把 7 个模型/行为 env 变量设进子进程环境，覆盖任何继承值。
- 覆盖值在 spawn 时从「当前生效 provider 决议」动态推导（复用现有 provider 持久化字段，零数据结构变更）。
- 三条 spawn 路径（fresh / resume / cancel-respawn）与三种 provider 模式（Direct / Router / Off-with-default）行为一致。
- 保持运行时 `SetModel` 通道不变，env 只做启动基线。

**Non-Goals:**
- 不做 T0/T1/T2 能力档位标注（spec 只留开放口）；落地时全部档位槽回退到主模型 id。
- 不新增任何持久化 provider 字段；不改 provider 数据结构。
- 不覆盖 OpenAI 协议 agent / 原生 ACP agent（非 claude）。
- 不改 router 协议层 / alias 翻译。

## Decisions

### D1: 推导值完全复用 provider 决议链，不引入独立配置源

新增模块读 `resolve_spawn_overrides` 同源的决议结果：`compute_provider_resolution` 返回的 `ProviderResolution`（Direct/Router 的 provider 身份）+ 现有 `default_model` 优先级（`default_selection.model` > provider `default_model` > 无）已产出"本次 spawn 的模型 id"。模型 env 的推导直接以它为唯一事实源——不再需要新配置、不改 overlay/state.json 结构。

- 采纳理由：用户已确认「从现有 provider 配置推导，不改 provider 数据结构」；同一决议喂两端（`--model` 与 env），天然满足 spec 的 "Model flag parity"（claude-env-cover Requirement: Model flag parity）。
- 备选：config.toml 新表 / daemon 环境变量 / provider 条目加字段——均被用户否决或属于"改 provider 数据结构"，弃。

### D2: 覆盖集合并进既有 extra_env，靠现有 SDK `.envs()` 语义达成覆盖

覆盖不是新机制，而是给 `resolve_spawn_overrides` 的 `extra_env` 多追加 7 个键。SDK 的 `.envs(&env)` 在 OS env 之上增量设置同名键 → 天然"注入值优先于继承值"。无需 `env_clear`（会清掉 sandbox 必需的三件套等），无需改动 `SessionManager`/`driver.rs` 的传递链，无需碰 SDK。

- 采纳理由：`driver.rs:166-167` 的 `env_map` → `ClaudeAgentOptions.env` → SDK `build_env()` clone + `.envs()` 链路已实证（子进程 env = OS env + extra_env 覆盖）。零新增泄漏面。
- 备选：spawn wrapper（`sebas` 侧包一层进程）——侵入 run 模型，弃。

### D3: 覆盖构造收敛在 `spawn_env.rs` 一个新纯函数，`session_boot.rs` 只加一次调用

`session_boot.rs::spawn_overrides`（fresh 与 resume 共用，`session_boot.rs:78-93,132,216`）是唯一注入点。在 `spawn_env.rs` 增加 `model_cover_env(provider, model_id) -> Vec<(String,String)>`（纯函数，7 键映射），`spawn_overrides` 在拿到 `(extra_env, extra_args)` 后把结果 append 进 `extra_env` 即可——fresh/resume/cancel-respawn 三路径自动同构。

- 采纳理由：改动面最小，单测可纯逻辑覆盖（沿用 `spawn_env.rs` 既有 ENV_LOCK + tempfile overlay 测试范式）。
- 覆盖 set 的 7 键是模块级常量表（key 列表），单测断言全量存在 + 覆盖语义。

### D4: 档位槽全部回退到主模型 id（为 T0/T1/T2 预留扩展点）

按用户决策：当前选什么模型，opus/sonnet/haiku/subagent 全部映射同一 id；`CLAUDE_CODE_EFFORT_LEVEL`/`CLAUDE_CODE_AUTO_COMPACT_WINDOW` 未配置时——依 spec 的最小强制集——同样回退主模型 id（不省略键）。未来 T0/T1/T2 落地时把 `model_cover_env` 的档位解析换成查 provider 模型的档位标注，其余不变。

### D5: Router 模式同样覆盖（协议层不动）

claude 永远只认 env；router 的请求转发不消费 `ANTHROPIC_MODEL`（alias 翻译若有另说）。Router 决议（`router_resolution`，`spawn_env.rs:219-235`）目前不产出模型 id——本轮 Router 决议链并入"router 上游 provider 的 default_model"作模型源（读 router_cfg.providers 或 overlay 的 model），使 Router 模式也能推导覆盖。用户未就此分叉作答，按现状技术事实定夺，spec 已写成统一覆盖（claude-env-cover "Cover applies uniformly across provider modes"）。

## Risks / Trade-offs

- [覆盖值把 `ANTHROPIC_MODEL` 等钉死，用户自己的 `~/.claude/settings.json` 模型设置不再生效] → 这正是本 change 意图（确定性优先）；Off 无模型可推导时仍不强制，保留逃生口（claude-env-cover "Off mode with no default still covers"）。
- [OS env 里存在 `CLAUDE_CODE_SUBAGENT_MODEL` 等业务相关覆盖值时被顶掉] → 覆盖为显式行为且集中可审计；值来源单一（决议链），排障看一条日志即可。
- [Router 模式下推导"router 上游 provider 的 default_model"可能读不到（config 结构差异）] → 读不到则不强制覆盖，行为等同旧 Off；spec 已允许 "no derivable model → not forced"。
- [7 键硬编码在代码常量表，上游 claude 新增 env 键时不自动跟随] → 演进成本低；spec 以枚举表锁定契约，claude 新键走新 change。
- [`ANTHROPIC_MODEL` 与 `--model` 双通道并存，若未来 claude 语义分叉可能不一致] → spec "Model flag parity" 要求同 id；实现上两者同源（同一决议结果），分歧不可能发生。

## Migration Plan

可选增量，无破坏：
1. 合并本 change → `model_cover_env` 默认**不启用**？—— 否：决议链有模型就推导覆盖（Direct/Off-with-default/Router 已配模型者行为变为"模型被钉到决议模型"）；未配模型者（裸 Off）不受影响。已运行会话不受影响（env 只在 spawn 时注入；resume 会按新逻辑重注入——对同一 provider 决议结果一致）。
2. 回滚：仅 revert 代码，无数据迁移；覆盖逻辑纯 spawn-time，重启即恢复旧行为。

## Open Questions

- T0/T1/T2 档位标注的持久化形态（放哪、谁来维护）——spec 已声明本次不落地，未来另开 change。
- Router 模式是否需要覆盖取决于"router 上游 default_model 是否可读"这一实现细节；spec 允许读不到时不强制，不会阻塞。
