# Thinking Fold + /settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fold `ThinkingDelta` content into collapsible panels by default; add a runtime-tunable `/settings` command that persists overrides to `~/.sebas/settings.json` and applies them live.

**Architecture:**
- New `ThinkingDisplay` enum (`show` | `hide`) lives in `feishu::cards::CardConfig.thinking`; `disable` is reserved but **not** exposed yet (per user decision).
- `card_events.rs` aggregates adjacent ThinkingDelta chunks into one `CollapsiblePanel` per "thinking burst"; bursts are bounded by non-thinking events (TextDelta / ToolStart / Finished / Error). `hide` drops ThinkingDelta entirely.
- `card_cfg` on `RouterHandle` becomes `Arc<RwLock<CardConfig>>` so changes apply live on the next event.
- New `router/src/settings.rs` handles JSON file I/O; startup merges TOML → settings.json (full-snapshot semantics); `/settings` re-reads, mutates one field, atomically writes back.
- New `Out::PlainText { key, content }` variant + `FeishuClient::send_text` enable real text-message replies (also fixes the existing `/help` no-op as a side effect).

**Tech Stack:** Rust 2021, serde / serde_json, tokio RwLock, dirs (for `home_dir`).

## Global Constraints

- 项目 git 提交遵循 Conventional Commits；commit message 一行为佳（见 `/home/bot/.claude/projects/-home-bot-workbench/memory/feedback-commit-message-style.md`）。
- 串行提交：相关代码同 commit，不琐碎拆。
- 中文注释、英文代码标识符；保持现有 feishu/router crate 风格。
- TDD：每个组件先红后绿；不允许"先写实现后补测试"。
- 不引入新 crate 依赖；只用已存在的 serde / serde_json / tokio / dirs。

---

## File Structure

| File | Responsibility |
|---|---|
| `feishu/src/cards.rs` | 加 `ThinkingDisplay` enum、`CardConfig.thinking`、`Serialize` derive |
| `router/src/card_events.rs` | ThinkingDelta 折叠/丢弃逻辑、聚合边界检测 |
| `router/src/settings.rs`（新） | settings.json 读 / 写 / merge / 路径解析 |
| `router/src/commands.rs` | 加 `Command::Settings` 解析 |
| `router/src/router/mod.rs` | `card_cfg: Arc<RwLock<CardConfig>>`、`Out::PlainText` 变体 |
| `router/src/router/inbound.rs` | `/settings` 分发与错误回复 |
| `router/src/lib.rs` | 导出 `settings` 模块 |
| `feishu/src/client.rs` | 加 `send_text` 辅助方法 |
| `src/dispatch.rs` | 加 `Out::PlainText` 处理分支 |
| `src/run.rs` | 启动时合并 settings.json → RwLock |
| `config/config.toml.example` | 文档化 `thinking` 字段 |
| `router/tests/card_events_test.rs` | 改旧 thinking 测试 + 加 show/hide 测试 |
| `router/tests/commands_test.rs`（新） | `/settings` 解析测试 |
| `router/tests/settings_test.rs`（新） | settings.json I/O + merge 测试 |

---

### Task 1: Add `ThinkingDisplay` enum + extend `CardConfig`

**Files:**
- Modify: `feishu/src/cards.rs:22-50` (the `CardConfig` struct + Default impl)

- [ ] **Step 1: Write failing test in `feishu/tests/cards_test.rs`**

```rust
#[test]
fn card_config_thinking_default_is_show() {
    let cfg = feishu::cards::CardConfig::default();
    assert_eq!(cfg.thinking, feishu::cards::ThinkingDisplay::Show);
}

#[test]
fn card_config_serializes_thinking_as_lowercase() {
    let cfg = feishu::cards::CardConfig::default();
    let v = serde_json::to_value(&cfg).unwrap();
    assert_eq!(v["thinking"], "show");
}

#[test]
fn card_config_deserializes_thinking_from_lowercase() {
    let v = serde_json::json!({ "thinking": "hide" });
    let cfg: feishu::cards::CardConfig = serde_json::from_value(v).unwrap();
    assert_eq!(cfg.thinking, feishu::cards::ThinkingDisplay::Hide);
}

#[test]
fn card_config_rejects_unknown_thinking_value() {
    let v = serde_json::json!({ "thinking": "disable" });
    let res: Result<feishu::cards::CardConfig, _> = serde_json::from_value(v);
    assert!(res.is_err(), "disable is not exposed yet, must fail to parse");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p feishu --test cards_test`
Expected: compile error (`ThinkingDisplay` not defined, `thinking` field missing).

