# 卡片流模型重建 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 sebas 的卡片流模型从「每个事件重建空卡整卡替换」重建为「同卡累积（v1→v5）+ 150ms 节流 + 接上 `[card]` 死配置」，复活被丢弃的 ThinkingDelta/ToolEnd/ToolProgress，让 terminal Error 保留死前 transcript。

**Architecture:** 状态在 router、节流在 pump。router 新增 `CardState`（`session_id → {user_prompt, status_emoji, body: Vec<CardElement>}`）与四个方法 `seed_card`/`apply_event`（纯状态）/`flush_card`/`drop_card`；`apply_event_to_out` 退化为 `apply_event + flush_card` 的同步薄封装（供即时路径与旧测试复用）。pump（`src/run.rs::spawn_acp_pump`）改造为 `tokio::select!` 循环：流式事件只累积状态 + 标脏，`tokio::time::interval(150ms)` 到点调 `flush_card`；Finished/terminal Error/PermissionRequest 走即时 `apply_event_to_out`；通道关闭 `drop_card`。`CardConfig` 从 `sebas::config` 迁到 `feishu::cards`（依赖链最底端，router/sebas 均可引用）。

**Tech Stack:** Rust 2024/edition 2021 混合（sebas crate = 2024, router/feishu = 2021），tokio 1.40（sebas crate `full` 含 `time`/`macros`），serde_json，agent-client-protocol 2.0，insta 1（feishu 快照测试）。

## Global Constraints

- 节流契约（spec §6，验收以契约为准）：**事件即时累积**（`apply_event` 同步更新状态）、**出站 UpdateCard 在 150ms 内至多一次**、**Finished/terminal 立即出最终态**。确切的 async 机制由本计划钉死为 `tokio::time::interval(150ms) + dirty bool`（见 Task 6），与 spec §6 建议的 `Option<Sleep>+select+pending()` 契约等价但规避了 select 跨臂 `&mut` 借用冲突。
- 截断（spec §7）：单元素文本 > `max_user_text_chars`(4000) 截断到上限 + 追加灰注 `(已折叠 N 字)`；`ToolEnd.result` > `max_tool_output_chars`(2000) 同上；仅 `fold_long_output=true` 时启用，`false` 时不截断（总量兜底仍生效）。
- 总量上限（spec §7）：body 累积字符 > 24000 → 从最旧行丢弃；**当最旧行是 `CardElement::Hr` 时，连同其后一个元素一起丢弃**（不留悬空分隔线）。本计划钉死的丢旧策略：逐个丢最旧元素，若最旧是 `Hr` 则再丢其后的一个元素。
- `theme_color`（spec §7）：`render_accumulated_card` 用它替代硬编码 `"blue"`；权限卡 `"orange"` 保留（独立卡路径不动）。
- 状态 emoji FSM（spec §5）：seed=`👀`；首个 TextDelta/ThinkingDelta/ToolStart/ToolProgress/ToolEnd/非 terminal Error → `🚧`；`Finished` → `✅`；terminal Error → `❌`；已 `🚧`/`✅`/`❌` 不回退 `👀`；terminal Error 即便之前 `🚧` 也置 `❌`。
- 构造器策略（本计划钉死，优于 spec §7 的 `new(map, card_cfg)` 改签名方案）：保留 `RouterHandle::new(map)`（用 `CardConfig::default()`）+ 新增 `RouterHandle::new_with_card_config(map, card_cfg)`。**14 处测试站点零改动**，仅 `src/run.rs:32` 改用新构造器。更好满足 spec §9「现有 router_test/e2e_test/terminal_error_test 零改动通过」。
- terminal Error 并入累积模型（spec §8）：走 `apply_event`（append `❌ {message}` + 置 ❌ + 保留死前 transcript）→ `flush_card` → `remove_by_session` → `drop_card`。
- 非目标（spec §10）：不动 permission 卡渲染路径、不搞真 emoji reaction、不做媒体/slash/重启恢复/ACP watchdog。
- 提交规范（`.claude/rules/how-to.md`）：Conventional Commits，相关性大的代码一起提交，能一行说明不啰嗦。
- 工作分支：`feat/card-streaming-model`（从 main 分出；spec 已在 0d4e468 落 main）。

## File Structure

| 文件 | 责任 | 动作 |
|---|---|---|
| `feishu/src/cards.rs` | 卡片渲染 + body 累积 + 截断/总量；`CardConfig` 定义 | 新增 `CardConfig`、`apply_event_to_card`、`render_accumulated_card`；删除旧 `apply_event(&mut Card,...)` 与 `render_root_card` 的硬编码 `"blue"`（改用参数） |
| `feishu/src/lib.rs` | 模块导出 | 导出 `CardConfig` |
| `src/config.rs` | TOML 配置 | 删除本地 `CardConfig`，改 `pub card: feishu::cards::CardConfig`（re-export，TOML `[card]` 段字段名不变） |
| `router/src/card_state.rs`（新建） | `CardState` 结构 + `CardStateMap` 存储 | 新建 |
| `router/src/lib.rs` | 模块导出 | `pub mod card_state;` |
| `router/src/router.rs` | router 状态/出站方法 | 新增 `seed_card`/`apply_event`/`flush_card`/`drop_card`/`new_with_card_config`；重写 `apply_event_to_out`；`RouterHandle` 加 `card_states` + `card_cfg` 字段 |
| `src/run.rs` | pump + dispatch_out 装配 | 重写 `spawn_acp_pump`（select+interval+dirty，`pub` 供测试）；dispatch_out SpawnAcp 臂加 `seed_card` + 初始卡用 `render_accumulated_card` + `new_with_card_config` |
| `tests/bin/fake-claude.rs` | ACP 测试桩 | 加 `stream` prompt 分支（5 个 chunk + end_turn） |
| `router/tests/card_state_test.rs`（新建） | CardState 累积/FSM/同步语义单测 | 新建 |
| `router/tests/terminal_error_test.rs` | terminal 保留 transcript | 追加一个测试 |
| `feishu/tests/cards_test.rs` | 渲染快照 + append/截断/总量 | 追加测试 |
| `tests/pump_unit_test.rs`（新建） | pump 节流单测（合成 rx，无 fake-claude） | 新建 |
| `tests/card_stream_e2e_test.rs`（新建） | pump 端到端（fake-claude stream 模式） | 新建 |

---

### Task 1: 迁移 CardConfig 到 feishu::cards

把 `CardConfig` 从 `sebas::config` 迁到 `feishu::cards`（依赖链最底端，router 与 cards 均可引用）。`sebas::config` 改为 re-export，TOML `[card]` 段字段名不变，现有 `tests/config_test.rs` 零改动通过。

**Files:**
- Create: 无
- Modify: `feishu/src/cards.rs`（文件顶部，`Card` 结构之前）
- Modify: `feishu/src/lib.rs`
- Modify: `src/config.rs:130-164`（删除本地 `CardConfig` + 4 个 default fn；`Config::card` 改 re-export）
- Test: `feishu/tests/cards_test.rs`（追加 `card_config_defaults`）；`tests/config_test.rs`（现有，零改动回归）

**Interfaces:**
- Consumes: 无（第一个任务）
- Produces: `feishu::cards::CardConfig { theme_color: String, max_user_text_chars: usize, max_tool_output_chars: usize, fold_long_output: bool }`，带 `#[derive(Debug, Clone, Deserialize)]` + `impl Default`。字段名与旧 `sebas::config::CardConfig` 完全一致（TOML 兼容）。`sebas::config::Config.card: feishu::cards::CardConfig`。

- [ ] **Step 1: 写失败测试（feishu 默认值）**

在 `feishu/tests/cards_test.rs` 顶部 `use` 之后追加：

```rust
#[test]
fn card_config_defaults() {
    use feishu::cards::CardConfig;
    let c = CardConfig::default();
    assert_eq!(c.theme_color, "blue");
    assert_eq!(c.max_user_text_chars, 4000);
    assert_eq!(c.max_tool_output_chars, 2000);
    assert!(c.fold_long_output);
}

#[test]
fn card_config_from_toml() {
    use feishu::cards::CardConfig;
    let toml = r#"
theme_color = "orange"
max_user_text_chars = 100
max_tool_output_chars = 50
fold_long_output = false
"#;
    let c: CardConfig = toml::from_str(toml).unwrap();
    assert_eq!(c.theme_color, "orange");
    assert_eq!(c.max_user_text_chars, 100);
    assert_eq!(c.max_tool_output_chars, 50);
    assert!(!c.fold_long_output);
}
```

`feishu/tests/cards_test.rs` 当前没有 `toml` 依赖。需在 `feishu/Cargo.toml` 的 `[dev-dependencies]` 加 `toml = "0.8"`：

```toml
[dev-dependencies]
insta = { version = "1", features = ["yaml"] }
toml = "0.8"
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p feishu --lib --tests card_config_defaults card_config_from_toml -- --nocapture`
Expected: FAIL — `feishu::cards::CardConfig` 不存在（编译错误 `cannot find type CardConfig`）。

- [ ] **Step 3: 实现 — 在 feishu::cards 定义 CardConfig**

在 `feishu/src/cards.rs` 顶部 `use` 之后、`Card` 结构之前插入：

```rust
use serde::Deserialize;

/// 卡片流配置（spec §7）。原 `[card]` TOML 段，解析后由 router/feishu 共用。
/// 落在 feishu crate（依赖链最底端），router 与 cards 均可引用。
#[derive(Debug, Clone, Deserialize)]
pub struct CardConfig {
    #[serde(default = "default_theme_color")]
    pub theme_color: String,
    #[serde(default = "default_max_user_text")]
    pub max_user_text_chars: usize,
    #[serde(default = "default_max_tool_output")]
    pub max_tool_output_chars: usize,
    #[serde(default = "default_true")]
    pub fold_long_output: bool,
}

impl Default for CardConfig {
    fn default() -> Self {
        Self {
            theme_color: default_theme_color(),
            max_user_text_chars: default_max_user_text(),
            max_tool_output_chars: default_max_tool_output(),
            fold_long_output: true,
        }
    }
}

fn default_theme_color() -> String {
    "blue".into()
}
fn default_max_user_text() -> usize {
    4000
}
fn default_max_tool_output() -> usize {
    2000
}
fn default_true() -> bool {
    true
}
```

注意：`feishu/src/cards.rs` 顶部已有 `use serde::Serialize;`（第 2 行）。把 `Deserialize` 合并进去：改为 `use serde::{Deserialize, Serialize};`（删除新加的独立 `use serde::Deserialize;` 行，避免重复导入告警）。

- [ ] **Step 4: 在 feishu::lib 导出 CardConfig**

`feishu/src/lib.rs` 当前：

```rust
pub mod cards;
pub mod client;
pub mod events;
pub mod media;

pub use client::{FeishuClient, FeishuConfig, FeishuToken};
pub use events::{CardAction, FeishuEnvelope, FeishuIn, MessageBody, SessionKey};
```

无需改动 `lib.rs` —— `CardConfig` 在 `cards` 模块里且 `cards` 是 `pub mod`，调用方用 `feishu::cards::CardConfig` 即可。跳过此步。

- [ ] **Step 5: sebas::config 改 re-export**

`src/config.rs:130-164` 当前定义了本地 `CardConfig` + 4 个 default fn。删除整段（130-164 行，即 `#[derive(Debug, Clone, Deserialize)] pub struct CardConfig { ... }` 到 `fn default_true() -> bool { true }`）。

