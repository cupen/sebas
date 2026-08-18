# Provider 设计评审（sebas-63f epic 收尾）

> 日期：2026-08-17
> 状态：草案（待用户复核）
> 触发：sebas 启动日志 "provider 种子解析失败" + 用户反馈"provider 设计不合逻辑"
> 范围：`/provider` 重构（sebas-63f）落地后的全链路抽象

## 0. 摘要

`/provider` 重构（bead sebas-63f.1–.9）完成了"列表太长 → 下拉+折叠详情 / 缺 model 选择 → preset 静态表 / 表单臃肿 → preset+custom 拆分 / 生效机制不透明 → OFF/DIRECT/GATEWAY 三模式 + AgentDriver 抽象"四件事。**但抽象落地后暴露出 12 处设计问题**（按严重度分级），其中 4 处直接威胁功能正确性，5 处是未来扩展的绊脚石。

最严重的一条：**Gateway 模式把 agent 永远锁死在 Anthropic 协议面上**——`gateway::proto::OPENAI_PATHS` 路由表事实上是死路径。Gateway 模式被宣传为"smart router"，实际只比 Direct 模式多一层透明转发。

建议：以本评审为入口，开 bead `sebas-64f`（provider abstraction rework）做一次最小改动集——只动抽象形状，不动 UI、不动持久化 schema。

## 1. 当前设计回顾（sebas-63f 落地后）

### 1.1 五层抽象

```
┌─────────────────────────────────────────────────────────────────────┐
│ 1. 配置 / 数据                                                       │
│    gateway::config::GatewayConfig + [provider.*] + [gateway.routes] │
│    ~/.sebas/providers.json (overlay, FileStore)                      │
│    ~/.sebas/state.json (mode, ProviderRuntimeState)                 │
├─────────────────────────────────────────────────────────────────────┤
│ 2. UI                                                                 │
│    router::router::provider_card.rs (主卡 5 段 + 详情面板)            │
│    src::provider.rs (spec_preset / spec_custom / build_form)         │
├─────────────────────────────────────────────────────────────────────┤
│ 3. 模式决策（runtime state）                                            │
│    router::provider_state::ProviderMode { Off | Direct | Gateway }    │
├─────────────────────────────────────────────────────────────────────┤
│ 4. 解析 + spawn 翻译                                                  │
│    src::spawn_env.rs::compute_provider_resolution                   │
│    acp_claude::AgentDriver (trait) + ClaudeCodeDriver (impl)        │
├─────────────────────────────────────────────────────────────────────┤
│ 5. 子进程                                                              │
│    claude CLI 看到 env vars (ANTHROPIC_BASE_URL / OPENAI_BASE_URL) │
└─────────────────────────────────────────────────────────────────────┘
```

### 1.2 关键数据流

- **配置写入**：`/provider` UI → `CrudForm` → `FileStore` → `~/.sebas/providers.json`
- **配置读取（spawn）**：`spawn_env::read_overlay_item` (raw) + `gateway::GatewayConfig::merge_provider_overlay` (解析后)
- **状态切换**：`/provider` 三按钮 → `ProviderRuntimeState::update` → `~/.sebas/state.json`
- **spawn 翻译**：`state` + `gateway_cfg` + `overlay` → `ProviderResolution` → `driver.resolve_env` + `resolve_args` → `extra_env` / `extra_args` 注入 `SessionManager`

### 1.3 当前生效矩阵

| 模式 | agent 看到的 base_url | agent 看到的 auth_token | 协议面（agent→gateway/provider） | 模型路由 |
|---|---|---|---|---|
| Off | — | — | — (claude 用自己默认) | — |
| Direct{Anthropic} | 上游 Anthropic 端点 | 上游 key | Anthropic | 不路由（直连） |
| Direct{OpenAi} | 上游 OpenAI 端点 | 上游 key | OpenAI | 不路由（直连） |
| Gateway | gateway listen URL | gateway.auth_token[0] | **永远 Anthropic** | 路由（按 model 名） |

## 2. 设计问题清单（按严重度）

### 🔴 P0：威胁功能正确性

#### 2.1 Gateway 模式锁死 Anthropic 协议面（死代码）