- [ ] **Step 3: Implement enum + extend CardConfig in `feishu/src/cards.rs`**

Add to `feishu/src/cards.rs` after the `CardConfig` struct:

```rust
/// How to render the model's `thinking` content into the Feishu card.
/// `disable` is intentionally not exposed — reserved for a future
/// feature that would also turn off thinking tokens at the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingDisplay {
    /// Fold each thinking burst into a collapsible_panel (default).
    Show,
    /// Drop ThinkingDelta events from the card body entirely. The model
    /// still produces thinking tokens; we just don't surface them.
    Hide,
}

impl Default for ThinkingDisplay {
    fn default() -> Self {
        Self::Show
    }
}
```

Modify the `CardConfig` struct:

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CardConfig {
    #[serde(default = "default_theme_color")]
    pub theme_color: String,
    #[serde(default = "default_max_user_text")]
    pub max_user_text_chars: usize,
    #[serde(default = "default_max_tool_output")]
    pub max_tool_output_chars: usize,
    #[serde(default = "default_true")]
    pub fold_long_output: bool,
    #[serde(default)]
    pub thinking: ThinkingDisplay,
}
```

Remove the now-unused `Default` impl for `CardConfig` (the derived one covers it).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p feishu --test cards_test`
Expected: PASS for all 4 new tests.

- [ ] **Step 5: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add feishu/src/cards.rs feishu/tests/cards_test.rs
git commit -m "feat(feishu): add ThinkingDisplay enum and card.thinking config"
```

---

### Task 2: Render ThinkingDelta into collapsible panels (show/hide)

**Files:**
- Modify: `router/src/card_events.rs:18-90` (the `apply_event_to_card` function's `ThinkingDelta` arm)

- [ ] **Step 1: Write failing tests in `router/tests/card_events_test.rs`**

Replace the existing `append_revives_thinking_toolend_toolprogress` test with the new shape (it currently asserts `Hr + Div`, which is no longer true). Add new tests for hide/show/boundary behavior.

```rust
use feishu::cards::{CardConfig, CardElement, ThinkingDisplay};

fn cfg_show() -> CardConfig {
    CardConfig::default()  // thinking = Show
}
fn cfg_hide() -> CardConfig {
    CardConfig { thinking: ThinkingDisplay::Hide, ..CardConfig::default() }
}

#[test]
fn thinking_hide_drops_delta() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta { session_id: "s".into(), delta: "hidden".into() },
        &cfg_hide(),
    );
    assert!(body.is_empty(), "hide mode must drop ThinkingDelta entirely");
}

#[test]
fn thinking_show_aggregates_adjacent_deltas() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta { session_id: "s".into(), delta: "A".into() },
        &cfg_show(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta { session_id: "s".into(), delta: "B".into() },
        &cfg_show(),
    );
    // Single CollapsiblePanel, no separator, content "A\nB".
    assert_eq!(body.len(), 1);
    let CardElement::CollapsiblePanel(panel) = &body[0] else {
        panic!("expected CollapsiblePanel, got {:?}", &body[0]);
    };
    assert_eq!(panel.elements.len(), 1);
    match &panel.elements[0] {
        CardElement::Markdown { content } => assert_eq!(content, "A\nB"),
        other => panic!("expected Markdown, got {other:?}"),
    }
}

#[test]
fn thinking_show_starts_new_panel_on_non_thinking_event() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta { session_id: "s".into(), delta: "first".into() },
        &cfg_show(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::TextDelta { session_id: "s".into(), delta: "interlude".into() },
        &cfg_show(),
    );
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta { session_id: "s".into(), delta: "second".into() },
        &cfg_show(),
    );
    // 3 elements: panel1, markdown("interlude"), panel2.
    assert_eq!(body.len(), 3);
    assert!(matches!(&body[0], CardElement::CollapsiblePanel(_)));
    match &body[1] {
        CardElement::Markdown { content } => assert_eq!(content, "interlude"),
        other => panic!("expected Markdown, got {other:?}"),
    }
    assert!(matches!(&body[2], CardElement::CollapsiblePanel(_)));
}

#[test]
fn thinking_show_panel_header_is_thinking_label() {
    let mut body = vec![];
    apply_event_to_card(
        &mut body,
        &AcpEvent::ThinkingDelta { session_id: "s".into(), delta: "x".into() },
        &cfg_show(),
    );
    let CardElement::CollapsiblePanel(panel) = &body[0] else {
        panic!("not a panel");
    };
    assert!(panel.header.title.content.contains("💭"));
    assert!(!panel.expanded, "default state is collapsed");
}
```

Remove the old `append_revives_thinking_toolend_toolprogress` test (its assertions no longer hold).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p router --test card_events_test`
Expected: compile error or assertion failure on the new tests.