然后把 `Config` 结构里的 `pub card: CardConfig` 改为 re-export 类型。`src/config.rs:4-18` 当前的 `Config` 结构：

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub feishu: FeishuConfig,
    #[serde(default)]
    pub acp: AcpConfig,
    #[serde(default)]
    pub router: RouterConfig,
    #[serde(default)]
    pub card: CardConfig,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub log: LogConfig,
}
```

把 `pub card: CardConfig,` 改为 `pub card: feishu::cards::CardConfig,`。

由于 `src/config.rs` 顶部没有 `use feishu`，需要确认 `sebas` crate 能引用 `feishu`（Cargo.toml 已有 `feishu = { path = "feishu" }`，✓）。用全路径 `feishu::cards::CardConfig` 即可，无需 use。

- [ ] **Step 6: 运行测试验证通过**

Run: `cargo test -p feishu card_config_defaults card_config_from_toml -- --nocapture`
Expected: PASS（两个测试绿）。

Run: `cargo test -p sebas --test config_test -- --nocapture`
Expected: PASS（现有 `tests/config_test.rs` 零改动通过 —— TOML `[card]` 段字段名未变，re-export 透明）。

- [ ] **Step 7: 全量编译确认**

Run: `cargo build --workspace`
Expected: 编译通过（`sebas::config::CardConfig` 已删除，引用点只剩 `Config.card` 字段类型；若有遗漏的 `CardConfig` 裸引用会编译失败，按报错修）。

- [ ] **Step 8: 提交**

```bash
git add feishu/src/cards.rs feishu/src/lib.rs feishu/Cargo.toml src/config.rs feishu/tests/cards_test.rs
git commit -m "refactor(feishu): 迁 CardConfig 到 feishu::cards，sebas::config 改 re-export"
```

---

### Task 2: CardState + CardStateMap（router）

router 新增纯状态结构 `CardState` 与并发存储 `CardStateMap`（平行于 `MsgIdMap`）。本任务只交付数据结构 + 存储方法（seed/get/apply/drop），不含渲染与 FSM（FSM 在 Task 5）。

**Files:**
- Create: `router/src/card_state.rs`
- Modify: `router/src/lib.rs`（加 `pub mod card_state;`）
- Test: `router/tests/card_state_test.rs`（新建，本任务只测存储三方法）

**Interfaces:**
- Consumes: `feishu::cards::CardElement`（router 已依赖 feishu）
- Produces:
  - `pub struct CardState { pub user_prompt: String, pub status_emoji: String, pub body: Vec<feishu::cards::CardElement> }`，带 `pub fn new(user_prompt: &str) -> Self`（emoji=`👀`，body 空）与 `pub fn lazy() -> Self`（user_prompt=`""`，emoji=`👀`，body 空）。
  - `#[derive(Default, Clone)] pub struct CardStateMap { inner: Arc<RwLock<HashMap<String, CardState>>> }`，方法：
    - `pub async fn seed(&self, session_id: String, user_prompt: String)` — 幂等：entry 已存在则保留（防 SpawnAcp 重入冲掉已累积状态）。
    - `pub async fn apply<F: FnOnce(&mut CardState)>(&self, session_id: &str, f: F)` — 无 entry 时 `lazy()` 兜底插入，再对 `&mut CardState` 跑 `f`。
    - `pub async fn snapshot(&self, session_id: &str) -> Option<CardState>` — 克隆一份给 flush 渲染。
    - `pub async fn drop(&self, session_id: &str)` — 移除 entry（session 死亡/通道关时防无界增长）。

- [ ] **Step 1: 写失败测试（存储三方法）**

`router/tests/card_state_test.rs`：

```rust
//! CardStateMap 存储语义单测（FSM/累积在 card_state_test 的后续测试 + Task 5 覆盖）。

use feishu::cards::CardElement;
use router::card_state::{CardState, CardStateMap};

#[tokio::test]
async fn seed_is_idempotent_keeps_accumulated_prompt() {
    let m = CardStateMap::default();
    m.seed("s1".into(), "original".into()).await;
    m.apply("s1", |st| {
        st.body.push(CardElement::Markdown {
            content: "accumulated".into(),
        })
    })
    .await;
    // 重入 seed：保留原 prompt 与 body，不冲掉。
    m.seed("s1".into(), "SHOULD_NOT_WIN".into()).await;
    let snap = m.snapshot("s1").await.expect("seeded");
    assert_eq!(snap.user_prompt, "original");
    assert_eq!(snap.status_emoji, "👀");
    assert_eq!(snap.body.len(), 1);
}

#[tokio::test]
async fn apply_lazy_seeds_with_empty_prompt() {
    let m = CardStateMap::default();
    // 未 seed 直接 apply：lazy 兜底，prompt=""。
    m.apply("s2", |st| {
        st.body.push(CardElement::Markdown {
            content: "early".into(),
        })
    })
    .await;
    let snap = m.snapshot("s2").await.expect("lazy seeded");
    assert_eq!(snap.user_prompt, "");
    assert_eq!(snap.status_emoji, "👀");
    assert_eq!(snap.body.len(), 1);
}

#[tokio::test]
async fn drop_removes_entry() {
    let m = CardStateMap::default();
    m.seed("s3".into(), "hi".into()).await;
    assert!(m.snapshot("s3").await.is_some());
    m.drop("s3").await;
    assert!(m.snapshot("s3").await.is_none());
    // 幂等：drop 不存在的 entry 不 panic。
    m.drop("s3").await;
}

#[tokio::test]
async fn new_and_lazy_constructors() {
    let a = CardState::new("prompt");
    assert_eq!(a.user_prompt, "prompt");
    assert_eq!(a.status_emoji, "👀");
    assert!(a.body.is_empty());
    let b = CardState::lazy();
    assert_eq!(b.user_prompt, "");
    assert_eq!(b.status_emoji, "👀");
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p router --test card_state_test -- --nocapture`
Expected: FAIL — `router::card_state` 模块不存在（编译错误）。

- [ ] **Step 3: 实现 card_state.rs**

`router/src/card_state.rs`：

```rust
//! 卡片流累积状态（spec §4.1）。纯状态，并行于 `MsgIdMap`：
//! `session_id -> CardState`。渲染与 FSM 在 router.rs / cards.rs。

use feishu::cards::CardElement;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct CardState {
    pub user_prompt: String,
    pub status_emoji: String,
    pub body: Vec<CardElement>,
}

impl CardState {
    /// seed_card 用：记录真实 user_prompt（重渲染引用块用），emoji 👀，空 body。
    pub fn new(user_prompt: &str) -> Self {
        Self {
            user_prompt: user_prompt.into(),
            status_emoji: "👀".into(),
            body: Vec::new(),
        }
    }

    /// 早到事件兜底：prompt=""，emoji 👀，空 body（spec §4.2 lazy seed）。
    pub fn lazy() -> Self {
        Self {
            user_prompt: String::new(),
            status_emoji: "👀".into(),
            body: Vec::new(),
        }
    }
}

#[derive(Default, Clone)]
pub struct CardStateMap {
    inner: Arc<RwLock<HashMap<String, CardState>>>,
}

impl CardStateMap {
    /// 幂等 seed：entry 已存在则保留（防 SpawnAcp 重入冲掉已累积状态）。
    pub async fn seed(&self, session_id: String, user_prompt: String) {
        let mut g = self.inner.write().await;
        g.entry(session_id).or_insert_with(|| CardState::new(&user_prompt));
    }

    /// 无 entry 时 `lazy()` 兜底插入，再对 `&mut CardState` 跑 `f`。
    pub async fn apply<F: FnOnce(&mut CardState)>(&self, session_id: &str, f: F) {
        let mut g = self.inner.write().await;
        let st = g.entry(session_id.to_string()).or_insert_with(CardState::lazy);
        f(st);
    }

    /// 克隆一份给 flush 渲染。
    pub async fn snapshot(&self, session_id: &str) -> Option<CardState> {
        self.inner.read().await.get(session_id).cloned()
    }

    /// session 死亡/通道关时移除（防无界增长）。
    pub async fn drop(&self, session_id: &str) {
        self.inner.write().await.remove(session_id);
    }
}
```

- [ ] **Step 4: 在 router::lib 导出模块**

`router/src/lib.rs` 当前：

```rust
pub mod commands;
pub mod error;
pub mod router;
pub mod state;

pub use commands::{parse_command, Command};
pub use router::{MsgIdMap, Out, RouterHandle};
pub use state::{Mapping, SessionMap};
```

加一行 `pub mod card_state;`（插在 `pub mod state;` 之后）。无需 re-export 类型（测试用全路径 `router::card_state::...`）。

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test -p router --test card_state_test -- --nocapture`
Expected: PASS（4 个测试绿）。

- [ ] **Step 6: 提交**

```bash
git add router/src/card_state.rs router/src/lib.rs router/tests/card_state_test.rs
git commit -m "feat(router): 新增 CardState + CardStateMap 累积状态存储"
```

---

### Task 3: apply_event_to_card + 截断/总量（feishu::cards）

把旧 `apply_event(&mut Card, evt)` 的分支逻辑迁到 `apply_event_to_card(body: &mut Vec<CardElement>, evt, cfg)`：append 进 body（复活 ThinkingDelta/ToolEnd/ToolProgress）、单元素截断（`max_user_text_chars`/`max_tool_output_chars` + `fold_long_output`）、总量兜底（24000 丢旧，Hr 连后一个一起丢）。删除旧 `apply_event`。

**Files:**
- Modify: `feishu/src/cards.rs`（替换 `apply_event` 为 `apply_event_to_card`；`truncate` 助手保留并复用）
- Test: `feishu/tests/cards_test.rs`（追加 append/截断/fold/总量测试）

**Interfaces:**
- Consumes: `feishu::cards::CardConfig`（Task 1）、`feishu::cards::CardElement`、`acp_claude::session::AcpEvent`
- Produces: `pub fn apply_event_to_card(body: &mut Vec<CardElement>, event: &AcpEvent, cfg: &CardConfig)`。删除旧 `pub fn apply_event(card: &mut Card, event: &AcpEvent)`（router 在 Task 5 会改用新签名，Task 3 期间 router 编译会断 —— 本任务只验 feishu 单测，Task 5 修 router）。

**关键语义钉死：**
- TextDelta → `Markdown{content: delta}`；ThinkingDelta → `Div{灰注 "💭 {delta}"}`（用 `push_note` 等价结构）；ToolStart → `Hr` + `Markdown "📖 **{tool_name}** `{args}`"`；ToolEnd → `Div{灰注 "✓ {tool_name} done: {truncated result}"}`；ToolProgress → `Div{灰注 "⏳ {tool_name}: {progress}"}`；Finished → `Markdown "✅ 完成"`；Error → `Markdown "❌ {message}"`；PermissionRequest → no-op（不累积进 root 卡，走独立 SendCard）。
- 截断灰注格式：`(已折叠 N 字)`，N = 溢出字符数（`chars().count()` 算）。
- 总量计数：`Markdown.content` + `Div.text.content` 的字符数之和（`Hr` 计 0，`Button` 不出现在累积 body）。超 24000 时丢旧。
- 丢旧策略：循环 `while total > 24000`：若 body 空 break；取第 0 个元素，若它是 `Hr` 则 `remove(0)` 两次（连后一个），否则 `remove(0)` 一次。

- [ ] **Step 1: 写失败测试**

在 `feishu/tests/cards_test.rs` 追加（顶部已有 `use feishu::cards::{render_permission_card, render_root_card};` —— 需补 `apply_event_to_card`、`CardConfig`、`CardElement` 到 use）。

把 `feishu/tests/cards_test.rs` 顶部 use 改为：

```rust
use feishu::cards::{apply_event_to_card, render_permission_card, render_root_card, CardConfig, CardElement};
use acp_claude::session::AcpEvent;
```

注意：`feishu` crate 的 `[dev-dependencies]` 当前只有 `insta` + Task 1 加的 `toml`。`acp-claude` 是 `feishu` 的正常依赖（`feishu/Cargo.toml` 有 `acp-claude = { path = "../acp-claude" }`），测试可直接 `use acp_claude::...`。✓

追加测试：

```rust
fn cfg() -> CardConfig {
    CardConfig::default()
}

fn cfg_small() -> CardConfig {
    CardConfig {
        max_user_text_chars: 10,
        max_tool_output_chars: 5,
        fold_long_output: true,
        ..cfg()
    }
}

#[test]
fn append_text_delta() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::TextDelta { session_id: "s".into(), delta: "hi".into() },
        &cfg(),
    );
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::Markdown { content } => assert_eq!(content, "hi"),
        other => panic!("expected Markdown, got {other:?}"),
    }
}

