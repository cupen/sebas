# sebas Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build sebas — a Rust daemon that bridges Claude Code (via ACP) to Feishu, with rich card/reaction/button UX, multi-session isolation per Feishu chat, and interactive permission approval.

**Architecture:** Single binary + 3 sibling crates (`router/`, `feishu/`, `acp-claude/`). In-memory `SessionKey → AcpSessionHandle` mapping. Feishu events flow through `mpsc` channels to router, which dispatches to ACP subprocesses via stdio JSON-RPC. ACP notifications stream back as card updates to Feishu.

**Tech Stack:**
- Rust 1.75+ (workspace)
- `tokio` 1.x (async runtime)
- `agent-client-protocol` (ACP rust-sdk)
- `reqwest` + `tokio-tungstenite` (Feishu HTTP + long-connection WS)
- `serde` / `serde_json` / `toml` / `thiserror` / `anyhow` / `tracing`
- `clap` (CLI args)
- `dirs` (home dir expansion)
- `insta` (snapshot testing)

Spec: `docs/superpowers/specs/2026-07-26-sebas-design.md`

## Global Constraints

- Only `app_id` / `app_secret` / `owner_id` are required config; **all other fields MUST have defaults** (per spec §6).
- All paths in error messages and config must use `~` shorthand; expansion happens in `config.rs`.
- `core.sshCommand` for `sebas` repo at `/home/bot/workbench/repos/sebas/.git/config` already configured.
- Crate / directory name: `acp-claude`. TOML section: `[acp.claude]`. Rust identifier: `acp_claude`.
- Branching: 3+ commits → self-named `feat/xxxx` branch, user merges to main.
- Commit messages: one sentence.
- Permission flow: never time out; hold child stdout until user replies.
- Emoji reactions: only on root card (one per session), not on every tool.
- Test coverage: `router/` ≥ 90%, `cards.rs` ≥ 90%, overall ≥ 80%.

---

## File Structure (created by Task 1, populated by later tasks)

```
sebas/
├── Cargo.toml                        ← workspace + binary
├── src/                              ← binary
│   ├── main.rs
│   ├── config.rs
│   └── error.rs
├── router/                           ← sibling crate
│   ├── Cargo.toml
│   └── src/lib.rs
├── feishu/                           ← sibling crate
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs
│       ├── cards.rs
│       └── media.rs
├── acp-claude/                       ← sibling crate
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs
│       └── session.rs
├── config/
│   └── sebas.toml.example
├── tests/
│   ├── fixtures/
│   │   └── acp/
│   │       └── basic_session.jsonl
│   └── bin/
│       └── fake-claude.rs
└── docs/superpowers/
    ├── specs/2026-07-26-sebas-design.md
    └── plans/2026-07-26-sebas-impl.md
```

---

## Task 1: Workspace skeleton

**Files:**
- Create: `Cargo.toml` (workspace + binary)
- Create: `src/main.rs` (hello-world binary)
- Create: `src/error.rs` (empty stub for now)
- Create: `src/config.rs` (empty stub for now)
- Create: `router/Cargo.toml` + `router/src/lib.rs`
- Create: `feishu/Cargo.toml` + `feishu/src/lib.rs`
- Create: `acp-claude/Cargo.toml` + `acp-claude/src/lib.rs`
- Create: `.gitignore`
- Create: `config/sebas.toml.example`

- [ ] **Step 1: Write `.gitignore`**

```
target/
**/*.rs.bk
Cargo.lock.bak
sebas.toml
```

- [ ] **Step 2: Write root `Cargo.toml` (workspace + binary)**

```toml
[workspace]
resolver = "2"
members = ["router", "feishu", "acp-claude"]

[package]
name = "sebas"
version = "0.1.0"
edition = "2021"
rust-version = "1.75"

[dependencies]
router = { path = "router" }
feishu = { path = "feishu" }
acp-claude = { path = "acp-claude" }
tokio = { version = "1.40", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
thiserror = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4.5", features = ["derive"] }
dirs = "5"

[[bin]]
name = "sebas"
path = "src/main.rs"
```

- [ ] **Step 3: Write sibling crate manifests (stubbed)**

`router/Cargo.toml`:
```toml
[package]
name = "router"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1.40", features = ["sync", "macros", "rt-multi-thread"] }
tracing = "0.1"
thiserror = "1"
```

`feishu/Cargo.toml`:
```toml
[package]
name = "feishu"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1.40", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "stream", "multipart"] }
tokio-tungstenite = { version = "0.24", features = ["connect-hyper"] }
tracing = "0.1"
thiserror = "1"
anyhow = "1"
```

`acp-claude/Cargo.toml`:
```toml
[package]
name = "acp-claude"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1.40", features = ["process", "io-util", "sync", "macros", "rt-multi-thread", "time"] }
agent-client-protocol = "2"     # latest major (2.0.0+ on crates.io)
tracing = "0.1"
thiserror = "1"
anyhow = "1"
```

Each `src/lib.rs`:
```rust
// stub — populated in later tasks
```

- [ ] **Step 4: Write `src/main.rs` (hello world)**

```rust
fn main() {
    println!("sebas v{}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 5: Build and run**

Run: `cargo build`
Expected: `Compiling sebas v0.1.0 ... Finished`

Run: `cargo run`
Expected: prints `sebas v0.1.0` and exits 0.

- [ ] **Step 6: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add .
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Bootstrap workspace with router, feishu, acp-claude crates"
```

---

## Task 2: Error types

**Files:**
- Modify: `src/error.rs` (replace stub)
- Create: `tests/error_test.rs`

- [ ] **Step 1: Write the failing test**

`tests/error_test.rs`:
```rust
use sebas::error::SebasError;

#[test]
fn error_display_includes_context() {
    let e = SebasError::Config("missing app_id".into());
    assert_eq!(e.to_string(), "config error: missing app_id");
}

#[test]
fn error_from_io() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
    let e: SebasError = io_err.into();
    assert!(matches!(e, SebasError::Io(_)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test error_test`
Expected: compile error — module `error` not found in `sebas`.

- [ ] **Step 3: Implement error types**

`src/error.rs`:
```rust
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SebasError {
    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("feishu error: {0}")]
    Feishu(String),

    #[error("acp error: {0}")]
    Acp(String),

    #[error("router error: {0}")]
    Router(String),
}

pub type Result<T> = std::result::Result<T, SebasError>;
```

Add to `src/main.rs`:
```rust
pub mod error;

fn main() {
    println!("sebas v{}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test error_test`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/error.rs src/main.rs tests/error_test.rs
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add SebasError enum with thiserror"
```

---

## Task 3: Config — schema, defaults, load, validate

**Files:**
- Modify: `src/config.rs` (replace stub)
- Create: `tests/config_test.rs`
- Modify: `src/main.rs` (declare module)
- Modify: `config/sebas.toml.example`

- [ ] **Step 1: Write the failing test**

`tests/config_test.rs`:
```rust
use sebas::config::Config;

#[test]
fn minimal_config_loads_with_defaults() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"
owner_id = "ou_x"
"#;
    let cfg = Config::parse(toml).expect("parse");
    assert_eq!(cfg.feishu.app_id, "cli_x");
    // defaults filled
    assert_eq!(cfg.acp_claude.idle_kill_secs, 172800);
    assert_eq!(cfg.router.max_concurrent_sessions, 32);
    assert_eq!(cfg.log.level, "info");
    assert!(matches!(cfg.log.file, None));
}

#[test]
fn missing_required_field_errors() {
    let toml = r#"
[feishu]
app_id = "cli_x"
"#;
    let r = Config::parse(toml);
    assert!(r.is_err());
    let msg = r.unwrap_err().to_string();
    assert!(msg.contains("app_secret") || msg.contains("owner_id"));
}

#[test]
fn overrides_apply() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"
owner_id = "ou_x"

[acp.claude]
idle_kill_secs = 60

[log]
level = "debug"
"#;
    let cfg = Config::parse(toml).unwrap();
    assert_eq!(cfg.acp_claude.idle_kill_secs, 60);
    assert_eq!(cfg.log.level, "debug");
}

#[test]
fn tilde_expansion_in_default_paths() {
    let toml = r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"
owner_id = "ou_x"
"#;
    let cfg = Config::parse(toml).unwrap();
    assert!(cfg.router.state_file.starts_with(std::env::var("HOME").unwrap_or_default()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test config_test`
Expected: compile error — module `config` not found.

- [ ] **Step 3: Implement `Config`**

`src/config.rs`:
```rust
use crate::error::{Result, SebasError};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub feishu: FeishuConfig,
    #[serde(default, rename = "acp.claude")]
    pub acp_claude: AcpClaudeConfig,
    #[serde(default)]
    pub router: RouterConfig,
    #[serde(default)]
    pub card: CardConfig,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub log: LogConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: String,
    pub owner_id: String,
    #[serde(default = "default_chat_types")]
    pub allowed_chat_types: Vec<String>,
}

impl Default for FeishuConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_secret: String::new(),
            owner_id: String::new(),
            allowed_chat_types: default_chat_types(),
        }
    }
}

fn default_chat_types() -> Vec<String> {
    vec!["private".into(), "group".into()]
}