- [ ] **Step 3: Implement aggregation logic in `router/src/card_events.rs`**

Replace the `AcpEvent::ThinkingDelta` arm in `apply_event_to_card`:

```rust
AcpEvent::ThinkingDelta { delta, .. } => {
    if cfg.thinking == ThinkingDisplay::Hide {
        // Drop entirely: model still thinks, just not surfaced in the card.
        return;
    }
    if append ThinkingDelta(body, delta);
}
```

Add the aggregation helper:

```rust
/// Append a ThinkingDelta to the trailing thinking panel. If the body
/// ends with a non-thinking element, start a new panel (boundary
/// aggregation: adjacent thinking chunks share one panel; any
/// non-thinking event ends the current burst).
fn append_thinking_delta(body: &mut Vec<CardElement>, delta: &str) {
    if let Some(CardElement::CollapsiblePanel(panel)) = body.last_mut()
        && panel.header.title.content.contains("💭")
    {
        // Extend the existing trailing thinking panel: append a newline
        // and the new delta to the trailing Markdown element.
        append_to_thinking_panel(panel, delta);
        return;
    }
    body.push(CardElement::CollapsiblePanel(CollapsiblePanel {
        expanded: false,
        header: thinking_panel_header(),
        elements: vec![CardElement::Markdown {
            content: delta.to_string(),
        }],
    }));
}

fn append_to_thinking_panel(panel: &mut CollapsiblePanel, delta: &str) {
    match panel.elements.last_mut() {
        Some(CardElement::Markdown { content }) => {
            content.push('\n');
            content.push_str(delta);
        }
        _ => panel.elements.push(CardElement::Markdown {
            content: delta.to_string(),
        }),
    }
}

fn thinking_panel_header() -> CollapsiblePanelHeader {
    CollapsiblePanelHeader {
        title: CardText {
            tag: "plain_text".into(),
            content: "💭 思考".into(),
        },
        icon: StandardIcon {
            tag: "standard_icon".into(),
            token: "down-small-ccm_outlined".into(),
            size: "16px 16px".into(),
        },
        icon_position: "right".into(),
        icon_expanded_angle: -180,
    }
}
```

Update `enforce_total_budget`'s caller: since we removed the `Hr` push for ThinkingDelta, no change needed in that function. But check the existing `element_chars` — it already handles `CollapsiblePanel`.

Add to the top of the file: `use feishu::cards::{CardConfig, CardElement, CardText, CollapsiblePanel, CollapsiblePanelHeader, DivText, StandardIcon, ThinkingDisplay};`

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p router --test card_events_test`
Expected: PASS for all 4 new tests.

- [ ] **Step 5: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add router/src/card_events.rs router/tests/card_events_test.rs
git commit -m "feat(router): fold ThinkingDelta into collapsible panels with hide/show modes"
```

---

### Task 3: Create `router/src/settings.rs` (file I/O + merge)

**Files:**
- Create: `router/src/settings.rs`
- Create: `router/tests/settings_test.rs`

- [ ] **Step 1: Write failing tests in `router/tests/settings_test.rs`**

```rust
use feishu::cards::{CardConfig, ThinkingDisplay};
use router::settings::{load_settings, save_settings, settings_path};

#[test]
fn settings_path_under_home_sebas_dir() {
    let p = settings_path();
    let s = p.to_string_lossy();
    assert!(s.contains(".sebas"), "expected .sebas dir, got {s}");
    assert!(s.ends_with("settings.json"), "got {s}");
}

#[test]
fn save_then_load_round_trips() {
    let dir = tempdir();
    let path = dir.join("settings.json");
    let mut cfg = CardConfig::default();
    cfg.thinking = ThinkingDisplay::Hide;
    save_settings(&path, &cfg).unwrap();
    let loaded = load_settings(&path).unwrap();
    assert_eq!(loaded.thinking, ThinkingDisplay::Hide);
}

#[test]
fn load_missing_returns_default() {
    let dir = tempdir();
    let path = dir.join("missing.json");
    let loaded = load_settings(&path).unwrap();
    assert_eq!(loaded, CardConfig::default());
}

#[test]
fn load_malformed_returns_error() {
    let dir = tempdir();
    let path = dir.join("bad.json");
    std::fs::write(&path, "{not json").unwrap();
    assert!(load_settings(&path).is_err());
}

#[test]
fn save_writes_pretty_json() {
    let dir = tempdir();
    let path = dir.join("settings.json");
    save_settings(&path, &CardConfig::default()).unwrap();
    let s = std::fs::read_to_string(&path).unwrap();
    assert!(s.contains('\n'), "expected pretty-printed JSON, got: {s}");
}

fn tempdir() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("sebas-settings-test-{}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
```