#[test]
fn append_revives_thinking_toolend_toolprogress() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta { session_id: "s".into(), delta: "thinking".into() },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolProgress { session_id: "s".into(), tool_name: "Bash".into(), progress: "in_progress".into() },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolEnd { session_id: "s".into(), tool_name: "Bash".into(), result: "ok".into() },
        &cfg(),
    );
    // ThinkingDelta -> Div; ToolProgress -> Div; ToolEnd -> Div（各 1 个元素）
    assert_eq!(body.len(), 3);
    assert!(matches!(body[0], CardElement::Div { .. }));
    assert!(matches!(body[1], CardElement::Div { .. }));
    assert!(matches!(body[2], CardElement::Div { .. }));
}

#[test]
fn tool_start_emits_hr_then_markdown() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart { session_id: "s".into(), tool_name: "Bash".into(), args: serde_json::json!({"cmd":"ls"}) },
        &cfg(),
    );
    assert!(matches!(body[0], CardElement::Hr));
    assert!(matches!(body[1], CardElement::Markdown { .. }));
}

#[test]
fn long_text_is_truncated_with_grey_note() {
    let mut body = vec![];
    let big = "a".repeat(50);
    apply_event_to_card(
        &mut body,
        &AcpEvent::TextDelta { session_id: "s".into(), delta: big.clone() },
        &cfg_small(),
    );
    // TextDelta 截断到 10 + 灰注（已折叠 40 字），共 2 个元素。
    assert_eq!(body.len(), 2);
    match &body[0] {
        CardElement::Markdown { content } => {
            assert_eq!(content.chars().count(), 10);
        }
        other => panic!("expected Markdown, got {other:?}"),
    }
    match &body[1] {
        CardElement::Div { text } => assert!(text.content.contains("已折叠 40 字")),
        other => panic!("expected Div note, got {other:?}"),
    }
}

#[test]
fn fold_disabled_skips_truncation() {
    let mut body = vec![];
    let big = "a".repeat(50);
    let c = CardConfig { fold_long_output: false, ..cfg_small() };
    apply_event_to_card(
        &mut body,
        &AcpEvent::TextDelta { session_id: "s".into(), delta: big },
        &c,
    );
    // 不截断：单元素，全文保留。
    assert_eq!(body.len(), 1);
    match &body[0] {
        CardElement::Markdown { content } => assert_eq!(content.chars().count(), 50),
        other => panic!("expected Markdown, got {other:?}"),
    }
}

#[test]
fn long_toolend_result_truncated() {
    let mut body = vec![];
    let big = "x".repeat(20);
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolEnd { session_id: "s".into(), tool_name: "Bash".into(), result: big },
        &cfg_small(),
    );
    // ToolEnd.result 截断到 5 + 灰注，共 2 个元素。
    assert_eq!(body.len(), 2);
}

#[test]
fn total_budget_drops_oldest() {
    // 总量 > 24000 -> 丢旧。用 max_user_text_chars=4000（default），塞 7 段 4000 字 = 28000 -> 丢到 ≤24000（丢 1 段 -> 24000）。
    let mut body = vec![];
    let c = cfg(); // max_user_text_chars=4000, total budget 24000
    for _ in 0..7 {
        apply_event_to_card(
            &mut body,
            &AcpEvent::TextDelta { session_id: "s".into(), delta: "a".repeat(4000) },
            &c,
        );
    }
    // 7*4000=28000 > 24000 -> 丢最旧 1 段 -> 6 段 *4000 = 24000 (==budget, 不再丢).
    assert_eq!(body.len(), 6);
}

#[test]
fn total_budget_drops_hr_with_following_element() {
    // 最旧是 Hr -> 连后一个一起丢。
    let mut body = vec![];
    let c = cfg();
    // 先 push 一个 Hr + 一个 text，再 push 大量 text 触发总量。
    apply_event_to_card(
        &mut body,
        &AcpEvent::ToolStart { session_id: "s".into(), tool_name: "Bash".into(), args: serde_json::json!({}) },
        &c,
    ); // body = [Hr, Markdown]
    for _ in 0..7 {
        apply_event_to_card(
            &mut body,
            &AcpEvent::TextDelta { session_id: "s".into(), delta: "a".repeat(4000) },
            &c,
        );
    } // body = [Hr, Markdown, M, M, M, M, M, M, M] -> Hr 最旧
    // 总量超 24000 -> 丢 Hr + 其后 1 个 Markdown（共 2 个），剩余 7-1=6 段 text + 原 Markdown? 需算:
    //   元素: [Hr, Markdown(ToolStart的), M, M, M, M, M, M, M] = 1 Hr + 8 Markdown
    //   字符: 8*4000 = 32000 -> 丢 Hr+第1个M -> 7*4000=28000 -> 继续 -> 丢第2个M -> 6*4000=24000 -> 停.
    //   但丢 Hr 时连后一个 -> 第一次丢 [Hr, Markdown(ToolStart)] -> 剩 7 M = 28000 -> 再丢 1 M -> 6 M = 24000.
    //   最终 body.len() = 6 (6 个 Markdown).
    assert_eq!(body.len(), 6);
    // 最旧的 Hr 已被连带丢掉.
    assert!(matches!(body[0], CardElement::Markdown { .. }));
}

#[test]
fn permission_request_is_noop_for_body() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::PermissionRequest { session_id: "s".into(), request_id: "r".into(), tool_name: "Bash".into(), args: serde_json::json!({}) },
        &cfg(),
    );
    assert!(body.is_empty(), "PermissionRequest 不累积进 root body");
}

#[test]
fn finished_and_error_append_markdown() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::Finished { session_id: "s".into() },
        &cfg(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::Error { session_id: "s".into(), message: "boom".into(), terminal: false },
        &cfg(),
    );
    assert_eq!(body.len(), 2);
    match &body[0] {
        CardElement::Markdown { content } => assert_eq!(content, "✅ 完成"),
        other => panic!("expected Finished Markdown, got {other:?}"),
    }
    match &body[1] {
        CardElement::Markdown { content } => assert_eq!(content, "❌ boom"),
        other => panic!("expected Error Markdown, got {other:?}"),
    }
}
```

注意 `cfg_small()` 里用了 `..cfg()` —— `CardConfig` 需支持 struct update syntax，所有字段 `pub` 已满足（Task 1）。✓

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p feishu --lib --tests apply_ finished_ long_ fold_ total_ permission_ append_revives tool_start_ -- --nocapture`
Expected: FAIL — `apply_event_to_card` 不存在（编译错误 `cannot find function apply_event_to_card`）。

- [ ] **Step 3: 实现 — 替换 apply_event 为 apply_event_to_card**

在 `feishu/src/cards.rs`，删除旧 `pub fn apply_event(card: &mut Card, event: &AcpEvent) { ... }` 整段（181-211 行），替换为：

```rust
/// 把一个事件累积进 body（spec §4.2/§7）。复活 ThinkingDelta/ToolEnd/ToolProgress。
/// 单元素截断（max_user_text_chars / max_tool_output_chars + fold_long_output）
/// + 总量兜底（24000 丢旧，Hr 连后一个一起丢）。PermissionRequest 不累积（走独立 SendCard）。
pub fn apply_event_to_card(body: &mut Vec<CardElement>, event: &AcpEvent, cfg: &CardConfig) {
    match event {
        AcpEvent::TextDelta { delta, .. } => {
            push_text_truncated(body, delta, cfg.max_user_text_chars, cfg.fold_long_output);
        }
        AcpEvent::ThinkingDelta { delta, .. } => {
            body.push(note_element(format!("💭 {delta}")));
        }
        AcpEvent::ToolStart { tool_name, args, .. } => {
            body.push(CardElement::Hr);
            push_text_truncated(
                body,
                &format!("📖 **{tool_name}** `{args}`"),
                cfg.max_user_text_chars,
                cfg.fold_long_output,
            );
        }
        AcpEvent::ToolEnd { tool_name, result, .. } => {
            let (text, note) = truncate_with_note(result, cfg.max_tool_output_chars, cfg.fold_long_output);
            body.push(note_element(format!("✓ {tool_name} done: {text}")));
            if let Some(n) = note {
                body.push(note_element(format!("（已折叠 {n} 字）")));
            }
        }
        AcpEvent::ToolProgress { tool_name, progress, .. } => {
            body.push(note_element(format!("⏳ {tool_name}: {progress}")));
        }
        AcpEvent::Finished { .. } => body.push(CardElement::Markdown {
            content: "✅ 完成".into(),
        }),
        AcpEvent::Error { message, .. } => body.push(CardElement::Markdown {
            content: format!("❌ {message}"),
        }),
        AcpEvent::PermissionRequest { .. } => {} // 独立 SendCard，不累积
    }
    enforce_total_budget(body, cfg);
}

/// 截断文本到 `limit` 字符；超限则返回 (截断文本, Some(溢出字符数))。
fn truncate_with_note(s: &str, limit: usize, fold: bool) -> (String, Option<usize>) {
    if !fold {
        return (s.to_string(), None);
    }
    let count = s.chars().count();
    if count <= limit {
        return (s.to_string(), None);
    }
    let truncated: String = s.chars().take(limit).collect();
    (truncated, Some(count - limit))
}

/// push 一段 Markdown 文本，必要时截断 + 追加灰注。
fn push_text_truncated(body: &mut Vec<CardElement>, text: &str, limit: usize, fold: bool) {
    let (content, note) = truncate_with_note(text, limit, fold);
    body.push(CardElement::Markdown { content });
    if let Some(n) = note {
        body.push(note_element(format!("（已折叠 {n} 字）")));
    }
}

/// 构造一个灰注 Div 元素（notation size + grey）。
fn note_element(content: String) -> CardElement {
    CardElement::Div {
        text: DivText {
            tag: "plain_text".into(),
            content,
            text_size: Some("notation".into()),
            text_color: Some("grey".into()),
        },
    }
}

/// 总量兜底（spec §7）：body 累积字符 > 24000 -> 丢最旧；最旧是 Hr 则连后一个一起丢。
fn enforce_total_budget(body: &mut Vec<CardElement>, _cfg: &CardConfig) {
    const TOTAL_BUDGET: usize = 24000;
    while total_chars(body) > TOTAL_BUDGET {
        if body.is_empty() {
            break;
        }
        // 最旧是 Hr -> 连后一个一起丢（不留悬空分隔线）。
        let drop_two = matches!(body.first(), Some(CardElement::Hr));
        body.remove(0);
        if drop_two && !body.is_empty() {
            body.remove(0);
        }
    }
}

fn total_chars(body: &[CardElement]) -> usize {
    body.iter()
        .map(|e| match e {
            CardElement::Markdown { content } => content.chars().count(),
            CardElement::Div { text } => text.content.chars().count(),
            _ => 0,
        })
        .sum()
}
```

旧的 `fn truncate(s: &str, n: usize) -> String`（213-221 行）现在被 `truncate_with_note` 取代。删除旧 `truncate`（若仍有其他引用会编译失败 —— grep 确认仅旧 apply_event 用过，已删）。运行 `cargo build -p feishu` 确认无残留引用。