#[derive(Debug, Clone, Deserialize)]
pub struct AcpClaudeConfig {
    #[serde(default = "default_claude_path")]
    pub path: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_sessions_dir")]
    pub sessions_dir: String,
    #[serde(default)]
    pub work_dir: Option<String>,
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_secs: u64,
    #[serde(default = "default_idle_kill")]
    pub idle_kill_secs: u64,
}

impl Default for AcpClaudeConfig {
    fn default() -> Self {
        Self {
            path: default_claude_path(),
            args: vec![],
            sessions_dir: default_sessions_dir(),
            work_dir: None,
            startup_timeout_secs: default_startup_timeout(),
            idle_kill_secs: default_idle_kill(),
        }
    }
}

fn default_claude_path() -> String { "claude".into() }
fn default_sessions_dir() -> String { "~/.claude/sessions".into() }
fn default_startup_timeout() -> u64 { 30 }
fn default_idle_kill() -> u64 { 172800 }

#[derive(Debug, Clone, Deserialize)]
pub struct RouterConfig {
    #[serde(default = "default_state_file")]
    pub state_file: String,
    #[serde(default = "default_channel_buffer")]
    pub channel_buffer: usize,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_sessions: usize,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            state_file: default_state_file(),
            channel_buffer: default_channel_buffer(),
            max_concurrent_sessions: default_max_concurrent(),
        }
    }
}

fn default_state_file() -> String { "~/.config/sebas/sessions.json".into() }
fn default_channel_buffer() -> usize { 256 }
fn default_max_concurrent() -> usize { 32 }

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

fn default_theme_color() -> String { "blue".into() }
fn default_max_user_text() -> usize { 4000 }
fn default_max_tool_output() -> usize { 2000 }
fn default_true() -> bool { true }

#[derive(Debug, Clone, Deserialize)]
pub struct MediaConfig {
    #[serde(default = "default_download_dir")]
    pub download_dir: String,
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir(),
            max_file_size: default_max_file_size(),
        }
    }
}

fn default_download_dir() -> String { "~/.cache/sebas/downloads".into() }
fn default_max_file_size() -> u64 { 52_428_800 }

#[derive(Debug, Clone, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub file: Option<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self { level: default_log_level(), file: None }
    }
}

fn default_log_level() -> String { "info".into() }

impl Config {
    pub fn parse(s: &str) -> Result<Self> {
        let cfg: Config = toml::from_str(s)
            .map_err(|e| SebasError::Config(format!("toml parse: {e}")))?;
        cfg.validate()?;
        Ok(cfg.with_expanded_paths())
    }

    fn validate(&self) -> Result<()> {
        if self.feishu.app_id.is_empty() {
            return Err(SebasError::Config("feishu.app_id is required".into()));
        }
        if self.feishu.app_secret.is_empty() {
            return Err(SebasError::Config("feishu.app_secret is required".into()));
        }
        if self.feishu.owner_id.is_empty() {
            return Err(SebasError::Config("feishu.owner_id is required".into()));
        }
        Ok(())
    }

    fn with_expanded_paths(mut self) -> Self {
        self.router.state_file = expand_tilde(&self.router.state_file);
        self.acp_claude.sessions_dir = expand_tilde(&self.acp_claude.sessions_dir);
        self.media.download_dir = expand_tilde(&self.media.download_dir);
        if let Some(ref wd) = self.acp_claude.work_dir {
            self.acp_claude.work_dir = Some(expand_tilde(wd));
        }
        if let Some(ref f) = self.log.file {
            self.log.file = Some(expand_tilde(f));
        }
        self
    }
}

pub fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into();
        }
    }
    p.to_string()
}
```

`src/main.rs`:
```rust
pub mod config;
pub mod error;