Add `tempdir` to keep tests hermetic — each test uses its own subdir keyed by `process::id()`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p router --test settings_test`
Expected: compile error (module doesn't exist).

- [ ] **Step 3: Implement `router/src/settings.rs`**

```rust
//! Persistent settings file: `~/.sebas/settings.json`.
//!
//! Full-snapshot semantics: each write serializes the entire `CardConfig`.
//! On startup, the in-memory config is the file content (which itself was
//! the TOML defaults at the time of first write). Strict parse: malformed
//! JSON or wrong-typed fields cause `load_settings` to return an error so
//! `run::run` can refuse to start with a clear message.

use feishu::cards::CardConfig;
use std::path::{Path, PathBuf};

/// `~/.sebas/settings.json`, expanded at call time so the env is honoured.
pub fn settings_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".sebas").join("settings.json")
}

/// Read + parse settings.json. Returns `Ok(CardConfig::default())` when
/// the file doesn't exist. Returns `Err` on any parse / IO error — the
/// caller decides whether to refuse to start.
pub fn load_settings(path: &Path) -> Result<CardConfig, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| {
            format!(
                "settings.json 解析失败 ({}): {e}",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CardConfig::default()),
        Err(e) => Err(format!("读取 settings.json 失败: {e}")),
    }
}

/// Pretty-print the full CardConfig to the file. Creates parent dirs.
/// Uses write-to-temp + rename for atomicity.
pub fn save_settings(path: &Path, cfg: &CardConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 settings 父目录失败: {e}"))?;
    }
    let s = serde_json::to_string_pretty(cfg)
        .map_err(|e| format!("序列化 settings 失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, s).map_err(|e| format!("写 settings 临时文件失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename settings 失败: {e}"))?;
    Ok(())
}
```

Add `pub mod settings;` to `router/src/lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p router --test settings_test`
Expected: PASS for all 5 tests.

- [ ] **Step 5: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add router/src/settings.rs router/src/lib.rs router/tests/settings_test.rs
git commit -m "feat(router): add settings.json I/O module"
```

---

### Task 4: Add `Command::Settings` to the command parser

**Files:**
- Modify: `router/src/commands.rs` (the `Command` enum + `parse_command` function)

- [ ] **Step 1: Write failing tests in `router/tests/commands_test.rs`**

```rust
use router::commands::{parse_command, Command};

#[test]
fn parse_settings_alone() {
    assert_eq!(parse_command("/settings"), Command::Settings(None, None));
}

#[test]
fn parse_settings_key_only() {
    assert_eq!(
        parse_command("/settings thinking"),
        Command::Settings(Some("thinking".into()), None)
    );
}

#[test]
fn parse_settings_key_value() {
    assert_eq!(
        parse_command("/settings thinking hide"),
        Command::Settings(Some("thinking".into()), Some("hide".into()))
    );
}

#[test]
fn parse_settings_trims_whitespace() {
    assert_eq!(
        parse_command("  /settings   thinking    show  "),
        Command::Settings(Some("thinking".into()), Some("show".into()))
    );
}

#[test]
fn parse_settings_unknown_key_value_passes_through_value() {
    // We don't validate key names at parse time — validation happens at
    // apply time so the error message can list known keys.
    assert_eq!(
        parse_command("/settings foo bar baz"),
        Command::Settings(Some("foo".into()), Some("bar baz".into()))
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p router --test commands_test`
Expected: compile error (variant not defined).

- [ ] **Step 3: Extend the parser**

Replace `router/src/commands.rs`:

```rust
#[derive(Debug, PartialEq)]
pub enum Command {
    New,
    Sessions,
    Switch(usize),
    Resume(String),
    Status,
    Compact,
    Cost,
    Cancel,
    Model(String),
    Cd(String),
    Help,
    Btw(String),
    /// `/settings` | `/settings <key>` | `/settings <key> <value>`.
    Settings(Option<String>, Option<String>),
    PassThrough(String),
}

pub fn parse_command(input: &str) -> Command {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("//") {
        return Command::PassThrough(format!("/{rest}"));
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();
    match head {
        "/new" => Command::New,
        "/sessions" => Command::Sessions,
        "/switch" => match arg.parse::<usize>() {
            Ok(n) => Command::Switch(n),
            Err(_) => Command::PassThrough(input.into()),
        },
        "/resume" => Command::Resume(arg.into()),
        "/status" => Command::Status,
        "/compact" => Command::Compact,
        "/cost" => Command::Cost,
        "/cancel" => Command::Cancel,
        "/model" => Command::Model(arg.into()),
        "/cd" => Command::Cd(arg.into()),
        "/help" => Command::Help,
        "/settings" => {
            let mut kv = arg.splitn(2, char::is_whitespace);
            let key = kv.next().unwrap_or("").trim();
            let val = kv.next().unwrap_or("").trim();
            let key = if key.is_empty() { None } else { Some(key.to_string()) };
            let val = if val.is_empty() { None } else { Some(val.to_string()) };
            Command::Settings(key, val)
        }
        "/btw" => {
            if arg.is_empty() {
                Command::PassThrough(input.into())
            } else {
                Command::Btw(arg.into())
            }
        }
        _ => Command::PassThrough(input.into()),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p router --test commands_test`
Expected: PASS for all 5 tests.

- [ ] **Step 5: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add router/src/commands.rs router/tests/commands_test.rs
git commit -m "feat(router): parse /settings command"
```

---

### Task 5: Wrap `card_cfg` in `Arc<RwLock<>>` on `RouterHandle`

**Files:**
- Modify: `router/src/router/mod.rs:82-152` (RouterHandle struct, new_with_config, apply_event)
- Modify: `router/src/router/inbound.rs` (any handler that reads `card_cfg` directly — verify by grep)

- [ ] **Step 1: Update all callers + `apply_event` to take read lock**

In `router/src/router/mod.rs`:

- Change field: `card_cfg: Arc<RwLock<CardConfig>>` (add `use std::sync::Arc; use tokio::sync::RwLock;`).
- In `new_with_config`: `card_cfg: Arc::new(RwLock::new(card_cfg))`.
- In `Clone for RouterHandle`: clone the `Arc`.
- In `apply_event`: hold a read guard across the closure so `apply_event_to_card` gets `&CardConfig`:

```rust
pub async fn apply_event(&self, session_id: &str, event: &AcpEvent) -> Option<&'static str> {
    let cfg = self.card_cfg.read().await;
    self.card_states
        .apply(session_id, |st| {
            let next = next_emoji(&st.status_emoji, event);
            if let Some(e) = next {
                st.status_emoji = e.into();
            }
            apply_event_to_card(&mut st.body, event, &cfg);
            next
        })
        .await
}
```

Add a public method to mutate the live config (used by `/settings` handler):

```rust
pub async fn set_card_config(&self, new_cfg: CardConfig) {
    let mut g = self.card_cfg.write().await;
    *g = new_cfg;
}