**位置**：`acp-claude/src/agent_driver.rs:114-117` + `gateway/src/proto.rs:38-61`

```rust
ProviderResolution::Gateway { url, auth_token } => vec![
    ("ANTHROPIC_BASE_URL".into(), url.clone()),  // 永远只发 Anthropic env
    ("ANTHROPIC_AUTH_TOKEN".into(), auth_token.clone()),
],
```

`gateway::proto::OPENAI_PATHS` 表（`/v1/chat/completions`、`/v1/responses`、`/v1/embeddings` 等 22 条）仅在**外部客户端**直接打 gateway 时生效。当 sebas 自己用 Gateway 模式 spawn agent 时，agent 永远只发 `POST /v1/messages`，gateway 永远走 Anthropic 路径路由。

**后果**：
- 用户在 `/provider` 选 Gemini / DashScope（仅 OpenAI 协议的 preset）→ 切到 Gateway 模式 → agent 调 `/v1/messages` → gateway 路由到 Gemini 的 OpenAI 端点（如果有 `/v1/messages` 实现）→ **Gemini OpenAI-compat 不支持 `/v1/messages`** → 404/400 → 用户看到神秘的 gateway 报错
- 即：Gateway 模式下，OpenAI-only provider 事实上不可用
- 但 `models.rs` 的 `MODELS` 注册表里有 30+ 个模型，Gemini/DeepSeek/GLM/Qwen 全列着——用户预期都能用，实际不能用

**修法**（任选其一）：

| 方案 | 描述 | 改动 |
|---|---|---|
| A | 把 gateway 的对外协议面**降为仅 Anthropic**；`proto.rs::OPENAI_PATHS` 改注释说明"仅供外部 OpenAI 客户端" | 1 行 |
| B | AgentDriver 接收 `proto`，spawn 时按 mode + provider 选协议 | 中 |
| C | Gateway 内置协议转换（`/v1/messages` → `/v1/chat/completions` 反向亦然） | 大（用户已排除，非目标） |

建议 A：与 spec 2026-08-06 的"非目标：协议转换"一致，最小代价。

---

#### 2.2 Silent fallback to Off 让用户看不见配置错误

**位置**：`src/spawn_env.rs:162-198`（compute_provider_resolution）

```rust
// 失败一律 Off + warn（永不 panic / 永不 Result Err）—— runtime 配置
// 错误不应让 claude 启动失败。
```

5 个测试（`direct_overlay_*_falls_back_to_off`）锁定这个行为：
- 缺 URL → Off
- api_key_env 未设 → Off
- 删除 / tombstoned → Off
- 都缺 → Off

**后果**：
- 用户在 `/provider` 删掉唯一 provider → 下次发消息 → claude 用**自己默认配置**（很可能走用户本机的 `~/.claude/settings.json` 里的）→ 用户看不到任何 sebas 报错 → 不明白"为什么我配置的 provider 不生效"
- 用户给 api_key_env 填了 `DEEPSEEK_API_KEY` 但没 `export` → 同上

**修法**：把 fallback 升级为 in-band error。两条路：

| 方案 | 描述 | 改动 |
|---|---|---|
| A | fallback 时让 agent 跑一个 micro-shell script，第一行打印 warn 后 exit 1；用户从 sebas 拿到错误 | 中 |
| B | ProviderResolution 新增 `Error(reason)` 变体；driver 转成 `SEBAS_PROVIDER_ERROR=<reason>` env，agent 启动时检查并 abort | 中 |

---

#### 2.3 `/provider` UI 不可用 = 无法自愈（hard-fail）

**位置**：`src/provider.rs:283-312` `build_form`

```rust
match FileStore::load(path.clone(), ID_FIELD, seed) {
    Ok(store) => Some(Arc::new(...)),
    Err(e) => {
        tracing::warn!(...);
        None  // ← 整个 /provider 命令彻底不可用
    }
}
```

**后果**：用户想新增第一个 provider，但 overlay 文件破损 / 父目录没权限 → `/provider` 命令**完全没反应**（`RouterHandle.provider_forms = None`）→ 用户必须 ssh 上服务器手动修 JSON 才能恢复