注意灰注文案不一致问题：上面 `ToolEnd` 用了 `（已折叠 {n} 字）`（全角括号），而 `push_text_truncated` 用 `（已折叠 {n} 字）` —— 统一为全角括号。Step 1 测试里 `long_text_is_truncated_with_grey_note` 断言 `text.content.contains("已折叠 40 字")`（不含括号，子串匹配，两种括号都过）。`total_budget` 测试不断言灰注文案。✓ 一致。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p feishu --lib --tests -- --nocapture`
Expected: PASS（所有 feishu 单测绿，含新 append/截断/fold/总量 + 现有快照）。

注意：此时 `cargo build --workspace` 会失败 —— `router/src/router.rs:5` 仍 `use feishu::cards::apply_event` 且 `:150/:169` 调用旧 `apply_event(&mut card, event)`。**这是预期的**（router 修在 Task 5）。本步只验 feishu 单测。

- [ ] **Step 5: 提交**

```bash
git add feishu/src/cards.rs feishu/tests/cards_test.rs
git commit -m "feat(feishu): apply_event_to_card 累积 body + 截断/fold + 24000 总量兜底"
```

---

### Task 4: render_accumulated_card（feishu::cards）

新增 `render_accumulated_card`：从累积状态构建完整卡（header `{emoji} Claude Code` + theme + 引用块 `> {user_prompt}` + 分隔线 + body 各元素 + 灰注 `msg_id: {session_id}`）。`render_root_card` 保留（cards_test 快照依赖），但改为 `render_accumulated_card` 的空 body 薄封装（消除重复 + 让 theme 流到 seed 卡）。

**Files:**
- Modify: `feishu/src/cards.rs`（`render_root_card` 改薄封装 + 新增 `render_accumulated_card`）
- Test: `feishu/tests/cards_test.rs`（追加 render_accumulated_card 结构断言）

**Interfaces:**
- Consumes: `feishu::cards::{Card, CardElement, CardConfig}`（已存在）
- Produces: `pub fn render_accumulated_card(user_prompt: &str, session_id: &str, status_emoji: &str, body: &[CardElement], theme: &str) -> Card`。

- [ ] **Step 1: 写失败测试**

在 `feishu/tests/cards_test.rs` 追加（use 行已有 `render_root_card`，补 `render_accumulated_card`）：

```rust
#[test]
fn render_accumulated_card_structure() {
    use feishu::cards::{render_accumulated_card, CardElement};
    let body = vec![
        CardElement::Markdown { content: "hello".into() },
        CardElement::Hr,
        CardElement::Markdown { content: "world".into() },
    ];
    let card = render_accumulated_card("重构 foo", "msg_9", "🚧", &body, "orange");
    let s = serde_json::to_string(&card).unwrap();
    // header title 含 emoji + "Claude Code"，template=orange
    assert!(s.contains("🚧 Claude Code"));
    assert!(s.contains("\"template\":\"orange\""));
    // 引用块
    assert!(s.contains("> 重构 foo"));
    // body 两段 text 都在
    assert!(s.contains("hello"));
    assert!(s.contains("world"));
    // footer msg_id
    assert!(s.contains("msg_id: msg_9"));
}

#[test]
fn render_accumulated_card_empty_body_matches_seed() {
    use feishu::cards::render_accumulated_card;
    let card = render_accumulated_card("hi", "msg_1", "👀", &[], "blue");
    let s = serde_json::to_string(&card).unwrap();
    assert!(s.contains("👀 Claude Code"));
    assert!(s.contains("> hi"));
    assert!(s.contains("msg_id: msg_1"));
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p feishu render_accumulated_card_structure render_accumulated_card_empty_body_matches_seed -- --nocapture`
Expected: FAIL — `render_accumulated_card` 不存在。

- [ ] **Step 3: 实现**

在 `feishu/src/cards.rs`，把 `render_root_card`（129-135 行）替换为 `render_accumulated_card` + 薄封装：

```rust
/// 从累积状态构建完整卡（spec §4.3）：
/// header(`{emoji} Claude Code`, theme) + 引用块(`> {user_prompt}`) + 分隔线
/// + body 各元素 + footer 灰注(`msg_id: {session_id}`)。
pub fn render_accumulated_card(
    user_prompt: &str,
    session_id: &str,
    status_emoji: &str,
    body: &[CardElement],
    theme: &str,
) -> Card {
    let mut card = Card::new(&format!("{status_emoji} Claude Code"), theme);
    card.push_text(format!("> {user_prompt}"));
    card.push_divider();
    for el in body {
        card.body.elements.push(el.clone());
    }
    card.push_note(format!("msg_id: {session_id}"));
    card
}

/// seed 时的初始卡构建器（不再被每个事件调用）。空 body 薄封装。
/// 保留供 cards_test 快照；theme 固定 "blue" 以保持快照不变。
pub fn render_root_card(user_prompt: &str, msg_id: &str, status_emoji: &str) -> Card {
    render_accumulated_card(user_prompt, msg_id, status_emoji, &[], "blue")
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p feishu --lib --tests -- --nocapture`
Expected: PASS（新结构测试绿；`root_card_initial_snapshot` / `root_card_after_text_delta_snapshot` 快照不变 —— `render_root_card` 行为等价）。

若 insta 快照因序列化字段顺序微变而失配：先 `cargo insta review` 接受新快照（仅当差异是结构等价的字段重排，非内容丢失）。若内容真变了，回退检查 `render_accumulated_card` 实现。

- [ ] **Step 5: 提交**

```bash
git add feishu/src/cards.rs feishu/tests/cards_test.rs feishu/tests/snapshots/
git commit -m "feat(feishu): render_accumulated_card 从累积状态构建整卡"
```

---

### Task 5: RouterHandle 卡方法 + apply_event_to_out 重写

router 接上 CardState：`seed_card`/`apply_event`（纯状态 + FSM emoji + 调 `apply_event_to_card`）/`flush_card`（快照 → `render_accumulated_card` → `Out::UpdateCard`）/`drop_card`。新增 `new_with_card_config`。重写 `apply_event_to_out` 为 `apply_event + flush_card` 同步薄封装（terminal 臂额外 `remove_by_session` + `drop_card`；PermissionRequest 臂不变）。**现有 router_test/e2e_test/terminal_error_test/permission_test 零改动通过。**

**Files:**
- Modify: `router/src/router.rs`（`RouterHandle` 加字段 + 方法；重写 `apply_event_to_out`；改 import）
- Test: `router/tests/card_state_test.rs`（追加累积/FSM/同步语义测试）；`router/tests/terminal_error_test.rs`（追加 terminal 保留 transcript 测试）

**Interfaces:**
- Consumes: `router::card_state::{CardState, CardStateMap}`（Task 2）、`feishu::cards::{apply_event_to_card, render_accumulated_card, CardConfig}`（Task 1/3/4）、`crate::state::SessionMap::remove_by_session`（已存在）
- Produces:
  - `RouterHandle { map, tx, msgid, card_states: CardStateMap, card_cfg: feishu::cards::CardConfig }`
  - `pub fn new_with_card_config(map: SessionMap, card_cfg: feishu::cards::CardConfig) -> (Self, mpsc::Receiver<Out>)`
  - `pub async fn seed_card(&self, session_id: String, user_prompt: String)` — 幂等 seed
  - `pub async fn apply_event(&self, session_id: &str, event: &AcpEvent)` — 纯状态（FSM emoji + apply_event_to_card），不发 Out
  - `pub async fn flush_card(&self, session_id: &str)` — 快照 → render_accumulated_card → `tx.send(Out::UpdateCard{...})`；无 CardState 则 no-op
  - `pub async fn drop_card(&self, session_id: &str)` — `card_states.drop`
  - `apply_event_to_out` 语义改为：PermissionRequest→SendCard（不变）；terminal Error→apply_event+flush+remove_by_session+drop_card；其余→apply_event+flush_card

- [ ] **Step 1: 写失败测试（累积 + FSM + 同步语义）**

在 `router/tests/card_state_test.rs` 追加（顶部 use 补 `router::router::{Out, RouterHandle}`、`acp_claude::session::AcpEvent`、`std::time::Duration`）：

```rust
use acp_claude::session::AcpEvent;
use feishu::cards::CardConfig;
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::time::Duration;

#[tokio::test]
async fn apply_event_accumulates_without_emitting_out() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router.seed_card("s1".into(), "hi".into()).await;
    // 连发多个流式事件：apply_event 期间无 Out。
    router
        .apply_event(
            "s1",
            &AcpEvent::TextDelta { session_id: "s1".into(), delta: "a".into() },
        )
        .await;
    router
        .apply_event(
            "s1",
            &AcpEvent::ThinkingDelta { session_id: "s1".into(), delta: "think".into() },
        )
        .await;
    router
        .apply_event(
            "s1",
            &AcpEvent::ToolEnd { session_id: "s1".into(), tool_name: "Bash".into(), result: "ok".into() },
        )
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), out_rx.recv())
            .await
            .is_err(),
        "apply_event 不得发 Out"
    );
    // flush_card 产 1 张 UpdateCard，正文含全部事件渲染，emoji 🚧。
    router.flush_card("s1").await;
    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::UpdateCard { session_id, card } => {
            assert_eq!(session_id, "s1");
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("a"), "含 TextDelta: {s}");
            assert!(s.contains("think"), "含 ThinkingDelta: {s}");
            assert!(s.contains("Bash"), "含 ToolEnd: {s}");
            assert!(s.contains("🚧"), "emoji 🚧: {s}");
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
}

#[tokio::test]
async fn fsm_eyes_to_construction_to_done() {
    let map = SessionMap::new();
    let (router, _) = RouterHandle::new(map);
    router.seed_card("s2".into(), "p".into()).await;
    // seed = 👀
    router.flush_card("s2").await; // 不验 Out，只驱动状态机内部（flush 不改 emoji）
    router
        .apply_event(
            "s2",
            &AcpEvent::TextDelta { session_id: "s2".into(), delta: "x".into() },
        )
        .await;
    router.flush_card("s2").await;
    // 验证 🚧：用 apply_event_to_out 同步路径产卡断言 emoji
    let (router2, mut out2) = RouterHandle::new(SessionMap::new());
    router2.seed_card("s2".into(), "p".into()).await;
    router2
        .apply_event(
            "s2",
            &AcpEvent::TextDelta { session_id: "s2".into(), delta: "x".into() },
        )
        .await;
    router2.flush_card("s2").await;
    let o = tokio::time::timeout(Duration::from_millis(200), out2.recv())
        .await
        .unwrap()
        .unwrap();
    let s = serde_json::to_string(&o).unwrap();
    assert!(s.contains("🚧"));
    // Finished -> ✅
    let (router3, mut out3) = RouterHandle::new(SessionMap::new());
    router3.seed_card("s3".into(), "p".into()).await;
    router3
        .apply_event_to_out(
            "s3".into(),
            &AcpEvent::Finished { session_id: "s3".into() },
        )
        .await;
    let o3 = tokio::time::timeout(Duration::from_millis(200), out3.recv())
        .await
        .unwrap()
        .unwrap();
    let s3 = serde_json::to_string(&o3).unwrap();
    assert!(s3.contains("✅"));
}

#[tokio::test]
async fn fsm_terminal_error_marks_red() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    router.seed_card("s4".into(), "p".into()).await;
    router
        .apply_event_to_out(
            "s4".into(),
            &AcpEvent::Error { session_id: "s4".into(), message: "dead".into(), terminal: true },
        )
        .await;
    let o = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let s = serde_json::to_string(&o).unwrap();
    assert!(s.contains("❌"));
}