pub async fn card_config(&self) -> CardConfig {
    self.card_cfg.read().await.clone()
}
```

- Update `replay.rs` and `tests/` that construct `RouterHandle::new` / `new_with_config` — they should still compile because the constructor signature is unchanged.

- [ ] **Step 2: Compile to verify nothing broke**

Run: `cargo build --workspace`
Expected: compiles. No new test changes here; structural refactor.

- [ ] **Step 3: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add router/src/router/mod.rs router/src/router/inbound.rs
git commit -m "refactor(router): wrap card_cfg in Arc<RwLock<>> for live updates"
```

---

### Task 6: Add `Out::PlainText` variant + `FeishuClient::send_text`

**Files:**
- Modify: `feishu/src/client.rs` (add `send_text` method after `send_card`)
- Modify: `router/src/router/mod.rs:23-80` (add variant)
- Modify: `src/dispatch.rs:210-213` (handle new variant)
- Modify: `src/run.rs` (no change to outbound pump loop, but `tokens` and `http` are already passed)

- [ ] **Step 1: Add `send_text` to `feishu/src/client.rs`**

Find the existing `send_card` signature to mirror the style, then add right after it:

```rust
/// Send a plain text message to the chat. Mirrors the HTTP shape used by
/// `hello_msg` / `test_msg` in run.rs, but goes through the same auth +
/// retry path as `send_card` for production callers.
pub async fn send_text(
    &self,
    http: &reqwest::Client,
    tokens: &TokenManager,
    key: &SessionKey,
    text: &str,
) -> anyhow::Result<()> {
    use crate::messages::{ReceiveIdType, SendTextRequest};
    let (receive_id, id_type) = match key {
        SessionKey::Private(open_id) => (open_id.clone(), ReceiveIdType::OpenId),
        SessionKey::Group(chat_id) => (chat_id.clone(), ReceiveIdType::ChatId),
    };
    let url = format!(
        "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type={}",
        match id_type {
            ReceiveIdType::OpenId => "open_id",
            ReceiveIdType::ChatId => "chat_id",
        }
    );
    let req = SendTextRequest::new(&receive_id, id_type, text);
    let body = serde_json::to_value(&req)?;
    let bearer = tokens.token().await?;
    let resp = http.post(&url).bearer_auth(&bearer).json(&body).send().await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("send_text failed: {status} {body}");
    }
    Ok(())
}
```