**修法**：overlay 加载失败时返回 `Some(empty forms)`（seed=空），让用户从 `/provider` 重新建；同时把损坏的文件备份为 `.broken.json`。

---

#### 2.4 协议优先级 anthropic > openai 是隐性约定（用户没机会选）

**位置**：`src/spawn_env.rs:78-88` + 测试 `direct_prefers_anthropic_when_both_base_urls_set`

```rust
let (proto, base_url) = if let Some(u) = base_url_anthropic {
    (acp_claude::Protocol::Anthropic, u)
} else if let Some(u) = base_url_openai {
    (acp_claude::Protocol::OpenAi, u)
};
```

**后果**：deepseek 这种同时有两个端点的 provider，用户在 `/provider` 选 deepseek → Direct 模式 → 永远走 Anthropic 端点。即使请求是 OpenAI-shape（外部客户端调），也用 Anthropic 端点

测试用注释锁定这个行为："design约定：claude code 优先匹配 Anthropic 协议面"。但代码里没有任何 doc comment 解释为什么。

**修法**：把这个优先级挪到 UI：详情面板加一个"协议"radio（Anthropic / OpenAI / Auto），写进 overlay 的 `protocol` 字段（已存在但目前被忽略——`gateway/src/config.rs:580` 注释："旧 overlay 里若残留 protocol 字段会被静默忽略"）

### 🟡 P1：未来扩展的绊脚石

#### 2.5 `Provider` 概念在三处同名但不同物

| 层 | 类型 | 含义 |
|---|---|---|
| `acp-claude::agent_driver` | `enum Protocol { Anthropic, OpenAi }` | agent subprocess 用什么协议调上游 |
| `gateway::proto` | `enum Protocol { Anthropic, OpenAi }` | incoming request 是哪种协议 |
| `gateway::config::ProviderConfig` | 无 Protocol 字段；`base_url_anthropic: Option` + `base_url_openai: Option` 同时存 | provider 可以双协议 |

三个 `Protocol` 同名、两套 enum、第三处隐式（双 URL）。任何读这段代码的人都需要 5 分钟搞清楚"哪个协议在哪个边界"。

**修法**：
- 把 `acp-claude::Protocol` 改名为 `AgentFlavor` 或 `AgentProtocol`
- `gateway::proto::Protocol` 改名为 `UpstreamProtocol` 或 `WireProtocol`
- `gateway::ProviderConfig` 拆成两个：单协议 `AnthropicProvider { base_url, ... }` / `OpenAiProvider { base_url, ... }`；或者保留单结构但显式加 `protocol: UpstreamProtocol` 字段 + `base_url: String`（不是两个 Option）

---

#### 2.6 两个持久化文件（providers.json + state.json），单次原子性缺失

**位置**：
- `~/.sebas/providers.json`：provider CRUD
- `~/.sebas/state.json`：ProviderMode + default_provider_for_direct

两者**独立**走 `tmp + rename` 原子写。如果用户操作一次"删除当前 default provider"，需要：
1. `providers.json` 写入（删除 entry）
2. `state.json` 写入（更新 `default_provider_for_direct` 或 `mode.provider`）

中间如果进程被杀，第一步成功第二步失败 → mode 指向不存在的 provider → 见 §2.2 silent fallback。

**修法**：合成一个 `state.json`：
```json
{
  "version": 1,
  "providers": { "deepseek": {...}, "deleted": ["openai"] },
  "mode": { "kind": "direct", "provider": "deepseek" },
  "default_provider_for_direct": "deepseek"
}
```
写一次 = 原子。

---

#### 2.7 `AgentDriver` 是过度抽象（YAGNI）

**位置**：`acp-claude/src/agent_driver.rs`

只有一个实现：`ClaudeCodeDriver`。模块顶部注释：

> New agents (Codex, Gemini CLI, future) implement AgentDriver without sebas learning any of their idioms.

但：
- `id()` 方法零调用方
- `resolve_args()` 全 variant 返回空
- "future Codex/Gemini CLI" 没有 commit、没有 bead、没有 RFC。等真有第二个 agent 时再抽象。