#[tokio::test]
async fn new_with_card_config_uses_theme() {
    // 自定义 theme_color 流到渲染卡。
    let cfg = CardConfig { theme_color: "orange".into(), ..CardConfig::default() };
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new_with_card_config(map, cfg);
    router.seed_card("s5".into(), "hi".into()).await;
    router
        .apply_event_to_out(
            "s5".into(),
            &AcpEvent::TextDelta { session_id: "s5".into(), delta: "x".into() },
        )
        .await;
    let o = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let s = serde_json::to_string(&o).unwrap();
    assert!(s.contains("\"template\":\"orange\""));
}
```

- [ ] **Step 2: 写失败测试（terminal 保留 transcript）**

在 `router/tests/terminal_error_test.rs` 追加：

```rust
#[tokio::test]
async fn terminal_error_preserves_pre_death_transcript() {
    let map = SessionMap::new();
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    map.insert(key.clone(), Mapping::active("s1")).await.unwrap();
    let (router, mut out_rx) = RouterHandle::new(map.clone());

    // 累积若干事件（死前 transcript）。
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::TextDelta { session_id: "s1".into(), delta: "step1".into() },
        )
        .await;
    let _ = tokio::time::timeout(Duration::from_millis(100), out_rx.recv()).await;
    router
        .apply_event_to_out(
            "s1".into(),
            &AcpEvent::ToolEnd { session_id: "s1".into(), tool_name: "Bash".into(), result: "step2".into() },
        )
        .await;
    let _ = tokio::time::timeout(Duration::from_millis(100), out_rx.recv()).await;

    // terminal Error：死前 transcript 必须保留 + 错误正文。
    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "s1".into(),
            message: "agent crashed".into(),
            terminal: true,
        })
        .await;

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    match out {
        Out::UpdateCard { session_id, card } => {
            assert_eq!(session_id, "s1");
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains('❌'), "❌ emoji: {s}");
            assert!(s.contains("step1"), "死前 TextDelta 保留: {s}");
            assert!(s.contains("step2"), "死前 ToolEnd 保留: {s}");
            assert!(s.contains("agent crashed"), "错误正文: {s}");
        }
        other => panic!("expected UpdateCard, got {other:?}"),
    }
    assert!(map.get(&key).await.is_none(), "terminal 必清 mapping");
}
```

- [ ] **Step 3: 运行测试验证失败**

Run: `cargo test -p router --test card_state_test --test terminal_error_test -- --nocapture`
Expected: FAIL — `RouterHandle::new_with_card_config`/`seed_card`/`apply_event`/`flush_card` 不存在；`apply_event_to_out` 仍是旧行为（terminal_error_preserves 测试会失败：旧实现重建空卡，无 step1/step2）。

- [ ] **Step 4: 实现 — RouterHandle 字段 + 构造器 + 卡方法 + 重写 apply_event_to_out**

`router/src/router.rs` 改动：

**4a. 改 import（顶部第 4-6 行）：**

当前：
```rust
use feishu::cards::{
    apply_event, render_dead_session_card, render_permission_card, render_root_card,
};
```
改为：
```rust
use feishu::cards::{
    apply_event_to_card, render_accumulated_card, render_dead_session_card,
    render_permission_card,
};
use feishu::cards::CardConfig;
```
（删除 `apply_event` 与 `render_root_card` 导入；`render_root_card` 不再被 router 用 —— dispatch_out 的初始卡在 Task 7 改用 `render_accumulated_card`。Task 5 期间 dispatch_out 仍 `use feishu::cards::render_root_card` 在 `src/run.rs`，不受 router 影响。）

**4b. RouterHandle 加字段（第 41-45 行）：**

当前：
```rust
pub struct RouterHandle {
    map: SessionMap,
    tx: mpsc::Sender<Out>,
    msgid: MsgIdMap,
}
```
改为：
```rust
pub struct RouterHandle {
    map: SessionMap,
    tx: mpsc::Sender<Out>,
    msgid: MsgIdMap,
    card_states: crate::card_state::CardStateMap,
    card_cfg: CardConfig,
}
```

**4c. Clone impl 加字段（第 47-55 行）：**

```rust
impl Clone for RouterHandle {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            tx: self.tx.clone(),
            msgid: self.msgid.clone(),
            card_states: self.card_states.clone(),
            card_cfg: self.card_cfg.clone(),
        }
    }
}
```

**4d. 构造器（第 58-68 行）+ 新增 new_with_card_config：**

当前 `new(map)` 用 `MsgIdMap::default()`。改为：

```rust
    pub fn new(map: SessionMap) -> (Self, mpsc::Receiver<Out>) {
        Self::new_with_card_config(map, CardConfig::default())
    }

    pub fn new_with_card_config(
        map: SessionMap,
        card_cfg: CardConfig,
    ) -> (Self, mpsc::Receiver<Out>) {
        let (tx, rx) = mpsc::channel(256);
        (
            Self {
                map,
                tx,
                msgid: MsgIdMap::default(),
                card_states: crate::card_state::CardStateMap::default(),
                card_cfg,
            },
            rx,
        )
    }
```

**4e. 新增卡方法**（插在 `record_root_msg_id` 之后，约第 78 行后）：

```rust
    /// seed_card：SpawnAcp 臂发完 root 卡后调用（dispatch_out）。
    /// 幂等：已存在则保留（防 SpawnAcp 重入冲掉已累积状态）。spec §4.2。
    pub async fn seed_card(&self, session_id: String, user_prompt: String) {
        self.card_states.seed(session_id, user_prompt).await;
    }

    /// apply_event：纯状态变更（FSM emoji + apply_event_to_card append/截断/总量）。
    /// 不发 Out。session 无 CardState 时 lazy seed（prompt="" 兜底）。spec §4.2。
    pub async fn apply_event(&self, session_id: &str, event: &AcpEvent) {
        let cfg = &self.card_cfg;
        self.card_states
            .apply(session_id, |st| {
                // FSM（spec §5）
                let next = next_emoji(&st.status_emoji, event);
                if let Some(e) = next {
                    st.status_emoji = e.into();
                }
                apply_event_to_card(&mut st.body, event, cfg);
            })
            .await;
    }

    /// flush_card：快照 → render_accumulated_card → Out::UpdateCard。
    /// 无 CardState 则 no-op。spec §4.2。节流契约保证 flush 只在 debounce 到点或
    /// Finished/terminal 即时被调，故不维护 dirty flag。
    pub async fn flush_card(&self, session_id: &str) {
        let Some(st) = self.card_states.snapshot(session_id).await else {
            return;
        };
        let card = render_accumulated_card(
            &st.user_prompt,
            session_id,
            &st.status_emoji,
            &st.body,
            &self.card_cfg.theme_color,
        );
        let _ = self
            .tx
            .send(Out::UpdateCard {
                session_id: session_id.to_string(),
                card: serde_json::to_value(&card).unwrap(),
            })
            .await;
    }

    /// drop_card：session 死亡/通道关时清 CardState（防无界增长）。spec §4.2。
    pub async fn drop_card(&self, session_id: &str) {
        self.card_states.drop(session_id).await;
    }
```

**4f. FSM 辅助函数**（文件级私有，插在 `extract_session_id` 附近）：

```rust
/// status emoji FSM（spec §5）。返回 Some(新emoji) 表示转移；None 表示不变。
/// seed=👀；首个流式事件 -> 🚧；Finished -> ✅；terminal Error -> ❌；
/// 已 🚧/✅/❌ 不回退 👀。
fn next_emoji(current: &str, event: &AcpEvent) -> Option<&'static str> {
    match event {
        AcpEvent::Finished { .. } => Some("✅"),
        AcpEvent::Error { terminal: true, .. } => Some("❌"),
        AcpEvent::TextDelta { .. }
        | AcpEvent::ThinkingDelta { .. }
        | AcpEvent::ToolStart { .. }
        | AcpEvent::ToolProgress { .. }
        | AcpEvent::ToolEnd { .. }
        | AcpEvent::Error { terminal: false, .. } => {
            if current == "👀" {
                Some("🚧")
            } else {
                None
            }
        }
        AcpEvent::PermissionRequest { .. } => None,
    }
}
```

**4g. 重写 apply_event_to_out（第 139-203 行）：**

当前实现是「terminal 臂重建空卡 + 共享臂 render_root_card+apply_event + Permission 臂 + `_=>{}`」。替换为：

```rust
    pub async fn apply_event_to_out(&self, session_id: String, event: &AcpEvent) {
        match event {
            AcpEvent::PermissionRequest {
                session_id,
                request_id,
                tool_name,
                args,
            } => {
                let card = render_permission_card(session_id, request_id, tool_name, args);
                let Some(key) = self.map.lookup_key_by_session(session_id).await else {
                    tracing::warn!(%session_id, "no SessionKey for permission request; dropping card");
                    return;
                };
                let _ = self
                    .tx
                    .send(Out::SendCard {
                        key,
                        card: serde_json::to_value(&card).unwrap(),
                        msg_id: None,
                    })
                    .await;
            }
            AcpEvent::Error { terminal: true, .. } => {
                // terminal Error 并入累积模型（spec §8）：apply_event（置 ❌ + append
                // 错误正文，保留死前 transcript）→ flush_card → remove_by_session → drop_card。
                self.apply_event(&session_id, event).await;
                self.flush_card(&session_id).await;
                self.map.remove_by_session(&session_id).await;
                self.drop_card(&session_id).await;
            }
            _ => {
                // 流式事件 + Finished + 非 terminal Error：apply_event（状态）+ flush_card（同步出卡）。
                self.apply_event(&session_id, event).await;
                self.flush_card(&session_id).await;
            }
        }
    }
```

注意：`session_id` 在 terminal 臂与 Permission 臂是 `&String`（match 绑定 `event` 的字段引用），`self.apply_event(&session_id, event)` 第一参要 `&str` —— `&session_id` 是 `&&String`，需 `session_id.as_str()` 或 `&**session_id`。用 `self.apply_event(session_id, event).await`（`&String` 可 deref coercion 到 `&str`？`apply_event(&self, session_id: &str, ...)` 传 `session_id: &String` —— `&String` 不自动转 `&str` 除非显式。用 `self.apply_event(session_id, event)` 其中 `session_id` 是 `&String`（match 绑定）—— 传 `&String` 给 `&str` 形参：Rust 自动 deref coercion `&String -> &str` 成立。✓ 但 terminal 臂里 `session_id` 绑定的是 `&String`（来自 `AcpEvent::Error{session_id: String, ..}` 的 match，绑定 `session_id` 是 `&String`）。`self.apply_event(session_id, event)` —— 形参 `&str`，实参 `&String`，deref coercion ✓。`self.flush_card(&session_id)` —— 形参 `&str`，`&session_id` 是 `&&String`，需 `session_id.as_str()`。为避免歧义，统一用 `self.apply_event(session_id.as_str(), event).await` 与 `self.flush_card(session_id.as_str()).await`。

修正后 terminal 臂：
```rust
                self.apply_event(session_id.as_str(), event).await;
                self.flush_card(session_id.as_str()).await;
                self.map.remove_by_session(session_id).await;
                self.drop_card(session_id.as_str()).await;
```
（`remove_by_session(&self, session_id: &str)` 传 `session_id: &String` —— deref coercion ✓。）

共享臂：
```rust
                self.apply_event(session_id.as_str(), event).await;
                self.flush_card(session_id.as_str()).await;
```
但共享臂的 `session_id` 是函数参数 `session_id: String`（move 进函数），match `_` 不绑定 event 字段。共享臂里 `session_id` 是 `String`（函数参）。`self.apply_event(&session_id, event)` —— `&String` → `&str` ✓。或 `session_id.as_str()`。用 `&session_id`。但共享臂末尾 `session_id` 被 move 进 `flush_card`? 不，`flush_card(&str)` 只借用。OK。为统一，共享臂用 `&session_id`（`&String` deref 到 `&str`）。

为彻底消除歧义，函数体内统一用 `session_id.as_str()` 或 `&session_id`。实现者按编译器提示修。

**4h. dispatch_acp_event 不变（仍调 apply_event_to_out）：** 第 134-137 行保持。pump 在 Task 6 改为调 apply_event + flush_card，不再走 dispatch_acp_event；但 dispatch_acp_event 保留（测试 + 即时入口用）。spec §6「dispatch_acp_event 改为调 apply_event 不发 Out」与本计划「保留 dispatch_acp_event 调 apply_event_to_out（同步）」的偏差：见下方「Spec 偏差记录」。

**Spec 偏差记录（写进提交信息 body 或代码注释）：** spec §6 说「dispatch_acp_event 改为调 apply_event（不发 Out）」，但 spec §9 要求「router_test/e2e_test/terminal_error_test 零改动通过」—— 这两个测试调 `dispatch_acp_event(TextDelta/Error)` 并断言立即收到 UpdateCard。若 dispatch_acp_event 不发 Out，这两个测试必挂。故本计划保留 `dispatch_acp_event → apply_event_to_out`（同步 flush），仅把 **pump** 从 dispatch_acp_event 改为 apply_event+debounce+flush_card（Task 6）。契约（§6 的真意：pump 路径不每事件同步发卡）满足，§9 零改动通过。在 `apply_event_to_out` 的 doc comment 注明此偏差。

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test -p router --test card_state_test -- --nocapture`
Expected: PASS（累积/FSM/同步/theme 测试绿）。