Read the existing `send_card` for the exact error-handling + log conventions and align.

- [ ] **Step 2: Add `Out::PlainText` variant in `router/src/router/mod.rs`**

After the existing `HelpText { key }` variant:

```rust
/// Plain-text reply to the originating chat (e.g. `/settings`, `/help`).
/// The dispatcher uses FeishuClient::send_text — not a card.
PlainText {
    key: SessionKey,
    content: String,
},
```

- [ ] **Step 3: Handle in `src/dispatch.rs`**

Replace the `Out::HelpText` arm:

```rust
Out::HelpText { key } => {
    info!(?key, "send help");
}
```

with two arms (or rename HelpText to PlainText and keep both — decision: keep HelpText as a no-op stub for now since `/help` is not in scope; add a new PlainText arm):

```rust
Out::HelpText { key } => {
    info!(?key, "send help (no-op: help text not implemented)");
}
Out::PlainText { key, content } => {
    if let Err(e) = feishu
        .send_text(http, tokens, &key, &content)
        .await
    {
        warn!(?e, "send_text failed");
    }
}
```

(Implement `/help` text content as a bonus in Task 8 — out of scope here.)

- [ ] **Step 4: Build and run existing tests**

Run: `cargo build --workspace && cargo test -p router --test routing_paths_test`
Expected: existing tests still pass (HelpText arm is preserved as a no-op).

- [ ] **Step 5: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add feishu/src/client.rs router/src/router/mod.rs src/dispatch.rs
git commit -m "feat(feishu,router): add Out::PlainText variant and send_text helper"
```

---

### Task 7: Implement `/settings` handler in `router/src/router/inbound.rs`

**Files:**
- Modify: `router/src/router/inbound.rs:35-115` (the `on_text` match on Command)

- [ ] **Step 1: Write a router integration test in `router/tests/settings_handler_test.rs`**

```rust
use feishu::cards::{CardConfig, ThinkingDisplay};
use router::commands::parse_command;
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use feishu::events::{ReceiveIdType, SessionKey};

fn key() -> SessionKey {
    SessionKey::Private("ou_test".into())
}

async fn next_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    rx.recv().await.expect("expected Out")
}

#[tokio::test]
async fn settings_list_emits_all_keys() {
    let (router, mut rx) = RouterHandle::new(SessionMap::new());
    let _ = std::fs::remove_file(router::settings::settings_path());
    router
        .dispatch(feishu::events::FeishuIn::Text {
            key: key(),
            text: "/settings".into(),
            reply_to: None,
        })
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { key, content } = out else {
        panic!("expected PlainText, got {out:?}");
    };
    assert!(content.contains("thinking"), "missing thinking in list: {content}");
    assert!(content.contains("show"), "default thinking not shown: {content}");
}

#[tokio::test]
async fn settings_set_persists_and_updates_router() {
    let (router, mut rx) = RouterHandle::new(SessionMap::new());
    // Clean any leftover from previous runs.
    let _ = std::fs::remove_file(router::settings::settings_path());

    router
        .dispatch(feishu::events::FeishuIn::Text {
            key: key(),
            text: "/settings thinking hide".into(),
            reply_to: None,
        })
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { content, .. } = out else {
        panic!("expected PlainText, got {out:?}");
    };
    assert!(content.contains("hide"));

    // Verify in-memory config updated.
    let cfg = router.card_config().await;
    assert_eq!(cfg.thinking, ThinkingDisplay::Hide);

    // Verify file written.
    let loaded = router::settings::load_settings(&router::settings::settings_path()).unwrap();
    assert_eq!(loaded.thinking, ThinkingDisplay::Hide);
}