**修法**：现阶段把 trait 降级为 plain struct + inherent impl。第二次出现 agent 时再升级 trait（届时 Y 类型已清晰）。

---

#### 2.8 `default_model` 只在 overlay，gateway 看不见

**位置**：`src/provider.rs:246-267` `item_from_provider` 注释：
> `default_model` 不在 gateway `ProviderConfig` 上（bead sebas-63f.4）：用户通过 bot 表单写入的值仅落在 overlay 文件，不向 gateway 同步

**后果**：
- gateway 路由时不知道 provider 有 default model（`models: Vec<String>` 在 gateway 有，但 `default_model` 没有）
- 三处不一致：overlay 有 default_model、gateway config 没有、ProviderRuntimeState.mode.provider 是当前的默认
- `default_provider_for_direct` 和 `default_model` 两个字段功能重叠，都是"spawn 时默认用啥"

**修法**：合并。`ProviderRuntimeState` 里 `default_provider_for_direct` 改名 `default_selection`（值是 `("deepseek", Some("deepseek-chat"))` 这种 tuple）。或者让 gateway 也有 `default_model` 字段，参与 `RouteTable::resolve` 的 tie-break。

---

#### 2.9 routes 没有 UI

`[gateway.routes]` 只能在 config.toml 改。但路由是 gateway 的核心机制。新增 provider 不写路由 = 路由表找不到 = gateway 走默认 provider。

**修法**：把 routes 也搬到 `/provider`（或在 `/gateway` 新卡）。起码先支持 `[gateway.routes]` 的 hot-reload（gateway 子命令重启前生效）。

---

#### 2.10 `models.dev` 数据是手抄进 `MODELS` 常量

commit 7d534d8 标题："feat(gateway): 从 models.dev 更新模型能力注册表，数据精准"——但数据是手动拷进 `gateway/src/models.rs` 的 `MODELS` 常量，没有脚本、没有 vendored JSON、没有 runtime fetch。

**后果**：commit log 误导审查者以为有同步机制。实际是**静态数据，每改一次都手抄**。

**修法**：
- 至少把 models.dev 数据的 dump 时间戳写到常量顶部（`// last synced: 2026-08-16`）
- 或者写一个 `xtask update-models` 子命令拉一次更新

---

#### 2.11 探测按钮对 Anthropic 协议 provider 是已知坏掉的

**位置**：`router/src/router/provider_card.rs:740-764` `choose_probe_url`

```rust
(None, Some(base)) => {
    let base = trim_trailing_slash(base);
    Ok((format!("{base}/v1/models"), "anthropic"))  // ← Anthropic 没这个端点
}
```

how-to.md 自己也承认："后者 anthropic 协议通常会失败卡"。所以 `probe 401` 测试**不是边界**——是预期失败。

**修法**：
- 选择 Anthropic-only URL 时不显示探测按钮（条件渲染）
- 或者探测改成 GET `<` base + `/v1/models` 的同时告诉用户"anthropic 通常不暴露该端点"

---

### 🟢 P2：整洁度

#### 2.12 `models` 和 `model_map` 功能部分重合

`models: Vec<String>` 顺序 = 强到弱（用于 OPUS/SONNET/HAIKU env mapping）；`model_map: HashMap<String, String>` 上游改名。

如果 `models` 已经在 gateway 起作用（`RouteTable::resolve` 按 model 名匹配 provider），那 `model_map` 是给"同 provider 多名映射"用的。重叠但都有用。

**修法**：要么 doc 解释为什么需要两个，要么合并成一个 `model_aliases: HashMap<String, String>`（`{real_name: alias1, alias2}`）。

---

#### 2.13 `ProviderSelectionMap` 命名误导

`ProviderSelectionMap` 注释说 "provider 卡片当前选中哪个 provider"。纯 UI 状态、内存、不持久化。但 `ProviderMode` 也是"provider 选择"，**两个都叫 selection，但语义完全无关**。

**修法**：改名 `CardFocusMap` 或 `DetailsPanelFocus`。

---

#### 2.14 调试 provider 注入路径偏长