Run: `cargo test -p router --test terminal_error_test -- --nocapture`
Expected: PASS（含新 terminal_error_preserves_pre_death_transcript）。

Run: `cargo test -p router --test router_test --test e2e_test --test permission_test -- --nocapture`
Expected: PASS（零改动回归 —— apply_event_to_out 同步语义保留）。

- [ ] **Step 6: 全量编译确认**

Run: `cargo build --workspace`
Expected: 编译通过。`src/run.rs` 仍 `use feishu::cards::render_root_card`（未改，Task 7 改）—— `render_root_card` 仍在 cards.rs（Task 4 保留为薄封装），✓ 不报错。pump 仍调 `dispatch_acp_event`（Task 6 改）—— 编译过，行为 Task 6 修。

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 0 警告。若有 `render_root_card` 未使用告警（若 Task 7 前 dispatch_out 暂未改）—— Task 5 期间 dispatch_out 仍用 render_root_card，不会未使用。✓

- [ ] **Step 7: 提交**

```bash
git add router/src/router.rs router/tests/card_state_test.rs router/tests/terminal_error_test.rs
git commit -m "feat(router): seed_card/apply_event/flush_card/drop_card + apply_event_to_out 同步重写"
```

---

### Task 6: pump 节流重写（interval + dirty）

重写 `spawn_acp_pump`（`src/run.rs`）为 `tokio::select!` 循环：流式事件 `apply_event`（状态）+ 标脏，`tokio::time::interval(150ms)` tick 时若脏则 `flush_card`；Finished/terminal Error/PermissionRequest 走即时 `apply_event_to_out`；通道关闭 `drop_card` + 退出。`spawn_acp_pump` 改 `pub` 供测试。

**Files:**
- Modify: `src/run.rs:399-415`（重写 `spawn_acp_pump`，改 `pub`）
- Test: `tests/pump_unit_test.rs`（新建，合成 rx 断言节流契约，无 fake-claude）

**Interfaces:**
- Consumes: `RouterHandle::{apply_event, flush_card, drop_card, apply_event_to_out}`（Task 5）、`AcpEvent`、`tokio::time::interval`
- Produces: `pub fn spawn_acp_pump(rx: Arc<Mutex<mpsc::Receiver<AcpEvent>>>, router: RouterHandle, session_id: String)`（签名不变，仅 `pub` + 函数体重写）

**节流契约（spec §6 验收）：**
1. 事件即时累积（`apply_event` 同步改状态）。
2. 出站 UpdateCard 在 150ms 内至多一次（interval period=150ms，每 tick ≤1 flush）。
3. Finished/terminal 立即出最终态（即时路径不等 tick）。
4. terminal Error 后 `remove_by_session` + `drop_card` + pump退出。
5. 通道关闭 `drop_card` + 退出。

- [ ] **Step 1: 写失败测试（节流契约，合成 rx）**

`tests/pump_unit_test.rs`：

```rust
//! pump 节流契约单测（spec §6）。合成 mpsc Receiver 喂事件，断言：
//! 5 个 TextDelta 合并成 1 个 UpdateCard（≤1/150ms）；Finished 立即再发 1 个（✅）；
//! terminal Error 立即发 1 个（❌）+ 清 mapping + 退出；通道关闭 drop_card + 退出。
//! 不依赖 fake-claude 二进制。

use acp_claude::session::AcpEvent;
use router::router::{Out, RouterHandle};
use router::state::{Mapping, SessionMap};
use feishu::events::SessionKey;
use sebas::run::spawn_acp_pump;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

async fn new_pump() -> (RouterHandle, tokio::sync::Mutex<mpsc::Receiver<Out>>, Arc<tokio::sync::Mutex<mpsc::Receiver<AcpEvent>>>, mpsc::Sender<AcpEvent>) {
    let map = SessionMap::new();
    let (router, out_rx) = RouterHandle::new(map);
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    let out_rx = tokio::sync::Mutex::new(out_rx);
    (router, out_rx, rx, tx)
}

#[tokio::test]
async fn five_deltas_merge_into_one_updatecard() {
    let (router, out_rx, rx, tx) = new_pump().await;
    router.seed_card("s1".into(), "hi".into()).await;
    spawn_acp_pump(rx, router.clone(), "s1".into());

    // 连发 5 个 TextDelta（全部落在第一个 150ms tick 之前）。
    for i in 0..5 {
        tx.send(AcpEvent::TextDelta { session_id: "s1".into(), delta: format!("chunk{i} ") })
            .await
            .unwrap();
    }

    // 第一个 tick（≤150ms + 抖动）产 1 个 UpdateCard，含 5 段。
    let out_rx = out_rx.lock().await;
    let first = tokio::time::timeout(Duration::from_millis(400), out_rx.recv())
        .await
        .expect("first UpdateCard within 400ms")
        .expect("channel open");
    let s = serde_json::to_string(&first).unwrap();
    for i in 0..5 {
        assert!(s.contains(&format!("chunk{i}")), "chunk{i} in card: {s}");
    }
    assert!(s.contains("🚧"));

    // 150ms 窗口内无第二个 UpdateCard（Finished 未到）。
    let second = tokio::time::timeout(Duration::from_millis(120), out_rx.recv()).await;
    assert!(second.is_err(), "150ms 窗口内不得发第二个 UpdateCard");
}

#[tokio::test]
async fn finished_flushes_immediately_after_stream() {
    let (router, out_rx, rx, tx) = new_pump().await;
    router.seed_card("s2".into(), "p".into()).await;
    spawn_acp_pump(rx, router.clone(), "s2".into());

    tx.send(AcpEvent::TextDelta { session_id: "s2".into(), delta: "x".into() })
        .await
        .unwrap();
    // 不等 tick，立即发 Finished -> 即时路径立即 flush ✅。
    tx.send(AcpEvent::Finished { session_id: "s2".into() })
        .await
        .unwrap();

    let out_rx = out_rx.lock().await;
    // 可能有 1 个 debounce ✅... 不，Finished 即时 flush；前一个 TextDelta 的 debounce tick 若先到则 2 个卡。
    // 契约：最终必有一个含 ✅ 的 UpdateCard，且在 Finished 后 200ms 内。
    let mut got_done = false;
    for _ in 0..3 {
        let o = tokio::time::timeout(Duration::from_millis(300), out_rx.recv())
            .await
            .expect("recv in time")
            .expect("channel open");
        let s = serde_json::to_string(&o).unwrap();
        if s.contains("✅") {
            got_done = true;
            break;
        }
    }
    assert!(got_done, "Finished 必产含 ✅ 的 UpdateCard");
}

#[tokio::test]
async fn terminal_error_flushes_removes_and_exits() {
    let (router, out_rx, rx, tx) = new_pump().await;
    let map = router_handle_map(&router).clone();
    let key = SessionKey { chat_id: "oc_t".into(), thread_id: None };
    map.insert(key.clone(), Mapping::active("s3")).await.unwrap();
    router.seed_card("s3".into(), "p".into()).await;
    spawn_acp_pump(rx, router.clone(), "s3".into());

    tx.send(AcpEvent::TextDelta { session_id: "s3".into(), delta: "before".into() })
        .await
        .unwrap();
    tx.send(AcpEvent::Error { session_id: "s3".into(), message: "crashed".into(), terminal: true })
        .await
        .unwrap();

    let out_rx = out_rx.lock().await;
    let mut got_red = false;
    for _ in 0..3 {
        let o = tokio::time::timeout(Duration::from_millis(300), out_rx.recv())
            .await
            .expect("recv in time")
            .expect("channel open");
        let s = serde_json::to_string(&o).unwrap();
        if s.contains("❌") && s.contains("before") && s.contains("crashed") {
            got_red = true;
            break;
        }
    }
    assert!(got_red, "terminal 必产含 ❌ + 死前 transcript + 错误正文的卡");
    drop(out_rx);
    // mapping 已清。
    assert!(map.get(&key).await.is_none(), "terminal 后 mapping 必清");
    // 卡状态已 drop。
    assert!(router_card_state_dropped(&router, "s3").await);
}

#[tokio::test]
async fn channel_closed_drops_card_and_exits() {
    let (router, out_rx, rx, tx) = new_pump().await;
    router.seed_card("s4".into(), "p".into()).await;
    spawn_acp_pump(rx, router.clone(), "s4".into());

    // 关闭通道。
    drop(tx);
    // 给 pump 一点时间处理 None -> drop_card。
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(router_card_state_dropped(&router, "s4").await, "通道关闭后必 drop_card");
    let _ = out_rx;
}

// —— 测试辅助：通过 RouterHandle 内部 map/状态做断言。由于字段私有，
//    用公开方法间接断言 —— map 通过 SessionMap::new() 在测试里直接持有引用。
```

**关键：测试需要访问 `router` 内部的 `map` 与 `card_states` 做断言。** `RouterHandle` 字段私有。两个方案：
- (A) 测试不持有 `map` 的独立句柄，改为通过 `RouterHandle` 的公开方法间接断言（`session_alive` / `apply_event` 后 `flush_card` 的产出）。terminal 后 `session_alive` 应为 false（mapping 清了）。card_states 无公开方法 —— 加 `pub async fn has_card_state(&self, sid: &str) -> bool` 仅供测试？这污染公开 API。
- (B) 测试持有独立的 `SessionMap` 句柄（`new_pump` 里 `let map = SessionMap::new();` 既传给 `RouterHandle::new(map.clone())` 又保留 `map` 给测试断言 mapping）。card_states 的「已 drop」断言改为间接：terminal 后再发一个 `apply_event_to_out(TextDelta)` 不应 panic 且不应产生含死前内容的卡（因为状态已 drop，lazy seed 空 body）—— 太绕。

**采用方案 B + 间接断言 card_states：** `new_pump` 返回 `map` 句柄。mapping 断言用 `map.get(&key)`。card_states「已 drop」断言用间接法：terminal 后调 `router.flush_card("s3")` 应 no-op（无 CardState）—— 但 `flush_card` no-op 时不发 Out，测试无法直接观测「no-op」。改为：terminal 后 `router.apply_event("s3", TextDelta{...})` 会 lazy seed（新空 body），再 `flush_card` 产一张只含新 delta（无死前 transcript）的卡 —— 证明旧状态确被 drop。但这又依赖 apply_event 的 lazy seed 行为，较绕。

**简化：删掉 `router_card_state_dropped` 断言，仅断言 mapping 清 + terminal 卡含死前 transcript（已足够验证 §8 契约）。** card_states drop 是防泄漏的实现细节，不作为强契约测试。更新测试：删 `router_card_state_dropped` 调用与 `router_handle_map` 辅助，terminal 测试只断言「❌ 卡含死前 transcript + mapping 清」。

修正后的 `terminal_error_flushes_removes_and_exits`：

```rust
#[tokio::test]
async fn terminal_error_flushes_removes_and_exits() {
    let map = SessionMap::new();
    let key = SessionKey { chat_id: "oc_t".into(), thread_id: None };
    map.insert(key.clone(), Mapping::active("s3")).await.unwrap();
    let (router, out_rx) = RouterHandle::new(map.clone());
    router.seed_card("s3".into(), "p".into()).await;
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    spawn_acp_pump(rx, router.clone(), "s3".into());

    tx.send(AcpEvent::TextDelta { session_id: "s3".into(), delta: "before".into() })
        .await
        .unwrap();
    tx.send(AcpEvent::Error { session_id: "s3".into(), message: "crashed".into(), terminal: true })
        .await
        .unwrap();

    let out_rx = out_rx.lock().await;
    let mut got_red = false;
    for _ in 0..3 {
        let o = tokio::time::timeout(Duration::from_millis(300), out_rx.recv())
            .await
            .expect("recv in time")
            .expect("channel open");
        let s = serde_json::to_string(&o).unwrap();
        if s.contains("❌") && s.contains("before") && s.contains("crashed") {
            got_red = true;
            break;
        }
    }
    assert!(got_red, "terminal 必产含 ❌ + 死前 transcript + 错误正文的卡");
    drop(out_rx);
    assert!(map.get(&key).await.is_none(), "terminal 后 mapping 必清");
}
```