fn main() {
    println!("sebas v{}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test config_test`
Expected: 4 passed.

- [ ] **Step 5: Write example config**

`config/sebas.toml.example`:
```toml
# sebas minimum config — only 3 required fields below.
# All other sections/fields have defaults; omit them entirely.
[feishu]
app_id = "cli_xxx"
app_secret = "..."
owner_id = "ou_xxx"
```

- [ ] **Step 6: Commit**

```bash
git add src/config.rs src/main.rs tests/config_test.rs config/sebas.toml.example
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add Config with defaults, validation, tilde expansion"
```

---

## Task 4: ACP subprocess spawn + stdio I/O

**Files:**
- Modify: `acp-claude/src/lib.rs`
- Create: `acp-claude/src/client.rs`
- Create: `acp-claude/tests/spawn_test.rs`

- [ ] **Step 1: Write the failing test**

`acp-claude/tests/spawn_test.rs`:
```rust
use std::process::Stdio;
use tokio::process::Command;

#[tokio::test]
async fn echo_subprocess_round_trip() {
    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut s = stdin;
        s.write_all(b"hello\n").await.unwrap();
    });

    let mut buf = vec![0u8; 6];
    tokio::io::AsyncReadExt::read_exact(&mut stdout, &mut buf).await.unwrap();
    assert_eq!(&buf, b"hello\n");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p acp-claude --test spawn_test`
Expected: compile error — module structure not present.

- [ ] **Step 3: Implement `AcpClient::spawn`**

`acp-claude/src/lib.rs`:
```rust
pub mod client;
pub mod session;

pub use client::AcpClient;
pub use session::{AcpEvent, AcpCommand, AcpSessionHandle};
```

`acp-claude/src/client.rs`:
```rust
use crate::session::{AcpCommand, AcpEvent, AcpSessionHandle};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub claude_path: String,
    pub claude_args: Vec<String>,
    pub work_dir: Option<String>,
}

pub struct AcpClient;

impl AcpClient {
    pub fn spawn(cfg: &SpawnConfig) -> std::io::Result<(AcpSessionHandle, mpsc::Receiver<AcpCommand>)> {
        let mut cmd = Command::new(&cfg.claude_path);
        cmd.args(&cfg.claude_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(ref wd) = cfg.work_dir {
            cmd.current_dir(wd);
        }
        let child: Child = cmd.spawn()?;
        Ok(handle_child(child))
    }
}

fn handle_child(mut child: Child) -> (AcpSessionHandle, mpsc::Receiver<AcpCommand>) {
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let stdin = child.stdin.take().expect("stdin piped");

    let (cmd_tx, cmd_rx) = mpsc::channel::<AcpCommand>(64);
    let (evt_tx, evt_rx) = mpsc::channel::<AcpEvent>(256);

    // stdout reader → events
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match serde_json::from_str::<AcpEvent>(&line) {
                Ok(ev) => { let _ = evt_tx.send(ev).await; }
                Err(e) => tracing::warn!(?e, raw=%line, "failed to parse acp stdout line"),
            }
        }
    });

    // stderr → tracing
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "acp_stderr", "{line}");
        }
    });

    // cmd_rx → stdin writer
    let stdin_task = tokio::spawn(async move {
        let mut s = stdin;
        while let Some(cmd) = cmd_rx.recv().await {
            match serde_json::to_string(&cmd) {
                Ok(mut json) => {
                    json.push('\n');
                    if let Err(e) = s.write_all(json.as_bytes()).await {
                        tracing::error!(?e, "failed to write to acp stdin");
                        break;
                    }
                }
                Err(e) => tracing::error!(?e, "failed to serialize acp command"),
            }
        }
    });

    (
        AcpSessionHandle {
            child_id: format!("{:?}", child.id()),
            cmd_tx,
            evt_rx,
            _stdin_task: stdin_task,
        },
        // unused direct receiver for test compatibility — moved into handle
        mpsc::channel(1).1,
    )
}
```

`acp-claude/src/session.rs` (initial — populated more in Task 5):
```rust
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionId(pub String);

#[derive(Debug)]
pub struct AcpSessionHandle {
    pub child_id: String,
    pub cmd_tx: mpsc::Sender<AcpCommand>,
    pub evt_rx: mpsc::Receiver<AcpEvent>,
    pub _stdin_task: JoinHandle<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpCommand {
    CreateSession { session_id: String, prompt: String },
    ContinueSession { session_id: String, prompt: String },
    Cancel { session_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpEvent {
    TextDelta { session_id: String, delta: String },
    Finished { session_id: String },
    Error { session_id: String, message: String },
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p acp-claude --test spawn_test`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add acp-claude/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add acp-claude subprocess spawn with stdio JSON-RPC pump"
```

---

## Task 5: ACP notification stream parsing

**Files:**
- Modify: `acp-claude/src/session.rs` (extend AcpEvent enum)
- Create: `acp-claude/tests/parse_test.rs`
- Create: `tests/fixtures/acp/basic_session.jsonl`

- [ ] **Step 1: Write fixture file**

`tests/fixtures/acp/basic_session.jsonl`:
```
{"type":"text_delta","session_id":"s1","delta":"hello "}
{"type":"text_delta","session_id":"s1","delta":"world"}
{"type":"tool_start","session_id":"s1","tool_name":"Read","args":{"file":"a.rs"}}
{"type":"tool_end","session_id":"s1","tool_name":"Read","result":"...content..."}
{"type":"permission_request","session_id":"s1","request_id":"r1","tool_name":"Bash","args":{"cmd":"ls"}}
{"type":"text_delta","session_id":"s1","delta":"done"}
{"type":"finished","session_id":"s1"}
```

- [ ] **Step 2: Write the failing test**

`acp-claude/tests/parse_test.rs`:
```rust
use acp_claude::session::AcpEvent;

#[test]
fn parses_full_session_stream() {
    let raw = include_str!("../../tests/fixtures/acp/basic_session.jsonl");
    let events: Vec<AcpEvent> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("parse"))
        .collect();
    assert_eq!(events.len(), 7);

    match &events[0] {
        AcpEvent::TextDelta { session_id, delta } => {
            assert_eq!(session_id, "s1");
            assert_eq!(delta, "hello ");
        }
        _ => panic!("expected text_delta"),
    }

    match &events[2] {
        AcpEvent::ToolStart { tool_name, .. } => assert_eq!(tool_name, "Read"),
        _ => panic!("expected tool_start"),
    }

    match &events[4] {
        AcpEvent::PermissionRequest { request_id, tool_name, .. } => {
            assert_eq!(request_id, "r1");
            assert_eq!(tool_name, "Bash");
        }
        _ => panic!("expected permission_request"),
    }

    matches!(events.last(), Some(AcpEvent::Finished { .. }));
}
```

- [ ] **Step 3: Extend `AcpEvent` enum**

`acp-claude/src/session.rs`:
```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionId(pub String);

#[derive(Debug)]
pub struct AcpSessionHandle {
    pub child_id: String,
    pub cmd_tx: mpsc::Sender<AcpCommand>,
    pub evt_rx: mpsc::Receiver<AcpEvent>,
    pub _stdin_task: JoinHandle<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpCommand {
    CreateSession    { session_id: String, prompt: String },
    ContinueSession  { session_id: String, prompt: String },
    PermissionReply  { session_id: String, request_id: String, decision: Decision },
    Cancel           { session_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    AllowSession,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpEvent {
    TextDelta         { session_id: String, delta: String },
    ThinkingDelta     { session_id: String, delta: String },
    ToolStart         { session_id: String, tool_name: String, args: Value },
    ToolProgress      { session_id: String, tool_name: String, progress: String },
    ToolEnd           { session_id: String, tool_name: String, result: String },
    PermissionRequest { session_id: String, request_id: String, tool_name: String, args: Value },
    Finished          { session_id: String },
    Error             { session_id: String, message: String },
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p acp-claude --test parse_test`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add acp-claude/ tests/fixtures/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add AcpEvent enum with permission flow variants"
```

---

## Task 6: ACP session lifecycle — create / resume / kill / idle

**Files:**
- Modify: `acp-claude/src/session.rs`
- Create: `acp-claude/src/manager.rs`
- Create: `acp-claude/tests/lifecycle_test.rs`

- [ ] **Step 1: Write the failing test**

`acp-claude/tests/lifecycle_test.rs`:
```rust
use acp_claude::manager::SessionManager;
use std::time::Duration;

#[tokio::test]
async fn create_and_kill() {
    let mgr = SessionManager::new();
    let id = mgr
        .create_session("/bin/cat", vec![], None, "hello".into())
        .await
        .expect("spawn cat");
    tokio::time::sleep(Duration::from_millis(100)).await;
    mgr.kill(&id).await;
}

#[tokio::test]
async fn kill_unknown_is_noop() {
    let mgr = SessionManager::new();
    mgr.kill("nope").await;  // must not panic
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p acp-claude --test lifecycle_test`
Expected: compile error — `manager` module missing.

- [ ] **Step 3: Implement `SessionManager`**

`acp-claude/src/session.rs` — add at the end:
```rust
pub struct SessionMeta {
    pub session_id: String,
    pub handle: AcpSessionHandle,
}
```

`acp-claude/src/manager.rs`:
```rust
use crate::client::{spawn, SpawnConfig};
use crate::session::{AcpSessionHandle, AcpCommand, SessionMeta};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SessionManager {
    inner: Arc<Mutex<HashMap<String, SessionMeta>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub async fn create_session(
        &self,
        claude_path: &str,
        args: Vec<String>,
        work_dir: Option<String>,
        _prompt: String,
    ) -> anyhow::Result<String> {
        let cfg = SpawnConfig {
            claude_path: claude_path.to_string(),
            claude_args: args,
            work_dir,
        };
        let session_id = uuid::Uuid::new_v4().to_string();
        let handle = spawn(&cfg)?;
        self.inner.lock().await.insert(
            session_id.clone(),
            SessionMeta { session_id: session_id.clone(), handle },
        );
        Ok(session_id)
    }

    pub async fn kill(&self, session_id: &str) {
        let meta = self.inner.lock().await.remove(session_id);
        if let Some(m) = meta {
            drop(m.handle.cmd_tx);  // closing tx causes stdin task to exit
        }
    }

    pub async fn send(&self, session_id: &str, cmd: AcpCommand) -> anyhow::Result<()> {
        let g = self.inner.lock().await;
        let m = g.get(session_id).ok_or_else(|| anyhow::anyhow!("unknown session"))?;
        m.handle.cmd_tx.send(cmd).await?;
        Ok(())
    }
}
```

`acp-claude/src/lib.rs`:
```rust
pub mod client;
pub mod manager;
pub mod session;

pub use manager::SessionManager;
```

Add to `acp-claude/Cargo.toml`:
```toml
uuid = { version = "1", features = ["v4"] }
anyhow = "1"
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p acp-claude`
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add acp-claude/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add SessionManager with create/kill lifecycle"
```

---

## Task 7: Feishu long-connection client init

**Files:**
- Modify: `feishu/src/lib.rs`
- Create: `feishu/src/client.rs`
- Create: `feishu/tests/connect_test.rs` (mocked — uses reqwest mock or skipped if no test infra)

> Real network test skipped in CI; structure verified by build.

- [ ] **Step 1: Add deps**

Add to `feishu/Cargo.toml`:
```toml
futures-util = "0.3"
url = "2"
```

- [ ] **Step 2: Implement client init**

`feishu/src/lib.rs`:
```rust
pub mod cards;
pub mod client;
pub mod media;

pub use client::{FeishuClient, FeishuConfig};
```

`feishu/src/client.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: String,
    pub owner_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuToken {
    pub access_token: String,
    pub expires_at: i64, // unix seconds
}

/// Placeholder struct — actual WS connection is built in Task 8.
pub struct FeishuClient {
    pub config: FeishuConfig,
}

impl FeishuClient {
    pub fn new(config: FeishuConfig) -> Self {
        Self { config }
    }

    /// Fetches tenant_access_token via HTTP (one-shot, not on hot path).
    pub async fn fetch_token(&self, http: &reqwest::Client) -> anyhow::Result<FeishuToken> {
        let url = "https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal";
        let body = serde_json::json!({
            "app_id": self.config.app_id,
            "app_secret": self.config.app_secret,
        });
        let resp: TokenResponse = http.post(url).json(&body).send().await?.json().await?;
        if resp.code != 0 {
            anyhow::bail!("feishu auth failed: code={} msg={}", resp.code, resp.msg);
        }
        let expires_at = chrono::Utc::now().timestamp() + resp.expire as i64 - 60; // refresh 60s early
        Ok(FeishuToken {
            access_token: resp.tenant_access_token,
            expires_at,
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    code: i32,
    msg: String,
    tenant_access_token: String,
    expire: i64,
}
```

Add to `feishu/Cargo.toml`:
```toml
chrono = "0.4"
```

`feishu/src/cards.rs` and `feishu/src/media.rs` — stubs:
```rust
// populated in later tasks
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build -p feishu`
Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add feishu/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add Feishu client init with token fetch"
```

---

## Task 8: Feishu event ingestion (long-connection)

**Files:**
- Modify: `feishu/src/client.rs`
- Create: `feishu/src/events.rs`
- Create: `feishu/tests/event_parse_test.rs`

- [ ] **Step 1: Write the failing test**

`feishu/tests/event_parse_test.rs`:
```rust
use feishu::events::{FeishuIn, FeishuEnvelope, MessageBody};

#[test]
fn parses_text_message_event() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "tk" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_x" } },
            "message": {
                "chat_id": "oc_x",
                "chat_type": "private",
                "message_id": "om_x",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}",
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    let evt = env.into_event("ou_owner").unwrap();
    match evt {
        FeishuIn::Text { text, .. } => assert_eq!(text, "hi"),
        _ => panic!("expected Text"),
    }
}

#[test]
fn ignores_events_from_non_owner() {
    let raw = serde_json::json!({
        "schema": "2.0",
        "header": { "event_type": "im.message.receive_v1", "tenant_key": "tk" },
        "event": {
            "sender": { "sender_id": { "open_id": "ou_stranger" } },
            "message": {
                "chat_id": "oc_x",
                "chat_type": "private",
                "message_id": "om_x",
                "message_type": "text",
                "content": "{\"text\":\"hi\"}"
            }
        }
    });
    let env: FeishuEnvelope = serde_json::from_value(raw).unwrap();
    assert!(env.into_event("ou_owner").is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p feishu --test event_parse_test`
Expected: compile error — module `events` not found.

- [ ] **Step 3: Implement event types**

`feishu/src/events.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum FeishuIn {
    Text      { key: SessionKey, text: String, reply_to: Option<String> },
    Media     { key: SessionKey, files: Vec<String>, caption: Option<String> },
    ButtonCb  { key: SessionKey, action: CardAction },
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct SessionKey {
    pub chat_id: String,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CardAction {
    pub session_id: String,
    pub request_id: Option<String>,
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct FeishuEnvelope {
    pub schema: String,
    pub header: FeishuHeader,
    pub event: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct FeishuHeader {
    pub event_type: String,
}

impl FeishuEnvelope {
    /// Returns None if event is from non-owner.
    pub fn into_event(self, owner_id: &str) -> Option<FeishuIn> {
        let sender_open_id = self.event
            .pointer("/sender/sender_id/open_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if sender_open_id != owner_id {
            return None;
        }
        let chat_id = self.event.pointer("/message/chat_id")?.as_str()?.to_string();
        let chat_type = self.event.pointer("/message/chat_type")?.as_str()?.unwrap_or("private").to_string();
        let message_id = self.event.pointer("/message/message_id")?.as_str()?.to_string();
        let message_type = self.event.pointer("/message/message_type")?.as_str()?.to_string();
        let content_str = self.event.pointer("/message/content")?.as_str()?.to_string();
        let thread_id = self.event.pointer("/message/thread_id").and_then(|v| v.as_str()).map(String::from);
        let key = SessionKey { chat_id, thread_id };

        match (self.header.event_type.as_str(), message_type.as_str()) {
            ("im.message.receive_v1", "text") => {
                let body: MessageBody = serde_json::from_str(&content_str).ok()?;
                Some(FeishuIn::Text { key, text: body.text.unwrap_or_default(), reply_to: Some(message_id) })
            }
            ("im.message.receive_v1", "image" | "file" | "audio") => {
                Some(FeishuIn::Media { key, files: vec![message_id], caption: None })
            }
            ("card.action.trigger", _) => {
                // action payload schema differs; minimal parse here, real impl in Task 16
                Some(FeishuIn::ButtonCb {
                    key,
                    action: CardAction { session_id: String::new(), request_id: None, value: self.event },
                })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct MessageBody {
    pub text: Option<String>,
}
```

`feishu/src/lib.rs`:
```rust
pub mod cards;
pub mod client;
pub mod events;
pub mod media;

pub use client::{FeishuClient, FeishuConfig, FeishuToken};
pub use events::{FeishuIn, FeishuEnvelope, SessionKey, CardAction, MessageBody};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p feishu --test event_parse_test`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add feishu/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add Feishu event envelope parser with owner filter"
```

---

## Task 9: Feishu outbound — message send / card update / reaction

**Files:**
- Modify: `feishu/src/client.rs`
- Create: `feishu/tests/outbound_test.rs` (parses request bodies — uses mock reqwest)

- [ ] **Step 1: Add outbound methods to `FeishuClient`**

Append to `feishu/src/client.rs`:
```rust
use crate::events::SessionKey;

impl FeishuClient {
    pub async fn send_card(
        &self,
        http: &reqwest::Client,
        token: &str,
        key: &SessionKey,
        card_json: serde_json::Value,
    ) -> anyhow::Result<String> {
        let receive_id = key.chat_id.clone();
        let url = if key.thread_id.is_some() {
            "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id"
        } else {
            "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id"
        };
        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": "interactive",
            "content": serde_json::to_string(&card_json)?,
        });
        let resp: ApiResponse<MessageOut> = http
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("send_card failed: {} {}", resp.code, resp.msg);
        }
        Ok(resp.data.message_id)
    }

    pub async fn update_card(
        &self,
        http: &reqwest::Client,
        token: &str,
        message_id: &str,
        card_json: serde_json::Value,
    ) -> anyhow::Result<()> {
        let url = format!("https://open.feishu.cn/open-apis/im/v1/messages/{message_id}");
        let body = serde_json::json!({ "content": serde_json::to_string(&card_json)? });
        let resp: ApiResponse<()> = http
            .patch(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("update_card failed: {} {}", resp.code, resp.msg);
        }
        Ok(())
    }

    pub async fn react(
        &self,
        http: &reqwest::Client,
        token: &str,
        message_id: &str,
        emoji_type: &str,
    ) -> anyhow::Result<()> {
        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/{message_id}/reactions"
        );
        let body = serde_json::json!({ "reaction_type": { "emoji_type": emoji_type } });
        let resp: ApiResponse<serde_json::Value> = http
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if resp.code != 0 {
            anyhow::bail!("react failed: {} {}", resp.code, resp.msg);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    code: i32,
    msg: String,
    #[serde(default)]
    data: T,
    #[serde(default)]
    #[allow(non_snake_case)]
    message_id: Option<String>,
}

#[derive(Deserialize)]
struct MessageOut {
    #[allow(non_snake_case)]
    message_id: Option<String>,
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p feishu`
Expected: compiles cleanly. (Outbound HTTP calls are network-dependent — covered by manual smoke test, not unit tests.)

- [ ] **Step 3: Commit**

```bash
git add feishu/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add Feishu outbound send_card/update_card/react"
```

---

## Task 10: Card data model + renderers (snapshot tests)

**Files:**
- Modify: `feishu/src/cards.rs`
- Create: `feishu/tests/cards_test.rs`

- [ ] **Step 1: Add insta dep**

Add to `feishu/Cargo.toml`:
```toml
insta = "1"
```

- [ ] **Step 2: Write the failing snapshot test**

`feishu/tests/cards_test.rs`:
```rust
use acp_claude::session::AcpEvent;
use feishu::cards::{render_root_card, render_permission_card};

#[test]
fn root_card_initial_snapshot() {
    let card = render_root_card("重构 src/foo.rs", "msg_1", "👀");
    insta::assert_yaml_snapshot!(card);
}

#[test]
fn root_card_after_text_delta_snapshot() {
    let mut card = render_root_card("重构 src/foo.rs", "msg_1", "🚧");
    card.push_text("我会先看一下 foo.rs 的结构。");
    insta::assert_yaml_snapshot!(card);
}

#[test]
fn permission_card_snapshot() {
    let card = render_permission_card("s1", "r1", "Bash", &serde_json::json!({"cmd": "rm -rf"}));
    insta::assert_yaml_snapshot!(card);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p feishu --test cards_test`
Expected: compile error — `render_root_card` not found.

- [ ] **Step 4: Implement card model + renderers**

`feishu/src/cards.rs`:
```rust
use acp_claude::session::AcpEvent;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub schema: String,
    pub header: CardHeader,
    pub elements: Vec<CardElement>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardHeader {
    pub title: CardTitle,
    pub template: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardTitle {
    pub content: String,
    pub tag: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "tag", rename_all = "snake_case")]
pub enum CardElement {
    Divider,
    PlainText { content: String },
    Markdown { content: String },
    Note     { elements: Vec<CardText> },
    Action   { actions: Vec<CardButton> },
}

#[derive(Debug, Clone, Serialize)]
pub struct CardText {
    pub tag: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardButton {
    pub tag: String,
    pub text: CardText,
    pub r#type: String, // "primary" | "danger" | "default"
    pub value: Value,
}

impl Card {
    pub fn new(title: &str, template: &str) -> Self {
        Self {
            schema: "2.0".into(),
            header: CardHeader {
                title: CardTitle { content: title.into(), tag: "plain_text".into() },
                template: template.into(),
            },
            elements: vec![],
        }
    }

    pub fn push_text(&mut self, content: impl Into<String>) {
        self.elements.push(CardElement::Markdown { content: content.into() });
    }

    pub fn push_note(&mut self, content: impl Into<String>) {
        self.elements.push(CardElement::Note {
            elements: vec![CardText { tag: "plain_text".into(), content: content.into() }],
        });
    }

    pub fn push_divider(&mut self) {
        self.elements.push(CardElement::Divider);
    }

    pub fn push_actions(&mut self, actions: Vec<CardButton>) {
        self.elements.push(CardElement::Action { actions });
    }
}

pub fn render_root_card(user_prompt: &str, msg_id: &str, status_emoji: &str) -> Card {
    let mut card = Card::new(&format!("{status_emoji} Claude Code"), "blue");
    card.push_text(format!("> {user_prompt}"));
    card.push_divider();
    card.push_note(format!("msg_id: {msg_id}"));
    card
}

pub fn render_permission_card(
    session_id: &str,
    request_id: &str,
    tool_name: &str,
    args: &Value,
) -> Card {
    let mut card = Card::new("⚠ 权限请求", "orange");
    card.push_text(format!("**{tool_name}** 想要执行："));
    card.push_note(serde_json::to_string_pretty(args).unwrap_or_default());
    let btn = |label: &str, kind: &str, decision: &str| CardButton {
        tag: "button".into(),
        text: CardText { tag: "plain_text".into(), content: label.into() },
        r#type: kind.into(),
        value: serde_json::json!({
            "session_id": session_id,
            "request_id": request_id,
            "decision": decision,
        }),
    };
    card.push_actions(vec![
        btn("Allow once", "primary", "allow_once"),
        btn("Allow session", "default", "allow_session"),
        btn("Deny", "danger", "deny"),
    ]);
    card
}

pub fn apply_event(card: &mut Card, event: &AcpEvent) {
    match event {
        AcpEvent::TextDelta { delta, .. } => card.push_text(delta.clone()),
        AcpEvent::ThinkingDelta { delta, .. } => card.push_note(format!("💭 {delta}")),
        AcpEvent::ToolStart { tool_name, args, .. } => {
            card.push_divider();
            card.push_text(format!("📖 **{tool_name}** `{}`", args));
        }
        AcpEvent::ToolEnd { tool_name, result, .. } => {
            card.push_note(format!("✓ {tool_name} done: {}", truncate(result, 200)));
        }
        AcpEvent::PermissionRequest { tool_name, args, .. } => {
            card.push_text(format!("⏸ waiting for permission: {tool_name} `{}`", args));
        }
        AcpEvent::Finished { .. } => card.push_text("✅ 完成"),
        AcpEvent::Error { message, .. } => card.push_text(format!("❌ {message}")),
        AcpEvent::ToolProgress { tool_name, progress, .. } => {
            card.push_note(format!("⏳ {tool_name}: {progress}"));
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}
```

- [ ] **Step 5: Run tests; accept snapshots**

Run: `cargo test -p feishu --test cards_test`
First run will print snapshot diffs. To accept:
Run: `cargo insta accept`
Expected: 3 snapshots accepted.

- [ ] **Step 6: Commit**

```bash
git add feishu/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add Card model + root/permission renderers with snapshot tests"
```

---

## Task 11: Media download from Feishu

**Files:**
- Modify: `feishu/src/media.rs`
- Create: `feishu/tests/media_test.rs`

- [ ] **Step 1: Write the failing test**

`feishu/tests/media_test.rs`:
```rust
use feishu::media::{download_file, MediaMeta};
use std::path::PathBuf;

#[tokio::test]
async fn download_writes_to_target_path() {
    // Mocks via mockito-style server skipped here; verify by build + integration.
    // Test only the path-composition helper:
    let meta = MediaMeta { file_key: "fk".into(), file_name: "a.png".into(), mime: None };
    let dest = download_file::compose_dest(PathBuf::from("/tmp/dl"), &meta);
    assert_eq!(dest, PathBuf::from("/tmp/dl/a.png"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p feishu --test media_test`
Expected: compile error — `compose_dest` not found.

- [ ] **Step 3: Implement download**

`feishu/src/media.rs`:
```rust
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct MediaMeta {
    pub file_key: String,
    pub file_name: String,
    #[serde(default)]
    pub mime: Option<String>,
}

pub mod download_file {
    use super::*;

    pub fn compose_dest(dir: PathBuf, meta: &MediaMeta) -> PathBuf {
        dir.join(&meta.file_name)
    }

    /// Downloads a media file from Feishu to `dest`. Network-dependent.
    pub async fn download(
        http: &reqwest::Client,
        token: &str,
        file_key: &str,
        dest: &Path,
    ) -> anyhow::Result<()> {
        // 1. GET /im/v1/messages/{message_id}/resources/{file_key} → redirect URL or stream
        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/msg/resources/{file_key}"
        );
        let bytes = http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(dest, &bytes).await?;
        Ok(())
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p feishu --test media_test`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add feishu/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add Feishu media download helper"
```

---

## Task 12: Router — SessionKey map + state dump/restore

**Files:**
- Modify: `router/src/lib.rs`
- Create: `router/src/state.rs`
- Create: `router/tests/state_test.rs`

- [ ] **Step 1: Write the failing test**

`router/tests/state_test.rs`:
```rust
use router::state::SessionMap;
use router::state::Mapping;
use feishu::events::SessionKey;

#[tokio::test]
async fn insert_and_lookup() {
    let m = SessionMap::new();
    let k = SessionKey { chat_id: "oc_x".into(), thread_id: None };
    m.insert(k.clone(), Mapping { session_id: "s1".into(), last_active_unix: 1 }).await;
    let got = m.get(&k).await;
    assert_eq!(got.unwrap().session_id, "s1");
}

#[tokio::test]
async fn dump_and_restore_round_trip() {
    let m = SessionMap::new();
    let k = SessionKey { chat_id: "oc_x".into(), thread_id: None };
    m.insert(k.clone(), Mapping { session_id: "s1".into(), last_active_unix: 1 }).await;

    let json = m.dump_json().await.unwrap();
    let m2 = SessionMap::restore_json(&json).unwrap();
    let got = m2.get(&k).await;
    assert_eq!(got.unwrap().session_id, "s1");
}

#[tokio::test]
async fn overflow_rejects() {
    let m = SessionMap::with_capacity(2);
    for i in 0..2 {
        m.insert(
            SessionKey { chat_id: format!("oc_{i}"), thread_id: None },
            Mapping { session_id: format!("s_{i}"), last_active_unix: 0 },
        ).await;
    }
    let r = m.insert(
        SessionKey { chat_id: "oc_3".into(), thread_id: None },
        Mapping { session_id: "s_3".into(), last_active_unix: 0 },
    ).await;
    assert!(r.is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p router --test state_test`
Expected: compile error.

- [ ] **Step 3: Implement state map**

`router/src/lib.rs`:
```rust
pub mod state;

pub use state::{Mapping, SessionMap};
```

`router/src/state.rs`:
```rust
use crate::error::RouterError;
use feishu::events::SessionKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mapping {
    pub session_id: String,
    pub last_active_unix: i64,
}

pub struct SessionMap {
    inner: Arc<RwLock<HashMap<SessionKey, Mapping>>>,
    capacity: usize,
}

impl SessionMap {
    pub fn new() -> Self { Self::with_capacity(usize::MAX) }
    pub fn with_capacity(cap: usize) -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())), capacity: cap }
    }

    pub async fn insert(&self, key: SessionKey, mapping: Mapping) -> Result<(), RouterError> {
        let mut g = self.inner.write().await;
        if !g.contains_key(&key) && g.len() >= self.capacity {
            return Err(RouterError::Capacity(self.capacity));
        }
        g.insert(key, mapping);
        Ok(())
    }

    pub async fn get(&self, key: &SessionKey) -> Option<Mapping> {
        self.inner.read().await.get(key).cloned()
    }

    pub async fn dump_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(&*self.inner.read().await)
    }

    pub fn restore_json(s: &str) -> serde_json::Result<Self> {
        let map: HashMap<SessionKey, Mapping> = serde_json::from_str(s)?;
        Ok(Self { inner: Arc::new(RwLock::new(map)), capacity: usize::MAX })
    }
}

impl Default for SessionMap {
    fn default() -> Self { Self::new() }
}
```

`router/src/error.rs`:
```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("router capacity {0} exceeded")]
    Capacity(usize),
}
```

Update `router/Cargo.toml` to include thiserror.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p router --test state_test`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add router/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add SessionMap with capacity limit and JSON dump/restore"
```

---

## Task 13: Router — Slash command parsing

**Files:**
- Modify: `router/src/lib.rs`
- Create: `router/src/commands.rs`
- Create: `router/tests/commands_test.rs`

- [ ] **Step 1: Write the failing test**

`router/tests/commands_test.rs`:
```rust
use router::commands::{parse_command, Command};

#[test]
fn parses_new() {
    match parse_command("/new") {
        Command::New => {},
        _ => panic!("expected New"),
    }
}

#[test]
fn parses_switch_with_arg() {
    match parse_command("/switch 3") {
        Command::Switch(3) => {},
        _ => panic!("expected Switch(3)"),
    }
}

#[test]
fn parses_sessions() {
    assert!(matches!(parse_command("/sessions"), Command::Sessions));
}

#[test]
fn parses_help() {
    assert!(matches!(parse_command("/help"), Command::Help));
}

#[test]
fn double_slash_escapes_to_prompt() {
    assert_eq!(parse_command("//compact"), Command::PassThrough("/compact".into()));
    assert_eq!(parse_command("/compact"), Command::Compact);
}

#[test]
fn unknown_command_passes_through() {
    assert!(matches!(parse_command("/foo"), Command::PassThrough(_)));
}

#[test]
fn plain_text_is_pass_through() {
    assert_eq!(parse_command("hello world"), Command::PassThrough("hello world".into()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p router --test commands_test`
Expected: compile error.

- [ ] **Step 3: Implement command parser**

`router/src/commands.rs`:
```rust
#[derive(Debug, PartialEq)]
pub enum Command {
    New,
    Sessions,
    Switch(usize),
    Resume(String),
    Cancel,
    Status,
    Compact,
    Cost,
    Model(String),
    Cd(String),
    Help,
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
        "/new"      => Command::New,
        "/sessions" => Command::Sessions,
        "/switch"   => match arg.parse::<usize>() {
            Ok(n) => Command::Switch(n),
            Err(_) => Command::PassThrough(input.into()),
        },
        "/resume"   => Command::Resume(arg.into()),
        "/cancel"   => Command::Cancel,
        "/status"   => Command::Status,
        "/compact"  => Command::Compact,
        "/cost"     => Command::Cost,
        "/model"    => Command::Model(arg.into()),
        "/cd"       => Command::Cd(arg.into()),
        "/help"     => Command::Help,
        _ => Command::PassThrough(input.into()),
    }
}
```

`router/src/lib.rs`:
```rust
pub mod commands;
pub mod error;
pub mod state;

pub use commands::{parse_command, Command};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p router --test commands_test`
Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
git add router/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add slash command parser with escape handling"
```

---

## Task 14: Router — message dispatch (FeishuIn → AcpCommand → AcpEvent → FeishuOut)

**Files:**
- Modify: `router/src/lib.rs`
- Create: `router/src/router.rs`
- Create: `router/tests/router_test.rs`

- [ ] **Step 1: Write the failing test**

`router/tests/router_test.rs`:
```rust
use router::state::{SessionMap, Mapping};
use router::router::{Router, Out};
use feishu::events::{FeishuIn, SessionKey};
use acp_claude::session::AcpEvent;
use std::time::Duration;

#[tokio::test]
async fn new_text_creates_session_and_emits_initial_card() {
    let map = SessionMap::new();
    let (router, mut out_rx) = Router::new(map.clone());
    let key = SessionKey { chat_id: "oc_x".into(), thread_id: None };

    tokio::spawn(async move {
        let _ = router
            .handle(FeishuIn::Text {
                key: key.clone(),
                text: "hello".into(),
                reply_to: None,
            })
            .await;
    });

    let first = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await.unwrap().unwrap();
    // First event is some "send_card" or "spawn acp" — we assert shape loosely:
    assert!(matches!(first, Out::SendCard { .. } | Out::SpawnAcp { .. }));
}

#[tokio::test]
async fn existing_session_dispatches_continue() {
    let map = SessionMap::new();
    let k = SessionKey { chat_id: "oc_x".into(), thread_id: None };
    map.insert(k.clone(), Mapping { session_id: "existing".into(), last_active_unix: 1 }).await;

    let (router, mut out_rx) = Router::new(map.clone());
    tokio::spawn(async move {
        let _ = router.handle(FeishuIn::Text {
            key: k.clone(),
            text: "more".into(),
            reply_to: None,
        }).await;
    });

    let out = tokio::time::timeout(Duration::from_millis(200), out_rx.recv())
        .await.unwrap().unwrap();
    match out {
        Out::SendAcp { session_id, .. } => assert_eq!(session_id, "existing"),
        other => panic!("expected SendAcp, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p router --test router_test`
Expected: compile error.

- [ ] **Step 3: Implement router dispatch**

`router/src/router.rs`:
```rust
use crate::commands::{parse_command, Command};
use crate::state::{Mapping, SessionMap};
use acp_claude::session::{AcpCommand, AcpEvent};
use feishu::cards::{apply_event, render_root_card};
use feishu::events::{FeishuIn, SessionKey};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Out {
    SpawnAcp    { key: SessionKey, prompt: String },
    SendAcp     { session_id: String, cmd: AcpCommand },
    SendCard    { key: SessionKey, card: serde_json::Value, msg_id: Option<String> },
    UpdateCard  { session_id: String, card: serde_json::Value },
    React       { session_id: String, emoji: String },
    HelpText    { key: SessionKey },
}

pub struct Router {
    map: SessionMap,
}

impl Router {
    pub fn new(map: SessionMap) -> (Self, mpsc::Receiver<Out>) {
        let (tx, rx) = mpsc::channel(256);
        (Self { map }, rx)
    }

    pub async fn handle(&self, evt: FeishuIn, tx: &mpsc::Sender<Out>) {
        match evt {
            FeishuIn::Text { key, text, .. } => self.on_text(key, text, tx).await,
            FeishuIn::Media { key, files, caption } => {
                let prompt = compose_media_prompt(&text_from_caption(&caption), &files);
                self.on_text(key, prompt, tx).await;
            }
            FeishuIn::ButtonCb { key, action } => self.on_button(key, action, tx).await,
        }
    }

    async fn on_text(&self, key: SessionKey, text: String, tx: &mpsc::Sender<Out>) {
        match parse_command(&text) {
            Command::New => self.spawn_new(key, String::new(), tx).await,
            Command::Help => { let _ = tx.send(Out::HelpText { key }).await; }
            Command::PassThrough(p) => {
                if let Some(m) = self.map.get(&key).await {
                    self.continue_session(m.session_id, p, tx).await;
                } else {
                    self.spawn_new(key, p, tx).await;
                }
            }
            Command::Compact | Command::Cost | Command::Cancel | Command::Status => {
                if let Some(m) = self.map.get(&key).await {
                    self.forward_to_session(&m.session_id, text, tx).await;
                } else {
                    let _ = tx.send(Out::HelpText { key }).await;
                }
            }
            _ => { let _ = tx.send(Out::HelpText { key }).await; }
        }
    }

    async fn on_button(&self, _key: SessionKey, action: crate::feishu_re_export::CardAction, _tx: &mpsc::Sender<Out>) {
        // implemented in Task 16
    }

    async fn spawn_new(&self, key: SessionKey, prompt: String, tx: &mpsc::Sender<Out>) {
        let _ = tx.send(Out::SpawnAcp { key: key.clone(), prompt: prompt.clone() }).await;
        let card = render_root_card(&prompt, "new", "👀");
        let _ = tx.send(Out::SendCard {
            key,
            card: serde_json::to_value(&card).unwrap(),
            msg_id: None,
        }).await;
    }

    async fn continue_session(&self, session_id: String, prompt: String, tx: &mpsc::Sender<Out>) {
        let _ = tx.send(Out::SendAcp {
            session_id: session_id.clone(),
            cmd: AcpCommand::ContinueSession { session_id, prompt },
        }).await;
    }

    async fn forward_to_session(&self, session_id: &str, text: String, tx: &mpsc::Sender<Out>) {
        let cmd = match parse_command(&text) {
            Command::Compact => AcpCommand::ContinueSession { session_id: session_id.into(), prompt: "/compact".into() },
            Command::Cost    => AcpCommand::ContinueSession { session_id: session_id.into(), prompt: "/cost".into() },
            Command::Cancel  => AcpCommand::Cancel { session_id: session_id.into() },
            _ => return,
        };
        let _ = tx.send(Out::SendAcp { session_id: session_id.into(), cmd }).await;
    }
}

pub fn compose_media_prompt(caption: &str, files: &[String]) -> String {
    let mut out = String::new();
    if !caption.is_empty() {
        out.push_str(caption);
        out.push('\n');
    }
    out.push_str("\n[attached: ");
    out.push_str(&files.join(", "));
    out.push(']');
    out
}

fn text_from_caption(c: &Option<String>) -> String {
    c.clone().unwrap_or_default()
}

pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub async fn apply_event_to_out(
    session_id: String,
    event: &AcpEvent,
    tx: &mpsc::Sender<Out>,
) {
    match event {
        AcpEvent::TextDelta { .. } | AcpEvent::ToolStart { .. } | AcpEvent::Finished { .. } | AcpEvent::Error { .. } => {
            let mut card = render_root_card("", &session_id, if matches!(event, AcpEvent::Finished { .. }) { "✅" } else { "🚧" });
            apply_event(&mut card, event);
            let _ = tx.send(Out::UpdateCard {
                session_id,
                card: serde_json::to_value(&card).unwrap(),
            }).await;
        }
        AcpEvent::PermissionRequest { .. } => {
            // Task 16 builds the permission card
        }
        _ => {}
    }
}
```

`router/src/lib.rs`:
```rust
pub mod commands;
pub mod error;
pub mod router;
pub mod state;

pub use commands::{parse_command, Command};
pub use router::{Router, Out};
pub use state::{Mapping, SessionMap};

// re-export for router.rs to avoid pulling feishu::events directly here
pub mod feishu_re_export {
    pub use feishu::events::CardAction;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p router --test router_test`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add router/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add Router dispatch with command/session routing"
```

---

## Task 15: Router — permission flow (cards with buttons, reply routing)

**Files:**
- Modify: `router/src/router.rs`
- Modify: `router/src/feishu_re_export.rs` (new)
- Create: `router/tests/permission_test.rs`

- [ ] **Step 1: Write the failing test**

`router/tests/permission_test.rs`:
```rust
use router::router::Router;
use router::state::SessionMap;
use feishu::events::{FeishuIn, SessionKey, CardAction};
use acp_claude::session::{AcpEvent, AcpCommand};

#[tokio::test]
async fn permission_request_emits_card_with_buttons() {
    let map = SessionMap::new();
    let (router, mut out_rx) = Router::new(map.clone());

    let event = AcpEvent::PermissionRequest {
        session_id: "s1".into(),
        request_id: "r1".into(),
        tool_name: "Bash".into(),
        args: serde_json::json!({"cmd": "ls"}),
    };
    tokio::spawn(async move {
        router.handle_acp_event(event).await;
    });

    let out = out_rx.recv().await.unwrap();
    match out {
        router::Out::SendCard { card, .. } => {
            let s = serde_json::to_string(&card).unwrap();
            assert!(s.contains("Allow once"));
            assert!(s.contains("Deny"));
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
}

#[tokio::test]
async fn button_callback_emits_permission_reply() {
    let map = SessionMap::new();
    let (router, mut out_rx) = Router::new(map.clone());
    let key = SessionKey { chat_id: "oc_x".into(), thread_id: None };
    let action = CardAction {
        session_id: "s1".into(),
        request_id: Some("r1".into()),
        value: serde_json::json!({ "decision": "allow_once" }),
    };

    tokio::spawn(async move {
        router.handle(FeishuIn::ButtonCb { key, action }, &mut out_rx.sender_clone()).await;
    });
}
```

Add `sender_clone()` helper:
```rust
// in router/src/router.rs
impl mpsc::Receiver<Out> {
    pub fn sender_clone(&self) -> mpsc::Sender<Out> { self.sender() }
}
```

Wait — mpsc::Receiver doesn't have `sender()`. Instead, refactor Router to expose sender. Add a new helper `RouterHandle { tx, map }`.

> Update test to use `RouterHandle`.

```rust
let (handle, mut out_rx) = RouterHandle::new(map);
tokio::spawn(async move {
    handle.handle(FeishuIn::ButtonCb { key, action }).await;
});
let out = out_rx.recv().await.unwrap();
match out {
    router::Out::SendAcp { cmd: AcpCommand::PermissionReply { .. }, .. } => {},
    other => panic!("expected SendAcp PermissionReply, got {other:?}"),
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p router --test permission_test`
Expected: compile error.

- [ ] **Step 3: Refactor Router to expose handle**

`router/src/router.rs` — replace `Router::new` with `RouterHandle::new`:
```rust
pub struct RouterHandle {
    map: SessionMap,
    tx: mpsc::Sender<Out>,
}

impl RouterHandle {
    pub fn new(map: SessionMap) -> (Self, mpsc::Receiver<Out>) {
        let (tx, rx) = mpsc::channel(256);
        (Self { map, tx }, rx)
    }
    pub async fn handle(self, evt: FeishuIn) { /* forwards self.tx */ }
    pub async fn handle_acp_event(self, evt: AcpEvent) { /* forwards self.tx */ }
}
```

Implement `on_button`:
```rust
async fn on_button(&self, key: SessionKey, action: CardAction) {
    let decision_str = action.value.get("decision").and_then(|v| v.as_str()).unwrap_or("deny");
    let decision = match decision_str {
        "allow_once" => Decision::AllowOnce,
        "allow_session" => Decision::AllowSession,
        _ => Decision::Deny,
    };
    if let (Some(sid), Some(rid)) = (Some(action.session_id), action.request_id) {
        let _ = self.tx.send(Out::SendAcp {
            session_id: sid.clone(),
            cmd: AcpCommand::PermissionReply { session_id: sid, request_id: rid, decision },
        }).await;
    } else {
        let _ = self.tx.send(Out::HelpText { key }).await;
    }
}
```

Implement `handle_acp_event` with permission card:
```rust
pub async fn handle_acp_event(&self, evt: AcpEvent) {
    match evt {
        AcpEvent::PermissionRequest { session_id, request_id, tool_name, args } => {
            let card = render_permission_card(&session_id, &request_id, &tool_name, &args);
            let _ = self.tx.send(Out::SendCard {
                key: SessionKey { chat_id: String::new(), thread_id: None },  // resolved by main loop
                card: serde_json::to_value(&card).unwrap(),
                msg_id: None,
            }).await;
        }
        _ => { /* covered by previous task */ }
    }
}
```

Note: `SendCard { key }` here uses empty SessionKey; in main loop we resolve via session_id → key. Refine if needed.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p router --test permission_test`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add router/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add permission flow with button callbacks"
```

---

## Task 16: Main entry — assembly + signal handling

**Files:**
- Modify: `src/main.rs`
- Create: `src/run.rs`

- [ ] **Step 1: Implement `run.rs`**

`src/run.rs`:
```rust
use crate::config::Config;
use crate::error::Result;
use acp_claude::manager::SessionManager;
use feishu::client::{FeishuClient, FeishuConfig};
use router::router::RouterHandle;
use router::state::SessionMap;
use std::sync::Arc;
use tokio::sync::mpsc;

pub async fn run(cfg: Config) -> Result<()> {
    init_tracing(&cfg);

    let map = SessionMap::restore_json(&std::fs::read_to_string(&cfg.router.state_file).unwrap_or_else(|_| "{}".into()))
        .map_err(|e| crate::error::SebasError::Config(format!("restore: {e}")))?;

    let (router, mut out_rx) = RouterHandle::new(map);
    let mgr = Arc::new(SessionManager::new());

    let feishu = FeishuClient::new(FeishuConfig {
        app_id: cfg.feishu.app_id.clone(),
        app_secret: cfg.feishu.app_secret.clone(),
        owner_id: cfg.feishu.owner_id.clone(),
    });

    let http = reqwest::Client::new();
    let token = feishu.fetch_token(&http).await
        .map_err(|e| crate::error::SebasError::Feishu(e.to_string()))?;

    // Spawn outbound pump
    let cfg_for_outbound = cfg.clone();
    let token_clone = token.access_token.clone();
    let http_for_outbound = http.clone();
    let feishu_for_outbound = feishu.clone_for_outbound();
    tokio::spawn(async move {
        while let Some(out) = out_rx.recv().await {
            if let Err(e) = dispatch_out(&feishu_for_outbound, &http_for_outbound, &token_clone, &cfg_for_outbound, out).await {
                tracing::error!(?e, "outbound dispatch failed");
            }
        }
    });

    // TODO Task 17: start long-connection event loop here
    tracing::info!("sebas started; waiting for SIGINT/SIGTERM");
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutting down");

    // Dump sessions on exit
    let json = router.dump_json().await.map_err(|e| crate::error::SebasError::Router(e.to_string()))?;
    std::fs::write(&cfg.router.state_file, json).ok();
    Ok(())
}

async fn dispatch_out(
    feishu: &FeishuClient,
    http: &reqwest::Client,
    token: &str,
    cfg: &Config,
    out: router::router::Out,
) -> anyhow::Result<()> {
    use router::router::Out;
    match out {
        Out::SendCard { key, card, msg_id } => {
            feishu.send_card(http, token, &key, card).await?;
        }
        Out::UpdateCard { session_id, card } => {
            // resolve session_id → msg_id via shared map (Task 17)
            tracing::debug!(?session_id, "would update card");
        }
        Out::React { session_id, emoji } => {
            tracing::debug!(?session_id, ?emoji, "would react");
        }
        Out::SpawnAcp { key, prompt } => {
            tracing::info!(?key, "spawn acp for {prompt:?}");
        }
        Out::SendAcp { session_id, cmd } => {
            tracing::debug!(?session_id, ?cmd, "would send acp command");
        }
        Out::HelpText { key } => {
            tracing::info!(?key, "send help");
        }
    }
    Ok(())
}

fn init_tracing(cfg: &Config) {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_new(&cfg.log.level).unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = fmt().with_env_filter(filter);
    if let Some(ref path) = cfg.log.file {
        let file = std::fs::File::create(path).ok();
        if let Some(f) = file {
            subscriber.with_writer(f).init();
        } else {
            subscriber.init();
        }
    } else {
        subscriber.init();
    }
}
```

Update `feishu/src/client.rs` to add `clone_for_outbound`:
```rust
impl FeishuClient {
    pub fn clone_for_outbound(&self) -> Self { self.clone() }
}
impl Clone for FeishuClient {
    fn clone(&self) -> Self { Self { config: self.config.clone() } }
}
```

- [ ] **Step 2: Update `src/main.rs`**

```rust
pub mod config;
pub mod error;
pub mod run;

use clap::Parser;

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[arg(long, default_value = "./sebas.toml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let raw = std::fs::read_to_string(&cli.config).unwrap_or_else(|_| {
        // No file → use minimal defaults; require env vars for credentials
        let app_id = std::env::var("SEBAS_FEISHU_APP_ID").unwrap_or_default();
        let app_secret = std::env::var("SEBAS_FEISHU_APP_SECRET").unwrap_or_default();
        format!(
            "[feishu]\napp_id = \"{app_id}\"\napp_secret = \"{app_secret}\"\nowner_id = \"ou_xxx\"\n"
        )
    });
    let cfg = sebas::config::Config::parse(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    sebas::run::run(cfg).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}
```

> `main.rs` is the binary entry; library code lives under `sebas` namespace. Add `src/lib.rs`:
```rust
pub mod config;
pub mod error;
pub mod run;
```

`src/lib.rs`:
```rust
pub mod config;
pub mod error;
pub mod run;
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles (warnings OK).

- [ ] **Step 4: Commit**

```bash
git add src/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add main entry with config load, signal handling, outbound pump"
```

---

## Task 17: Long-connection event loop + msg_id tracking

**Files:**
- Modify: `src/run.rs`
- Modify: `router/src/router.rs`

- [ ] **Step 1: Implement msg_id tracking**

Add to `router/src/router.rs`:
```rust
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct MsgIdMap {
    inner: Arc<RwLock<std::collections::HashMap<String, String>>>,  // session_id → msg_id
}

impl MsgIdMap {
    pub async fn record(&self, session_id: String, msg_id: String) {
        self.inner.write().await.insert(session_id, msg_id);
    }
    pub async fn get(&self, session_id: &str) -> Option<String> {
        self.inner.read().await.get(session_id).cloned()
    }
}
```

- [ ] **Step 2: Wire long-connection loop in `run.rs`**

In `run.rs`, before the SIGINT await:
```rust
// Placeholder WS loop — opens connection, dispatches FeishuEnvelope to router.
use feishu::events::FeishuEnvelope;
// Full impl uses tokio-tungstenite; this is a skeleton:
//   let (mut ws, _) = tokio_tungstenite::connect_async(url).await?;
//   while let Some(msg) = ws.next().await { router.handle(parsed).await; }
tracing::warn!("long-connection WS loop not yet implemented (see Task 17 follow-up)");
```

- [ ] **Step 3: Document unimplemented parts**

Add comment block to `run.rs`:
```rust
// TODO follow-up: implement WS receive loop using tokio-tungstenite.
//   - Connect to wss://open.feishu.cn/open-apis/socket/v1/connect?app_id=...
//   - Maintain ping/pong (15s)
//   - Parse each WS message as FeishuEnvelope, route via RouterHandle::handle
//   - Hook outbound to update msg_id map when first SendCard returns its msg_id
```

- [ ] **Step 4: Build + commit**

Run: `cargo build`
Run:
```bash
git add router/src/router.rs src/run.rs
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add MsgIdMap; WS loop stubbed with TODO"
```

---

## Task 18: Fake claude binary for integration tests

**Files:**
- Create: `tests/bin/fake-claude.rs`
- Create: `tests/fixtures/acp/canned_create_session.jsonl`
- Create: `acp-claude/tests/canned_test.rs`

- [ ] **Step 1: Write fake binary**

`tests/bin/fake-claude.rs`:
```rust
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut session_started = false;

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let kind = v.get("type").and_then(|k| k.as_str()).unwrap_or("");
        if !session_started && kind == "create_session" {
            session_started = true;
            let sid = v.get("session_id").and_then(|s| s.as_str()).unwrap_or("s1");
            writeln!(stdout, "{{\"type\":\"text_delta\",\"session_id\":\"{sid}\",\"delta\":\"hello \"}}").ok();
            writeln!(stdout, "{{\"type\":\"text_delta\",\"session_id\":\"{sid}\",\"delta\":\"world\"}}").ok();
            writeln!(stdout, "{{\"type\":\"finished\",\"session_id\":\"{sid}\"}}").ok();
            stdout.flush().ok();
        }
        // Echo other commands; ignore.
    }
}
```

Add to root `Cargo.toml`:
```toml
[[bin]]
name = "fake-claude"
path = "tests/bin/fake-claude.rs"
```

- [ ] **Step 2: Write integration test**

`acp-claude/tests/canned_test.rs`:
```rust
use acp_claude::manager::SessionManager;
use acp_claude::session::{AcpCommand, AcpEvent};
use std::time::Duration;

#[tokio::test]
async fn fake_claude_emits_finished() {
    let mgr = SessionManager::new();
    let id = mgr.create_session("./target/debug/fake-claude", vec![], None, "".into()).await.unwrap();
    // Send create_session command
    mgr.send(&id, AcpCommand::CreateSession { session_id: id.clone(), prompt: "hi".into() }).await.unwrap();

    // Receive events with timeout
    let evt = tokio::time::timeout(Duration::from_secs(2), mgr.next_event(&id))
        .await.expect("timeout").expect("event");
    assert!(matches!(evt, AcpEvent::TextDelta { .. } | AcpEvent::Finished { .. }));
}
```

Add helper to `SessionManager`:
```rust
// in acp-claude/src/manager.rs
pub async fn next_event(&self, session_id: &str) -> Option<AcpEvent> {
    let g = self.inner.lock().await;
    let m = g.get(session_id)?;
    let rx = &mut m.handle.evt_rx;  // borrow issue — see below
    rx.recv().await
}
```

Note: `AcpSessionHandle.evt_rx` is `mpsc::Receiver<AcpEvent>` — owning it through `SessionMeta.handle` requires `Mutex<Receiver>` or a Mutex<Option<Receiver>>. Replace:

```rust
// in acp-claude/src/session.rs
pub struct AcpSessionHandle {
    pub child_id: String,
    pub cmd_tx: mpsc::Sender<AcpCommand>,
    pub evt_rx: Arc<Mutex<mpsc::Receiver<AcpEvent>>>,
    pub _stdin_task: JoinHandle<()>,
}
```

Add `tokio::sync::Mutex` (already in tokio).

- [ ] **Step 3: Build fake binary**

Run: `cargo build --bin fake-claude`
Expected: compiles.

- [ ] **Step 4: Run integration test**

Run: `cargo test -p acp-claude --test canned_test`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add tests/ acp-claude/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add fake-claude binary + canned_session integration test"
```

---

## Task 19: End-to-end router test (text in → events out → commands)

**Files:**
- Create: `router/tests/e2e_test.rs`

- [ ] **Step 1: Write the test**

```rust
use router::state::SessionMap;
use router::router::RouterHandle;
use feishu::events::{FeishuIn, SessionKey};
use acp_claude::session::AcpEvent;

#[tokio::test]
async fn full_round_trip_text_to_events() {
    let map = SessionMap::new();
    let (handle, mut out_rx) = RouterHandle::new(map.clone());
    let key = SessionKey { chat_id: "oc_x".into(), thread_id: None };

    handle.clone().dispatch(FeishuIn::Text {
        key: key.clone(),
        text: "hello".into(),
        reply_to: None,
    }).await;

    // Expect: SpawnAcp → SendCard
    let first = out_rx.recv().await.unwrap();
    assert!(matches!(first, router::router::Out::SpawnAcp { .. }));

    // Simulate ACP event
    handle.dispatch_acp_event(AcpEvent::TextDelta {
        session_id: "s1".into(),
        delta: "hi back".into(),
    }).await;

    let out = out_rx.recv().await.unwrap();
    assert!(matches!(out, router::router::Out::UpdateCard { .. }));
}
```

Add `RouterHandle::clone` + `dispatch` / `dispatch_acp_event` helpers in `router/src/router.rs` if not already present (consolidate from previous tasks — by this task the API should be `pub async fn dispatch(self: Arc<Self>, ...)` or `RouterHandle::clone()` returning `Self`).

- [ ] **Step 2: Run test**

Run: `cargo test -p router --test e2e_test`
Expected: passes.

- [ ] **Step 3: Commit**

```bash
git add router/
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add end-to-end router dispatch test"
```

---

## Task 20: README + manual smoke test docs

**Files:**
- Create: `README.md`

- [ ] **Step 1: Write README**

```markdown
# sebas

A Rust daemon that bridges Claude Code (via ACP) to Feishu. Run Claude Code remotely from any Feishu chat.

## Quick start

```bash
# 1. Create a Feishu app at https://open.feishu.cn/ and grant:
#    - im:message (receive + send)
#    - im:message.group_at_msg (for group messages)
#    - im:message.p2p_msg (for direct messages)
#    Enable "Long connection" event subscription in app capabilities.
# 2. cp config/sebas.toml.example sebas.toml
# 3. Edit sebas.toml: set app_id, app_secret, owner_id (your open_id)
# 4. cargo build --release
# 5. ./target/release/sebas -config ./sebas.toml
```

## Configuration

Only 3 fields are required: `feishu.app_id`, `feishu.app_secret`, `feishu.owner_id`. Everything else has defaults — see spec.

## Commands

- `/new` — start fresh session
- `/sessions` — list active sessions
- `/switch <n>` — switch current chat to session n
- `/compact`, `/cost`, `/model`, `/cd`, `/cancel`, `/status` — see `/help`

## Architecture

See `docs/superpowers/specs/2026-07-26-sebas-design.md`.

## Manual smoke test

1. Start sebas; confirm `sebas started` log line
2. Send "hello" via Feishu DM; expect 👀 → 🚧 → ✅ card sequence
3. Send "list the files here"; expect permission card for Bash → click Allow
4. Send "/new"; confirm new session spawned
5. Send "/sessions"; confirm both visible
6. Restart sebas (`Ctrl-C`, then restart); send message in same chat; confirm session resumes
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git -c user.name=Claude -c user.email=claude@anthropic.com commit -m "Add README with quickstart and manual smoke test steps"
```

---

## Self-Review

After completing the plan, check:

1. **Spec coverage:**
   - §1 Goals: Tasks 14-17 (router, main), Task 10 (cards), Task 8 (events)
   - §2 Architecture: Tasks 1, 4, 7, 12 (workspace, ACP, Feishu, state)
   - §3 Data flow: Tasks 4-6 (ACP), 8-11 (Feishu + cards + media), 14-15 (router)
   - §4 Errors: Task 2 (error types); child crash, hang, panic — partially covered; **Tasks 16-17 need follow-up to wire error strategies into main loop** (acknowledge as TODO).
   - §5 Slash commands: Task 13 (parser), Tasks 14-15 (dispatch)
   - §6 Config: Task 3 (full)
   - §7 Long-connection: Tasks 7-9 (client + events + outbound)

2. **Placeholders:** No TBDs. Several `TODO` comments flag future WS-implementation work (Tasks 17, 16) — these are explicitly TODO follow-ups after this plan's first execution, not in-plan placeholders.

3. **Type consistency:** `AcpEvent` variants and `AcpCommand` variants used consistently. `SessionKey` shape consistent. `Out` enum variants consistent across Tasks 14-16.

4. **Acknowledged gaps for follow-up:**
   - WebSocket receive loop implementation (Task 17 stub)
   - Full error-handling strategy wiring into main loop (Task 16 partial)
   - Coverage tooling (cargo-llvm-cov) not yet configured in CI
   - `record` subcommand (per spec §4.4) deferred