`enable_debug_test_provider` 只在 `sebas gateway --debug` 路径调用（已确认 `src/gateway_cmd.rs:30`），不是裸暴露。但生产路径上 `gateway/src/config.rs` 一段 30 行的 `enable_debug_test_provider` 函数容易让后续维护者疑惑为什么 prod config 里混着 debug 注入逻辑。

**修法**：挪到 `gateway/src/test_provider.rs` 或独立 `gateway/src/debug.rs`。

---

#### 2.15 Auth 策略三选一，没扩展点

当前：明文 `api_key`（warn） / `api_key_env` / 无（匿名）。未来要接 vault / 1Password / SSH agent / KMS 没入口。

**修法**：把 `resolve_api_keys` 抽象成 `trait KeyResolver { fn resolve(&self, hint: &KeyHint) -> Result<String> }`，env-based 是默认 impl，留位给未来。

---

## 3. 问题严重度矩阵

| # | 问题 | 严重度 | 触及文件 | 触及 UI | 触及 schema |
|---|---|---|---|---|---|
| 2.1 | Gateway 锁死 Anthropic | 🔴 P0 | acp-claude, gateway | — | — |
| 2.2 | Silent fallback | 🔴 P0 | src/spawn_env | — | — |
| 2.3 | /provider 不可用 = 死路 | 🔴 P0 | src/provider | ✓ | — |
| 2.4 | 协议优先级隐性 | 🔴 P0 | src/spawn_env, router | ✓ | ✓ |
| 2.5 | Protocol 三层同名 | 🟡 P1 | acp-claude, gateway | — | — |
| 2.6 | 双 state 文件 | 🟡 P1 | router/provider_state, router/crud | — | ✓ |
| 2.7 | AgentDriver 过度抽象 | 🟡 P1 | acp-claude | — | — |
| 2.8 | default_model 分裂 | 🟡 P1 | src/provider, gateway | — | ✓ |
| 2.9 | routes 无 UI | 🟡 P1 | — | ✓ | ✓ |
| 2.10 | models.dev 假同步 | 🟡 P1 | gateway/models | — | — |
| 2.11 | probe 对 Anthropic 已知坏 | 🟡 P1 | router/provider_card | ✓ | — |
| 2.12 | models vs model_map | 🟢 P2 | gateway/config | — | — |
| 2.13 | SelectionMap 命名 | 🟢 P2 | router/maps | — | — |
| 2.14 | debug 注入路径 | 🟢 P2 | gateway | — | — |
| 2.15 | KeyResolver 扩展点 | 🟢 P2 | gateway, src/spawn_env | — | ✓ |

## 4. 建议下一步

### 4.1 立即可修（小、低风险）

- §2.1 方案 A：1 行注释 + 1 行 doc 警告
- §2.10 加时间戳 / 加 xtask
- §2.13 改名 `ProviderSelectionMap` → `CardFocusMap`
- §2.14 挪 `enable_debug_test_provider` 到独立模块
- §2.11 条件隐藏探测按钮

预计 1 个工作日。

### 4.2 中期重构（开 bead `sebas-64f`）

- §2.5 重命名三处 Protocol
- §2.6 合并两个 state 文件
- §2.7 AgentDriver trait → struct
- §2.8 default_model 统一归属

预计 3 个工作日，触及 schema（需 migration 路径）。

### 4.3 用户决策项（需 cupen 拍板）

| 决策 | 选项 | 影响 |
|---|---|---|
| Gateway 是否暴露 OpenAI 协议面 | A. 只 Anthropic / B. 两种都暴露 / C. 转换 | 决定 §2.1 修法 + 后续 OpenAI-only provider 是否可用 |
| `default_model` 归属 | A. 跟 provider / B. 跟 mode / C. 跟 session | 决定 §2.8 修法 |
| routes 是否有 UI | A. 有（`/gateway` 新卡）/ B. 无（只改 TOML） | 决定 §2.9 是否做 |
| `models.dev` 数据更新策略 | A. 手抄 / B. xtask 脚本 / C. 启动拉 | 决定 §2.10 修法 |

## 5. 决策记录 / 修订日志