对应地 `new_pump` 也调整为返回 `map` 或测试自建。为统一，**所有测试都自建 `SessionMap` + `RouterHandle::new(map.clone())`**，删掉 `new_pump` 辅助。`five_deltas` 与 `finished` 与 `channel_closed` 测试也改为自建。重写整个测试文件（替换上面的 `new_pump` 版本）：

`tests/pump_unit_test.rs`（最终版）：

```rust
//! pump 节流契约单测（spec §6）。合成 mpsc Receiver 喂事件，断言：
//! 5 个 TextDelta 合并成 1 个 UpdateCard（≤1/150ms）；Finished 立即再发 ✅；
//! terminal Error 立即发 ❌ + 清 mapping；通道关闭 drop_card + 退出。
//! 不依赖 fake-claude 二进制。

use acp_claude::session::AcpEvent;
use router::router::{Out, RouterHandle};
use router::state::{Mapping, SessionMap};
use sebas::run::spawn_acp_pump;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn five_deltas_merge_into_one_updatecard() {
    let map = SessionMap::new();
    let (router, out_rx) = RouterHandle::new(map);
    router.seed_card("s1".into(), "hi".into()).await;
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    spawn_acp_pump(rx, router.clone(), "s1".into());

    for i in 0..5 {
        tx.send(AcpEvent::TextDelta { session_id: "s1".into(), delta: format!("chunk{i} ") })
            .await
            .unwrap();
    }
    let first = tokio::time::timeout(Duration::from_millis(400), out_rx.recv())
        .await
        .expect("first UpdateCard within 400ms")
        .expect("channel open");
    let s = serde_json::to_string(&first).unwrap();
    for i in 0..5 {
        assert!(s.contains(&format!("chunk{i}")), "chunk{i} in card: {s}");
    }
    assert!(s.contains("🚧"));
    let second = tokio::time::timeout(Duration::from_millis(120), out_rx.recv()).await;
    assert!(second.is_err(), "150ms 窗口内不得发第二个 UpdateCard");
}

#[tokio::test]
async fn finished_flushes_immediately_after_stream() {
    let map = SessionMap::new();
    let (router, out_rx) = RouterHandle::new(map);
    router.seed_card("s2".into(), "p".into()).await;
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    spawn_acp_pump(rx, router.clone(), "s2".into());

    tx.send(AcpEvent::TextDelta { session_id: "s2".into(), delta: "x".into() })
        .await
        .unwrap();
    tx.send(AcpEvent::Finished { session_id: "s2".into() })
        .await
        .unwrap();

    let mut got_done = false;
    for _ in 0..3 {
        let o = tokio::time::timeout(Duration::from_millis(300), out_rx.recv())
            .await
            .expect("recv in time")
            .expect("channel open");
        let s = serde_json::to_string(&o).unwrap();
        if s.contains("✅") {
            got_done = true;
            break;
        }
    }
    assert!(got_done, "Finished 必产含 ✅ 的 UpdateCard");
}

#[tokio::test]
async fn terminal_error_flushes_removes_and_exits() {
    let map = SessionMap::new();
    let key = feishu::events::SessionKey { chat_id: "oc_t".into(), thread_id: None };
    map.insert(key.clone(), Mapping::active("s3")).await.unwrap();
    let (router, out_rx) = RouterHandle::new(map.clone());
    router.seed_card("s3".into(), "p".into()).await;
    let (tx, rx) = mpsc::channel::<AcpEvent>(64);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    spawn_acp_pump(rx, router.clone(), "s3".into());

    tx.send(AcpEvent::TextDelta { session_id: "s3".into(), delta: "before".into() })
        .await
        .unwrap();
    tx.send(AcpEvent::Error { session_id: "s3".into(), message: "crashed".into(), terminal: true })
        .await
        .unwrap();

    let mut got_red = false;
    for _ in 0..3 {
        let o = tokio::time::timeout(Duration::from_millis(300), out_rx.recv())
            .await
            .expect("recv in time")
            .expect("channel open");
        let s = serde_json::to_string(&o).unwrap();
        if s.contains("❌") && s.contains("before") && s.contains("crashed") {
            got_red = true;
            break;
        }
    }
    assert!(got_red, "terminal 必产含 ❌ + 死前 transcript + 错误正文的卡");
    assert!(map.get(&key).await.is_none(), "terminal 后 mapping 必清");
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p sebas --test pump_unit_test -- --nocapture`
Expected: FAIL — `sebas::run::spawn_acp_pump` 不存在或私有（`error: function spawn_acp_pump is private` 或 `cannot find function`）。

- [ ] **Step 3: 实现 — 重写 spawn_acp_pump（src/run.rs:399-415）**

把当前的 `spawn_acp_pump`（私有 fn）替换为 `pub fn` + select/interval/dirty 循环：

```rust
/// Drain ACP events for one session, accumulating them into CardState and
/// flushing a single UpdateCard at most once per 150 ms (spec §6 节流契约).
///
/// - 流式事件（TextDelta/ThinkingDelta/ToolStart/ToolProgress/ToolEnd/非
///   terminal Error）: `apply_event`（状态）+ 标脏；interval tick 到点若脏
///   则 `flush_card`。
/// - Finished / terminal Error / PermissionRequest: 即时 `apply_event_to_out`
///   （terminal 额外 remove_by_session + drop_card 后泵退出）。
/// - 通道关闭（recv → None）: `drop_card` + 退出。
///
/// `rx` 在 `acp_spawn_and_activate` 里于任何慢 I/O 之前克隆，故即便 agent
/// 首次 prompt 即崩（D6）、wrapper 急切移除表项，终端事件仍能经此克隆抵达。
///
/// 机制选择（spec §6 把 async 机制委托给计划钉死）：用
/// `tokio::time::interval(150ms) + dirty bool`，而非 spec 建议的
/// `Option<Sleep> + select + pending()` —— 后者在 select 跨臂借用 `&mut`
/// 会冲突，interval + Copy bool 规避之，契约等价。
pub fn spawn_acp_pump(
    rx: std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<acp_claude::session::AcpEvent>>>,
    router: RouterHandle,
    session_id: String,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(150));
        // 第一个 tick 立即触发（tokio interval 语义）；此时 dirty=false，是 no-op。
        let mut dirty = false;
        let mut rx = rx.lock().await;
        loop {
            tokio::select! {
                maybe_evt = rx.recv() => {
                    let Some(evt) = maybe_evt else {
                        router.drop_card(&session_id).await;
                        break;
                    };
                    let is_terminal = matches!(evt, AcpEvent::Error { terminal: true, .. });
                    let is_immediate = matches!(
                        evt,
                        AcpEvent::Finished { .. }
                            | AcpEvent::Error { terminal: true, .. }
                            | AcpEvent::PermissionRequest { .. }
                    );
                    if is_immediate {
                        // 即时路径：取消待发 debounce，同步出最终态。
                        dirty = false;
                        router.apply_event_to_out(session_id.clone(), &evt).await;
                        if is_terminal {
                            break;
                        }
                    } else {
                        // 流式：只累积状态，标脏，重置 debounce 由 interval 周期保证。
                        router.apply_event(&session_id, &evt).await;
                        dirty = true;
                    }
                }
                _ = ticker.tick() => {
                    if dirty {
                        dirty = false;
                        router.flush_card(&session_id).await;
                    }
                }
            }
        }
        debug!(%session_id, "acp event stream closed; pump exiting");
    });
}
```

注意：`tokio::time::interval` 需要 `tokio` 的 `time` feature —— sebas crate 用 `features=["full"]`，✓。`select!` 需 `macros` —— `full` 含，✓。

`apply_event_to_out` 第一参是 `String`（Task 5 签名 `apply_event_to_out(&self, session_id: String, event: &AcpEvent)`）—— 传 `session_id.clone()`。`apply_event`/`flush_card`/`drop_card` 第一参 `&str` —— 传 `&session_id`。✓

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p sebas --test pump_unit_test -- --nocapture`
Expected: PASS（4 个测试绿 —— 节流契约成立）。

**若 `five_deltas` 时序偶发失败（CI 抖动）：** interval 首次 tick 立即触发，5 个事件若发送得不够快可能跨过第一个 tick（tick 时 dirty=false → no-op，下次 tick 在 150ms 后 → 那时 5 个事件已发完，dirty=true → flush）。400ms 超时应足够覆盖。若仍抖，把 first 超时调到 600ms。时序测试天然有抖动，允许在 [200, 600]ms 范围内调整超时。

- [ ] **Step 5: 全量回归确认**

Run: `cargo test --workspace --lib --tests`
Expected: PASS（router/feishu 单测 + sebas 集成测试，不含 `#[ignore]` 的 SIGTERM）。pump 重写后 `tests/spawn_race_test.rs`（binary 级）仍绿 —— 它直接 drain rx 不走 pump。

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 0 警告。

- [ ] **Step 6: 提交**

```bash
git add src/run.rs tests/pump_unit_test.rs
git commit -m "feat(sebas): pump 重写为 interval(150ms)+dirty 节流，即时路径走 apply_event_to_out"
```

---

### Task 7: dispatch_out 装配 + fake-claude stream 模式 + 端到端测试

dispatch_out SpawnAcp 臂：用 `new_with_card_config` 传真实 `CardConfig`，初始卡改 `render_accumulated_card`（theme 流入），发卡后 `seed_card`。fake-claude 加 `stream` prompt（5 chunk + end_turn）。新增端到端测试：fake-claude stream → pump → 1 个合并 UpdateCard + Finished ✅。

**Files:**
- Modify: `src/run.rs:32`（`RouterHandle::new(map)` → `new_with_card_config(map, cfg.card.clone())`）
- Modify: `src/run.rs:299-310`（初始卡改 `render_accumulated_card` + `seed_card`）
- Modify: `src/run.rs:5`（import：`render_root_card` → `render_accumulated_card`）
- Modify: `tests/bin/fake-claude.rs:151-202`（`session/prompt` 的 `_ =>` 默认分支加 `stream` 文本分支）
- Test: `tests/card_stream_e2e_test.rs`（新建）

**Interfaces:**
- Consumes: `RouterHandle::new_with_card_config` + `seed_card`（Task 5）、`feishu::cards::render_accumulated_card`（Task 4）、`spawn_acp_pump`（Task 6）
- Produces: 端到端可运行的 sebas（pump 节流 + 真实 CardConfig）

- [ ] **Step 1: 写失败测试（端到端，fake-claude stream）**

`tests/card_stream_e2e_test.rs`：