#[tokio::test]
async fn settings_rejects_invalid_value() {
    let (router, mut rx) = RouterHandle::new(SessionMap::new());
    let _ = std::fs::remove_file(router::settings::settings_path());
    router
        .dispatch(feishu::events::FeishuIn::Text {
            key: key(),
            text: "/settings thinking disable".into(),
            reply_to: None,
        })
        .await;
    let out = next_out(&mut rx).await;
    let Out::PlainText { content, .. } = out else {
        panic!("expected PlainText");
    };
    assert!(
        content.contains("可选值") || content.contains("show"),
        "expected validation error, got {content}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p router --test settings_handler_test`
Expected: compile error or panic on `Out::PlainText` match.

- [ ] **Step 3: Implement the handler in `inbound.rs`**

Add to the imports at the top of `router/src/router/inbound.rs`:

```rust
use feishu::cards::{CardConfig, ThinkingDisplay};
use crate::settings;
```

Add a new match arm before `Command::Help`:

```rust
Command::Settings(key, val) => {
    self.handle_settings(key, val, key_origin.clone()).await;
}
```

Wait — `on_text` signature uses `key: SessionKey`. The `key` from `Command::Settings` is the option<String> (key name), shadowing the chat key. Rename to avoid collision:

```rust
Command::Settings(setting_key, val) => {
    self.handle_settings(key, setting_key, val).await;
}
```

Add the helper method on `impl RouterHandle`:

```rust
async fn handle_settings(
    &self,
    key: SessionKey,
    setting_key: Option<String>,
    val: Option<String>,
) {
    let path = settings::settings_path();
    let mut cfg = self.card_cfg.read().await.clone();

    let content = match (setting_key, val) {
        (None, _) => self.render_settings_list(&cfg),
        (Some(k), None) => self.render_setting(&cfg, &k),
        (Some(k), Some(v)) => match self.apply_setting(&mut cfg, &k, &v) {
            Ok(()) => {
                // Persist + apply live.
                if let Err(e) = settings::save_settings(&path, &cfg) {
                    self.emit(Out::PlainText {
                        key,
                        content: format!("保存失败: {e}"),
                    })
                    .await;
                    return;
                }
                self.set_card_config(cfg.clone()).await;
                format!("{k} = {v} (已写入 {})", path.display())
            }
            Err(msg) => msg,
        },
    };
    self.emit(Out::PlainText { key, content }).await;
}

fn render_settings_list(&self, cfg: &CardConfig) -> String {
    format!(
        "当前设置（来源：{}）:\n\
         thinking = {}\n\
         max_user_text_chars = {}\n\
         max_tool_output_chars = {}\n\
         fold_long_output = {}\n\
         theme_color = {}",
        settings::settings_path().display(),
        thinking_label(cfg.thinking),
        cfg.max_user_text_chars,
        cfg.max_tool_output_chars,
        cfg.fold_long_output,
        cfg.theme_color,
    )
}

fn render_setting(&self, cfg: &CardConfig, k: &str) -> String {
    match k {
        "thinking" => format!("thinking = {}", thinking_label(cfg.thinking)),
        "max_user_text_chars" => format!("max_user_text_chars = {}", cfg.max_user_text_chars),
        "max_tool_output_chars" => format!("max_tool_output_chars = {}", cfg.max_tool_output_chars),
        "fold_long_output" => format!("fold_long_output = {}", cfg.fold_long_output),
        "theme_color" => format!("theme_color = {}", cfg.theme_color),
        other => format!(
            "未知键: {other}\n可用键: thinking, max_user_text_chars, max_tool_output_chars, fold_long_output, theme_color"
        ),
    }
}

fn apply_setting(&self, cfg: &mut CardConfig, k: &str, v: &str) -> Result<(), String> {
    match k {
        "thinking" => match v {
            "show" => cfg.thinking = ThinkingDisplay::Show,
            "hide" => cfg.thinking = ThinkingDisplay::Hide,
            other => return Err(format!(
                "thinking 可选值: show, hide（拒绝: {other})"
            )),
        },
        "max_user_text_chars" => {
            cfg.max_user_text_chars = v.parse().map_err(|e| format!("数字解析失败: {e}"))?
        }
        "max_tool_output_chars" => {
            cfg.max_tool_output_chars = v.parse().map_err(|e| format!("数字解析失败: {e}"))?
        }
        "fold_long_output" => match v {
            "true" => cfg.fold_long_output = true,
            "false" => cfg.fold_long_output = false,
            other => return Err(format!("布尔值应为 true / false（拒绝: {other}）")),
        },
        "theme_color" => cfg.theme_color = v.into(),
        other => return Err(format!("未知键: {other}")),
    }
    Ok(())
}

fn thinking_label(t: ThinkingDisplay) -> &'static str {
    match t {
        ThinkingDisplay::Show => "show",
        ThinkingDisplay::Hide => "hide",
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p router --test settings_handler_test`
Expected: PASS for all 3 new tests.

- [ ] **Step 5: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add router/src/router/inbound.rs router/tests/settings_handler_test.rs
git commit -m "feat(router): /settings command handler with live config update"
```

---

### Task 8: Wire settings.json merge into startup (`src/run.rs`)

**Files:**
- Modify: `src/run.rs:55-58` (replace the `new_with_config` call)

- [ ] **Step 1: Update `run.rs` to merge settings.json**

Replace:

```rust
let (router, mut out_rx) =
    RouterHandle::new_with_config(map, cfg.card.clone(), cfg.router.channel_buffer);
```

with:

```rust
// TOML is bootstrap; settings.json (if present) wins wholesale.
// Strict: malformed settings.json refuses to start with a clear error.
let merged_card_cfg = match router::settings::load_settings(&router::settings::settings_path())
{
    Ok(s) => s,
    Err(e) => {
        error!(error = %e, "settings.json 解析失败，拒绝启动");
        return Err(crate::error::SebasError::Config(e));
    }
};
let (router, mut out_rx) =
    RouterHandle::new_with_config(map, merged_card_cfg, cfg.router.channel_buffer);
```

Add `use router::settings;` if not already present (likely need to add `use router;`).

- [ ] **Step 2: Build**

Run: `cargo build --workspace`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add src/run.rs
git commit -m "feat(run): load settings.json on startup (strict parse, refuse on error)"
```

---

### Task 9: Document new field in `config.toml.example`

**Files:**
- Modify: `config/config.toml.example:23-27`

- [ ] **Step 1: Add `thinking` to the `[card]` section comment**

Replace the existing `[card]` block:

```toml
# ── card（卡片流配置，全部可选，见 2026-07-26-sebas-design.md §6.2）───
# [card]
# max_user_text_chars = 4000     # 单元素文本软上限：超过则截断 + 灰注
# max_tool_output_chars = 0      # 0 = 不输出 tool call 结果内容（默认）；>0 时超过则折叠进工具面板
# fold_long_output = true        # true：tool call 折叠成 collapsible_panel（默认收起）；false：内联全文
# thinking = "show"              # 模型思考内容的卡片展示：show（折叠面板，默认）/ hide（不渲染）
```

- [ ] **Step 2: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add config/config.toml.example
git commit -m "docs(config): document card.thinking field"
```

---

### Task 10: Workspace-wide verification

**Files:** none — verification only.

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace`
Expected: all tests pass (existing + new).

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings. Fix any surfaced.

- [ ] **Step 3: Smoke-test settings.json write manually**

```bash
cat > /tmp/settings-smoke.json <<EOF
{
  "theme_color": "blue",
  "max_user_text_chars": 4000,
  "max_tool_output_chars": 0,
  "fold_long_output": true,
  "thinking": "hide"
}
EOF
# Stage it in the expected path:
mkdir -p ~/.sebas && cp /tmp/settings-smoke.json ~/.sebas/settings.json
# Run the bot briefly with `cargo run --bin sebas` (or use the existing
# SEBAS_* env vars to point at a dev config). Confirm it boots without
# error and `/settings` from a Feishu chat shows `thinking = hide`.
```

- [ ] **Step 4: Final commit if any clippy / format fixes were needed**

```bash
git add -A
git commit -m "chore: post-implementation clippy + format fixes"
```

---

## Self-Review

**1. Spec coverage:**
- ✅ thinking folding (Q1, Q8) → Task 2
- ✅ `show` / `hide` only, `disable` not exposed → Task 1 enum, Task 7 parser rejection
- ✅ `/settings` command (Q4-C, Q5) → Tasks 4, 7
- ✅ plain text reply (Q6-a) → Tasks 6, 7
- ✅ settings.json overlay semantics (Q9) → Task 3
- ✅ strict parse (Q7-B) → Tasks 3, 8
- ✅ full-snapshot write (Q9-A) → Task 3 `save_settings`
- ✅ live config update (design) → Tasks 5, 7
- ✅ `$HOME/.sebas/settings.json` (user) → Task 3 `settings_path`
- ✅ pretty-printed JSON → Task 3 `to_string_pretty`
- ✅ TOML documentation → Task 9

**2. Placeholder scan:** no TBD / TODO / "implement later" markers.

**3. Type consistency:** `ThinkingDisplay` (PascalCase) ↔ `"show"`/`"hide"` (lowercase) via `#[serde(rename_all = "lowercase")]`. `Command::Settings(Option<String>, Option<String>)` matches the parser. `Out::PlainText { key, content }` is consistent in mod.rs / inbound.rs / dispatch.rs. `apply_setting` returns `Result<(), String>` consistently.

**4. Out-of-scope but flagged:**
- `/help` text content (the current `Out::HelpText` is a no-op log line). Mentioned in Task 6 comment but not implemented — out of scope per user "顺便开发个小功能".
- `disable` mode: reserved enum value, not exposed — user explicitly deferred.