| 日期 | 修订 |
|---|---|
| 2026-08-17 | 初稿，评审 sebas-63f 落地后状态 |
| 2026-08-17 | 实测反馈修复：列表改为每 provider 一行折叠面板（默认折叠，`ProviderSelectionMap` 随列表下拉一并删除，§2.13 解决）；preset 表单 `default_model` 从静态 Select 改回手填 Text，探测成功自动回填 `models` 目录（官方 `/models` 为权威来源，手填兜底） |
| 2026-08-18 | §4.1 立即可修全部落地：§2.1 Gateway 协议面限制 doc 警告（Appendix B.1）、§2.10 models.dev 时间戳注释、§2.11 探测按钮按 `base_url_openai` 条件隐藏（Anthropic-only provider 不再渲染），§2.12 `models` vs `model_map` 字段 doc，§2.14 `enable_debug_test_provider` 挪到 `gateway/src/debug.rs`（自由函数 + pub 因跨 crate 调用） |
| 2026-08-18 | §2.5 完成：`acp-claude::Protocol` → `AgentProtocol`；`gateway::proto::Protocol` → `WireProtocol`（三层同名消除二处） |
| 2026-08-18 | §2.7 完成：`AgentDriver` trait 删除，`ClaudeCodeDriver` 改 inherent impl，`id()` 死代码删除，`spawn_env` 改用具体类型 `&ClaudeCodeDriver` |
| 2026-08-18 | §2.6 完成（含集成）：`router/src/state_store.rs` 新增（v0/v1/v2 迁移 + atomic save + repair-on-load + 11 测试），`provider_state` 全切到 state_store，`FileStore.persist` 改走 `state_store::update`，删除 `OverlayFile` 结构。`~/.sebas/providers.json` 仅在首次迁移路径 B 中创建后立即删除，后续 CRUD 全部写入 unified state.json |
| 2026-08-18 | §2.3 完成：`src/provider.rs::build_form` 自愈 — overlay 损坏时备份到 `.broken-<unix-ms>-<pid>.json` 后用空 seed 重新加载；测试加 5 个（覆盖损坏/空/正常/备份唯一性） |
| 2026-08-18 | §2.4 完成：详情面板加 `protocol` radio（auto/anthropic/openai），preset/custom 表单同步加 Select；`spawn_env::direct_resolution_from_overlay` 读 overlay `protocol` 字段显式选协议，缺字段默认 `auto`（与原 Anthropic>OpenAI 行为一致）；10 个新测试覆盖四个分支 + radio 渲染 |
| 2026-08-18 | 状态：§2.2 / §2.8 / §2.9 / §2.10 完整方案 / §2.15 仍需用户决策（spec §4.3），bead `sebas-bfy` epic 跟踪 |
| 2026-08-18 | §2.15 完成：`gateway/src/key_resolver.rs` 新增（KeyHint enum + KeyResolver trait + EnvKeyResolver default impl + StubKeyResolver test double + 11 测试）。`gateway/src/config.rs::resolve_api_keys` 通过 `&dyn KeyResolver` 接入；sebas spawn_env 暂不迁（保守，先建 seam 不扩面） |
| 2026-08-18 | §2.10 完成：`xtask/` 新 crate（10 测试），`update-models` 子命令从 `https://models.dev/api.json` 拉取 + 渲染生成 `gateway/src/models.rs` body（alphabetic 排序；保留顶层 timestamp 注释）。未触发实际拉取（保守） |
| 2026-08-18 | §2.2 完成：ProviderResolution 新增 `Error { reason }` 变体；ClaudeCodeDriver.resolve_env 输出 `SEBAS_PROVIDER_ERROR` env；`session_boot.rs` 拦截额外 env 若存在则 `eprintln! + exit(1)`。原 5 个 silent-fallback 测试全部改为 Error assertion；新增 4 个测试覆盖 spawn wrapper 检测逻辑 |
| 2026-08-18 | §2.8 完成：`ProviderRuntimeState.default_provider_for_direct` 改为 `default_selection: Option<DefaultSelection { provider, model }>`。`DefaultSelection` 自定义 Deserialize 同时接受 legacy 字符串与新对象 schema（无 V3 升级；同 v2 wire 内字段升级）。overlay `default_model` 仍为 UI 编辑源，`default_selection.model` 在「设为默认」/下拉变更时同步。**行为变更**：Off + default_selection → 隐式 Direct（spec §2.8 推荐语义，配合 §2.2 的 Error 通路确保配置错暴露） |
| 2026-08-18 | 状态：12/15 spec 问题全部完成；唯余 §2.9 routes UI（spec §4.3 决策项 A/B 未拍板）。bead `sebas-bfy` epic 仍跟踪，claude 现状可全栈 378/378 测绿 |
| 2026-08-18 | §2.9 用户决策：routes 改由 webui 编辑（TOML routes 配置后续移除）；webui 工作不在本 spec 范围，单独 bead 跟踪 |