```rust
//! 端到端节流：fake-claude "stream" prompt 连发 5 个 TextDelta + end_turn。
//! 经 spawn_acp_pump（production 路径：acp_spawn_and_activate → seed_card
//! 隐含于 pump 的 lazy seed，但此处显式走 dispatch_out 不便，故直接驱动 pump）
//! 断言 150ms 内合并成 1 个含 5 段的 UpdateCard，随后 Finished 立即产 ✅ 卡。

use acp_claude::manager::SessionManager;
use acp_claude::session::AcpEvent;
use feishu::events::{FeishuIn, SessionKey};
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/fake-claude")
}

#[tokio::test]
async fn fake_claude_stream_merges_five_chunks_then_done() {
    let map = SessionMap::new();
    let (router, mut out_rx) = RouterHandle::new(map);
    let mgr = Arc::new(SessionManager::new(Duration::from_secs(30)));
    let key = SessionKey { chat_id: "oc_stream".into(), thread_id: None };

    // Text "stream" -> SpawnAcp.
    router
        .dispatch(FeishuIn::Text { key: key.clone(), text: "stream".into(), reply_to: None })
        .await;
    let out = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let Out::SpawnAcp { key: k, prompt } = out else { panic!("expected SpawnAcp, got {out:?}") };

    // 走 production spawn：create_session + rx 克隆 + CreateSession prompt + activate.
    let (session_id, _pending, rx) = sebas::run::acp_spawn_and_activate(
        &mgr, &router, &k, &prompt, fake().to_str().unwrap(), vec![], None,
    )
    .await
    .expect("spawn ok");

    // 显式 seed_card（production 在 dispatch_out 里调，此处 pump 单测路径补上）。
    router.seed_card(session_id.clone(), prompt.clone()).await;

    // 跑 production pump。
    sebas::run::spawn_acp_pump(rx, router.clone(), session_id.clone());

    // 第一个 UpdateCard：含 5 个 chunk0..chunk4，emoji 🚧。
    let first = tokio::time::timeout(Duration::from_millis(600), out_rx.recv())
        .await
        .expect("first merged UpdateCard within 600ms")
        .expect("channel open");
    let s = serde_json::to_string(&first).unwrap();
    for i in 0..5 {
        assert!(s.contains(&format!("chunk{i}")), "chunk{i} in merged card: {s}");
    }
    assert!(s.contains("🚧"));

    // Finished 立即产 ✅ 卡。
    let mut got_done = false;
    for _ in 0..3 {
        let o = tokio::time::timeout(Duration::from_millis(400), out_rx.recv())
            .await
            .expect("recv in time")
            .expect("channel open");
        let s = serde_json::to_string(&o).unwrap();
        if s.contains("✅") {
            got_done = true;
            break;
        }
    }
    assert!(got_done, "Finished 必产 ✅ 卡");
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test -p sebas --test card_stream_e2e_test -- --nocapture`
Expected: FAIL — fake-claude 还没 `stream` 分支（默认分支发 "hello "/"world" 2 chunk，断言 chunk0..chunk4 失败）。

- [ ] **Step 3: fake-claude 加 stream 分支**

`tests/bin/fake-claude.rs:162-202` 的 `"session/prompt"` 分支当前 `match text { "crash" => {...} "perm" => {...} _ => {send_chunk("hello "); send_chunk("world"); end_turn} }`。在 `"perm"` 之后、`_` 之前加 `"stream"`：

```rust
                    "stream" => {
                        for i in 0..5 {
                            send_chunk(&mut out, &sid, &format!("chunk{i} "));
                        }
                        send(
                            &mut out,
                            json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn"}}),
                        );
                    }
```

- [ ] **Step 4: dispatch_out 装配（src/run.rs）**

**4a. import（第 5 行）：**

当前：
```rust
use feishu::cards::render_root_card;
```
改为：
```rust
use feishu::cards::render_accumulated_card;
```

**4b. 构造器（第 32 行）：**

当前：
```rust
    let (router, mut out_rx) = RouterHandle::new(map);
```
改为：
```rust
    let (router, mut out_rx) = RouterHandle::new_with_card_config(map, cfg.card.clone());
```

**4c. SpawnAcp 臂初始卡 + seed_card（第 296-310 行）：**

当前：
```rust
            // 2) Send the root card and record its message_id keyed by the
            //    real session_id (so streaming UpdateCards resolve correctly).
            //    Done before the event pump starts so no early delta is lost.
            let card = render_root_card(&prompt, &session_id, "👀");
            let msg_id = feishu
                .send_card(http, tokens, &key, serde_json::to_value(&card)?)
                .await?;
            if !msg_id.is_empty() {
                router.record_root_msg_id(session_id.clone(), msg_id).await;
            }
            // 3) Pump ACP events from this session back into the router.
            //    `rx` was cloned before any slow I/O (the send_card HTTP
            //    round trip above) so a crash-on-first-prompt terminal event
            //    survives the wrapper's eager table removal (D6).
            spawn_acp_pump(rx, router.clone(), session_id.clone());
```
改为：
```rust
            // 2) seed_card（spec §4.2）: 记录 user_prompt 供后续 flush 重渲染
            //    引用块。幂等。必须在 pump 启动前，否则首个事件 lazy seed
            //    会用 prompt="" 冲掉引用块。
            router.seed_card(session_id.clone(), prompt.clone()).await;
            // 3) Send the seed card (empty body) and record its message_id
            //    keyed by the real session_id (so streaming UpdateCards
            //    resolve correctly). render_accumulated_card 用真实 theme，
            //    与后续 flush 产出的卡结构一致（避免初始卡蓝、后续卡变色的跳变）。
            let card = render_accumulated_card(&prompt, &session_id, "👀", &[], &cfg.card.theme_color);
            let msg_id = feishu
                .send_card(http, tokens, &key, serde_json::to_value(&card)?)
                .await?;
            if !msg_id.is_empty() {
                router.record_root_msg_id(session_id.clone(), msg_id).await;
            }
            // 4) Pump ACP events from this session back into the router.
            //    `rx` was cloned before any slow I/O (the send_card HTTP
            //    round trip above) so a crash-on-first-prompt terminal event
            //    survives the wrapper's eager table removal (D6).
            spawn_acp_pump(rx, router.clone(), session_id.clone());
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test -p sebas --test card_stream_e2e_test -- --nocapture`
Expected: PASS（fake-claude stream → 5 chunk 合并 1 卡 + ✅）。

- [ ] **Step 6: 全量回归**

Run: `cargo test --workspace --lib --tests`
Expected: PASS。

Run: `cargo build --workspace`
Expected: 编译通过（`render_root_card` 不再被 `src/run.rs` 引用；仅 `feishu/tests/cards_test.rs` 用 —— 保留为薄封装，✓）。

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 0 警告（若 `render_root_card` 报「未使用」—— 它被 cards_test 用，不报；若报，确认 cards_test 仍 import 它）。

- [ ] **Step 7: 提交**

```bash
git add src/run.rs tests/bin/fake-claude.rs tests/card_stream_e2e_test.rs
git commit -m "feat(sebas): dispatch_out 接 seed_card+真实 CardConfig，fake-claude stream 模式"
```

---

### Task 8: 全量验收 + clippy + SIGTERM opt-in

全量 `cargo test --workspace` 绿 + clippy 0 警告 + SIGTERM opt-in 集成测试绿（确认 pump/seed_card 改动未破坏 daemon 生命周期）。

**Files:**
- 无新文件；可能微调时序超时（若 SIGTERM 或 pump 单测抖动）。

**Interfaces:**
- Consumes: 全部前序任务
- Produces: 可合并的分支（全量绿）。

- [ ] **Step 1: 全量单测 + 集成**

Run: `cargo test --workspace --lib --tests -- --nocapture`
Expected: PASS（router 4 个测试文件 + feishu cards_test/event_parse_test/media_test/token_manager_test + sebas config_test/replay_test/spawn_race_test/pump_unit_test/card_stream_e2e_test/error_test）。

- [ ] **Step 2: SIGTERM opt-in**

Run: `cargo test --workspace --test sigterm_cleanup_test -- --ignored --nocapture`
Expected: PASS（daemon 正常起停、fake-claude 子进程被 reap、state 文件持久）。若失败，检查 pump/seed_card 改动是否影响 `SEBAS_TEST_SPAWN_SESSION` 启动路径（应无 —— 那条路径不走 pump）。

- [ ] **Step 3: clippy 0 警告**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 0 警告。若有，按提示修（常见：`render_root_card` 未使用 —— 加 `#[allow(dead_code)]` 或确认 cards_test 引用；unused `truncate` —— Task 3 已删）。

- [ ] **Step 4: 最终全量确认**

Run: `cargo test --workspace -- --include-ignored --nocapture 2>&1 | tail -5` （若想一次跑含 ignored）
Expected: 全绿。

- [ ] **Step 5: 提交（若有微调）**

```bash
git status
# 若有 clippy/时序微调：
git add -A
git commit -m "test(sebas): 全量绿 + clippy 0 警告 + SIGTERM opt-in 通过"
# 若无改动，跳过提交。
```

- [ ] **Step 6: 收尾 beads**

关闭 beads issue sebas-tz4（若存在）：
```bash
bd close sebas-tz4
```
报告 handoff：变更文件清单、验证结果（全量绿 + clippy 0 + SIGTERM opt-in）、建议合并命令（等用户授权后）。

---

## Self-Review 结果

**1. Spec 覆盖：**
- §2 C1（每事件重建空卡）→ Task 5 重写 apply_event_to_out 为 apply_event+flush_card（累积）。✓
- §2 C2（Finished 清空 transcript）→ Task 5 共享臂 apply_event 累积 + flush_card。✓
- §2 C3（ThinkingDelta/ToolEnd/ToolProgress 被吞）→ Task 3 apply_event_to_card 复活三分支 + Task 5 apply_event 调用它。✓
- §2 C4（[card] 死配置）→ Task 1 CardConfig 迁移 + Task 3 截断/fold + Task 4 theme_color + Task 5 new_with_card_config + Task 7 dispatch_out 传真实 cfg。✓
- §2 C5（terminal Error 重建空卡）→ Task 5 terminal 臂 apply_event+flush_card 保留 transcript。✓
- §2 C6（无节流）→ Task 6 pump interval(150ms)+dirty。✓
- §4.1 CardState → Task 2。✓
- §4.2 seed_card/apply_event/flush_card/drop_card/apply_event_to_out → Task 5。✓
- §4.3 render_accumulated_card → Task 4。✓
- §5 FSM → Task 5 next_emoji。✓
- §6 节流契约 → Task 6（机制钉死为 interval+dirty，契约等价，已注明偏差）。✓
- §7 截断/fold/总量 → Task 3。✓
- §7 theme_color → Task 4 + Task 5/7。✓
- §8 terminal 并入累积 → Task 5。✓
- §9 测试（累积单测/FSM/节流/截断/terminal 保留/regression/fake-claude stream/全量+SIGTERM）→ Task 5/3/6/7/8。✓
- §10 不做 → 未涉及 reaction/媒体/slash/重启恢复/permission 卡渲染。✓

**2. 占位符扫描：** 无 TBD/TODO；每个代码步骤含完整代码。✓

**3. 类型一致性：**
- `CardConfig`：Task 1 定义 4 字段 → Task 3 `..cfg()`/`..CardConfig::default()` 结构更新语法 → Task 5 `CardConfig { theme_color:.., ..CardConfig::default() }` → Task 7 `cfg.card.clone()`。一致。✓
- `apply_event_to_card(body: &mut Vec<CardElement>, evt: &AcpEvent, cfg: &CardConfig)`：Task 3 定义 → Task 5 `apply_event` 调用签名一致。✓
- `render_accumulated_card(user_prompt, session_id, status_emoji, body: &[CardElement], theme: &str)`：Task 4 定义 → Task 5 `flush_card` 调用一致 → Task 7 dispatch_out 调用一致。✓
- `RouterHandle::{seed_card(String,String)/apply_event(&str,&AcpEvent)/flush_card(&str)/drop_card(&str)/new_with_card_config(map,cfg)/apply_event_to_out(String,&AcpEvent)}`：Task 5 定义 → Task 6 pump 调用 `apply_event(&session_id,&evt)`/`flush_card(&session_id)`/`drop_card(&session_id)`/`apply_event_to_out(session_id.clone(),&evt)` 一致 → Task 7 dispatch_out 调用 `seed_card(sid,prompt)` 一致。✓
- `spawn_acp_pump(rx: Arc<Mutex<Receiver>>, router, session_id: String)`：Task 6 定义（pub）→ Task 7 端到端测试 + dispatch_out 调用一致。✓
- `CardStateMap::{seed/apply/snapshot/drop}`：Task 2 定义 → Task 5 `seed_card`/`apply_event`/`flush_card`/`drop_card` 调用一致。✓

**4. 偏差已记录：** spec §6「dispatch_acp_event 改为不发 Out」与 §9「零改动通过」冲突 —— 本计划保留 dispatch_acp_event 同步语义，仅改 pump 路径（Task 5 Step 4h 注明）。spec §7 `new(map,card_cfg)` 改签名 → 本计划 `new(map)`+`new_with_card_config`（Global Constraints 注明，更稳）。