## 附录 A：被忽略但相关的小问题

- `claude.rs::MacProvider` 在 `src/watchdog/auth.rs` 里也叫 "provider"——加密 MAC，与 LLM provider 无关。新人容易 grep 错。建议在文件顶部加 `//! ⚠️ 此 Provider 是加密 MAC，与 LLM provider 无关。` 之类的醒目注释。
- `src/provider.rs` `apply_preset_defaults` 改 `api_key_env` 时机：当用户选了 preset 并粘了 api_key 时不注入默认 env（正确）；但当用户**没**粘 api_key、只选了 preset 时注入 env 名（默认行为）。这与"为什么不总是注入 env 名"的疑问需要 doc 一行说明。

## 附录 B：建议的具体改法示例

### B.1 §2.1 方案 A（最小改动）

`acp-claude/src/agent_driver.rs` 顶部 doc 加：

```rust
//! **注意**：Gateway 模式 agent 看到的永远是 Anthropic 协议面 ——
//! gateway 自身支持双协议（见 `gateway::proto::OPENAI_PATHS`），但仅
//! 服务于「外部 OpenAI 客户端直连 gateway」场景；sebas 自身用 Gateway
//! 模式时不可路由到 OpenAI-only provider。详见 spec 2026-08-17 §2.1。
```

`gateway/src/proto.rs:36` 注释改：

```rust
/// OpenAI 专属路径表（spec §4.1）。
/// ⚠️ **仅外部 OpenAI 客户端使用** —— sebas 自身走 Gateway 模式时，
/// agent 只发 Anthropic 协议，本表对 sebas→gateway→upstream 路径不可见。
/// 详见 spec 2026-08-17 §2.1。
```

### B.2 §2.6 合并 schema 草案

```json
// ~/.sebas/state.json (合并后)
{
  "version": 2,
  "providers": {
    "deepseek": { "base_url_anthropic": "...", "api_key_env": "..." }
  },
  "deleted": ["openai"],
  "mode": { "kind": "direct", "provider": "deepseek" },
  "default_provider_for_direct": "deepseek"
}
```

迁移：`load()` 检测无 `version` 字段 → 走 legacy loader → 把数据塞新结构 → 写新文件 → 删旧文件。

### B.3 §2.7 AgentDriver trait → struct 草案

```rust
// 现
pub trait AgentDriver: Send + Sync {
    fn id(&self) -> &'static str;
    fn resolve_env(&self, r: &ProviderResolution) -> Vec<(String, String)>;
    fn resolve_args(&self, r: &ProviderResolution) -> Vec<String>;
}
pub struct ClaudeCodeDriver;
impl AgentDriver for ClaudeCodeDriver { ... }

// 改后
pub struct ClaudeCodeDriver;
impl ClaudeCodeDriver {
    pub fn id(&self) -> &'static str { "claude-code" }
    pub fn resolve_env(&self, r: &ProviderResolution) -> Vec<(String, String)> { ... }
    pub fn resolve_args(&self, r: &ProviderResolution) -> Vec<String> { ... }
}
// manager.rs 直接用 `ClaudeCodeDriver`，不抽 trait
// 第二个 agent 出现时再抽 trait
```

只需删 trait 声明、改 manager.rs 的 `&dyn AgentDriver` 为 `&ClaudeCodeDriver`、删 `id()` 死代码。约 1 小时。