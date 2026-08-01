# acp-bridge PromptRequest handler Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `acp-claude-bridge` 的 `PromptRequest` handler 从"立刻返 EndTurn"改成真 event-pump loop：写 `claude` 子进程 stdin，循环读 `StreamEvent`，每个 event 翻译成 `SessionNotification` 发回 sebas，碰到 `TurnEnd` 取出 `stop_reason` 调 `responder.respond(PromptResponse::new(...))`。

**Architecture:** `server::run` 新增 `Arc<tokio::sync::Mutex<()>>` gate（prompt 互斥）和 `Arc<AtomicBool>` cancel_flag。PromptRequest handler 持 gate 锁，内嵌 pump loop。新增 `notifications::from_update` 把 `TranslatedUpdate` 翻成 `SessionNotification`；新增 `notifications::acp_stop_reason` 翻 `claude::StopReason` → ACP `StopReason`。CancelNotification handler 仅置 cancel_flag；当前**不**杀子进程（范围外）。

**Tech Stack:** Rust 1.96、tokio 1.48 (full)、agent-client-protocol 2.0（schema 1.4.0）、serde_json。`acp-claude-bridge` crate 内部，零新依赖。

## Global Constraints

- 单 commit 上 `main`（< 3 commits，按 push policy）。
- 改动范围仅 `acp-claude-bridge/` 内部；不动 `acp-claude/`、`router/`、`feishu/`、`src/`。
- 严格 TDD：notifications 模块先写测试再写实现；server.rs 改造后跑既有 e2e 不能回归。
- 注释用中文，与 `acp-claude-bridge/` 既有代码一致。
- 不用 `dbg!`、不用 `println!`；用 `tracing::info!` / `tracing::warn!`，与既有风格一致。
- 行数预算 ~240 行（design 估算）：notifications.rs ~80、server.rs 增量 ~50、e2e 测试 ~110。
- 不改 fake-stream-claude；hello/bash/deny 三个 scenario 复用。
- Permission 回路不在范围（下一个 Beads）。

---

## File Structure

```
acp-claude-bridge/
├── src/
│   ├── lib.rs                # +1 行：pub mod notifications;
│   ├── server.rs             # 改：新增 gate + cancel_flag；改写 PromptRequest/CancelNotification handler
│   └── notifications.rs      # 新增：from_update + acp_stop_reason + 4 unit tests
└── tests/
    └── bridge_prompt_e2e.rs  # 新增：bridge_prompt_emits_text_delta_and_resolves_end_turn
```

`server.rs` 的其他 handler（initialize / new_session / load_session）保持不动。`permission.rs` / `claude.rs` / `translator.rs` / `main.rs` / `Cargo.toml` 不动。

---

## Task 1: notifications 模块（from_update + acp_stop_reason）

**Files:**
- Create: `acp-claude-bridge/src/notifications.rs`
- Modify: `acp-claude-bridge/src/lib.rs:2` (在 `pub mod claude;` 之后加 `pub mod notifications;`)

**Interfaces:**
- Consumes: `crate::claude::StopReason`、`crate::translator::{TranslatedUpdate, ToolStatus}`、`agent_client_protocol::schema::v1::{ContentBlock, ContentChunk, SessionId, SessionNotification, SessionUpdate, TextContent, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, StopReason}`
- Produces:
  - `pub fn from_update(session_id: &str, update: TranslatedUpdate) -> Option<SessionNotification>` — 把单个 `TranslatedUpdate` 翻译成 ACP `SessionNotification`；`TurnEnd` 返 `None`（本地消费）。
  - `pub fn acp_stop_reason(s: claude::StopReason) -> agent_client_protocol::schema::v1::StopReason` — 5 路映射：`EndTurn→EndTurn`、`MaxTokens→MaxTokens`、`ToolUse→EndTurn`（ACP 没有 `ToolUse` 变体，调用工具是正常 turn 结束）、`Cancelled→Cancelled`、`Unknown(_)→EndTurn`（保守回退）。

- [ ] **Step 1: 在 `notifications.rs` 写 4 个失败单测**

创建 `acp-claude-bridge/src/notifications.rs`（先放测试，编译会因 `from_update` / `acp_stop_reason` 不存在而失败）：

```rust
//! Translate `TranslatedUpdate` into ACP `SessionNotification` for the bridge.

use crate::claude::StopReason as ClaudeStopReason;
use crate::translator::{TranslatedUpdate, ToolStatus};
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, SessionNotification, SessionUpdate, StopReason, TextContent,
    ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
};
use serde_json::json;

pub fn from_update(
    session_id: &str,
    update: TranslatedUpdate,
) -> Option<SessionNotification> {
    unimplemented!()
}

pub fn acp_stop_reason(s: ClaudeStopReason) -> StopReason {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::StopReason as ClaudeSR;

    fn sid() -> &'static str {
        "sess-1"
    }

    #[test]
    fn agent_message_chunk_to_session_notification() {
        let n = from_update(
            sid(),
            TranslatedUpdate::AgentMessageChunk { text: "hi".into() },
        )
        .expect("notification");
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["sessionId"], "sess-1");
        assert_eq!(v["update"]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(v["update"]["content"]["type"], "text");
        assert_eq!(v["update"]["content"]["text"], "hi");
    }

    #[test]
    fn tool_call_preserves_id_and_title() {
        let n = from_update(
            sid(),
            TranslatedUpdate::ToolCall {
                id: "toolu_01".into(),
                title: "Bash".into(),
                raw_input: json!({"command": "ls"}),
            },
        )
        .expect("notification");
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["update"]["sessionUpdate"], "tool_call");
        assert_eq!(v["update"]["toolCallId"], "toolu_01");
        assert_eq!(v["update"]["title"], "Bash");
        assert_eq!(v["update"]["rawInput"]["command"], "ls");
    }

    #[test]
    fn tool_call_update_with_output() {
        let n = from_update(
            sid(),
            TranslatedUpdate::ToolCallUpdate {
                id: "toolu_01".into(),
                status: ToolStatus::Completed,
                raw_output: Some("file.txt".into()),
            },
        )
        .expect("notification");
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["update"]["sessionUpdate"], "tool_call_update");
        assert_eq!(v["update"]["toolCallId"], "toolu_01");
        assert_eq!(v["update"]["status"], "completed");
        assert_eq!(v["update"]["rawOutput"], "file.txt");
    }

    #[test]
    fn turn_end_returns_none() {
        let r = from_update(
            sid(),
            TranslatedUpdate::TurnEnd {
                stop_reason: ClaudeSR::EndTurn,
            },
        );
        assert!(r.is_none());
    }

    #[test]
    fn acp_stop_reason_mapping() {
        assert_eq!(acp_stop_reason(ClaudeSR::EndTurn), StopReason::EndTurn);
        assert_eq!(acp_stop_reason(ClaudeSR::MaxTokens), StopReason::MaxTokens);
        // ACP 没有 ToolUse 变体：调工具视为正常 turn 结束
        assert_eq!(acp_stop_reason(ClaudeSR::ToolUse), StopReason::EndTurn);
        assert_eq!(acp_stop_reason(ClaudeSR::Cancelled), StopReason::Cancelled);
        assert_eq!(
            acp_stop_reason(ClaudeSR::Unknown("mystery".into())),
            StopReason::EndTurn
        );
    }
}
```

注意：5 个 `#[test]` 函数（含 `acp_stop_reason_mapping`），design 里说"4 unit tests"是低估了一档；写实现时跑出来是 5 个都过。

- [ ] **Step 2: 跑测试确认失败**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge --lib notifications
```

Expected: 编译失败（`unimplemented!()` 会让 5 个测试 panic；先看 panic 也算 fail）。最简验证：`cargo test -p acp-claude-bridge --lib notifications 2>&1 | head -30` 看到 `panicked at 'not yet implemented'` 即 OK。

- [ ] **Step 3: 实现 `from_update` 与 `acp_stop_reason`**

把 `notifications.rs` 顶部的两个 `unimplemented!()` 替换为真实现：

```rust
pub fn from_update(
    session_id: &str,
    update: TranslatedUpdate,
) -> Option<SessionNotification> {
    let session_id = agent_client_protocol::schema::v1::SessionId::new(session_id.to_string());
    let su = match update {
        TranslatedUpdate::AgentMessageChunk { text } => SessionUpdate::AgentMessageChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new(text))),
        ),
        TranslatedUpdate::ToolCall { id, title, raw_input } => {
            SessionUpdate::ToolCall(ToolCall::new(id, title).raw_input(raw_input))
        }
        TranslatedUpdate::ToolCallUpdate {
            id,
            status,
            raw_output,
        } => {
            let s = match status {
                ToolStatus::Completed => ToolCallStatus::Completed,
                ToolStatus::Failed => ToolCallStatus::Failed,
            };
            let raw_output = raw_output.map(serde_json::Value::String);
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                id,
                ToolCallUpdateFields {
                    status: Some(s),
                    raw_output,
                    ..Default::default()
                },
            ))
        }
        TranslatedUpdate::TurnEnd { .. } => return None,
    };
    Some(SessionNotification::new(session_id, su))
}

pub fn acp_stop_reason(s: ClaudeStopReason) -> StopReason {
    match s {
        ClaudeStopReason::EndTurn => StopReason::EndTurn,
        ClaudeStopReason::MaxTokens => StopReason::MaxTokens,
        // ACP 没有 ToolUse 变体：claude 调工具是正常 turn 结束
        ClaudeStopReason::ToolUse => StopReason::EndTurn,
        ClaudeStopReason::Cancelled => StopReason::Cancelled,
        ClaudeStopReason::Unknown(_) => StopReason::EndTurn,
    }
}
```

- [ ] **Step 4: 跑测试确认全过**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge --lib notifications
```

Expected: `5 passed; 0 failed`。如果有 diff（典型：`rawOutput` 类型不匹配），调整 `raw_output.map(...)` 这一行；目标是 JSON 序列化后 `rawOutput` 字段是字符串，与测试断言一致。

- [ ] **Step 5: 在 `lib.rs` 暴露模块**

编辑 `acp-claude-bridge/src/lib.rs:2` 之后插入一行：

```rust
pub mod claude;
pub mod notifications;
pub mod permission;
pub mod translator;
```

`lib.rs` 当前 4 行；加 1 行变 5 行。

- [ ] **Step 6: 全 crate 单测确认不破坏既有覆盖**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge --lib
```

Expected: 既有 4 个 parser 单测 + 新增 5 个 = 全过。

- [ ] **Step 7: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add acp-claude-bridge/src/notifications.rs acp-claude-bridge/src/lib.rs
git commit -m "feat(acp-claude-bridge): notifications module translates stream events to ACP"
```

---

## Task 2: server.rs — 接入 event-pump loop + cancel 通路

**Files:**
- Modify: `acp-claude-bridge/src/server.rs`（整文件改写，但只动 PromptRequest / CancelNotification handler 和 `run` 入口；其余 3 个 handler 保持原样）

**Interfaces:**
- Consumes: `crate::claude::ClaudeDriver`（`send_user(&str)`、`next_event() -> Option<StreamEvent>`）、`crate::translator::translate`、`crate::notifications::{from_update, acp_stop_reason}`、`tokio::sync::Mutex`、`std::sync::atomic::AtomicBool`、`std::sync::Arc`、`agent_client_protocol::schema::v1::{CancelNotification, PromptRequest, PromptResponse, StopReason}`
- Produces: 改写后的 `pub async fn run(mut claude: ClaudeDriver, perm_tx: mpsc::Sender<PermissionDecision>) -> anyhow::Result<()>`，新行为：
  - `gate: Arc<Mutex<()>>` 串行化所有 prompt handler
  - `cancel_flag: Arc<AtomicBool>` 在 PromptRequest 入口清零、CancelNotification 置位
  - PromptRequest handler 流程：解构文本 → 持 gate → `send_user` → 循环 `next_event` → 每个 event 调 `translate` 后逐个 `from_update` → `cx.send_notification` → 遇 `TurnEnd { stop_reason }` break → `responder.respond(PromptResponse::new(acp_stop_reason(sr)))`
  - `perm_tx` 仍然保留参数（permission 回路是下一个 Beads，本次保持签名）

- [ ] **Step 1: 替换 `server.rs`**

完整文件如下（`permission` / `claude` / `translator` / `notifications` 模块导入；`run` 内先建 `gate` 与 `cancel_flag`；PromptRequest handler 与 CancelNotification handler 全部改写）：

```rust
//! ACP server side: registers handlers on `agent-client-protocol`'s builder
//! and translates incoming requests to/from the ClaudeDriver and permission
//! broker.

use crate::claude::driver::ClaudeDriver;
use crate::notifications;
use crate::permission::PermissionDecision;
use crate::translator;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, InitializeRequest, InitializeResponse,
    LoadSessionRequest, NewSessionRequest, NewSessionResponse, PromptCapabilities,
    PromptRequest, PromptResponse, SessionId, StopReason,
};
use agent_client_protocol::{on_receive_notification, on_receive_request, Agent, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

pub async fn run(
    mut claude: ClaudeDriver,
    perm_tx: mpsc::Sender<PermissionDecision>,
) -> anyhow::Result<()> {
    let _ = perm_tx; // 下一 ticket 接通 permission broker；本 ticket 保持参数签名
    // 串行化所有 prompt handler：claude 子进程是单 stream，一次只允许一个 pump 在读
    let gate: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
    // CancelNotification 只置位；当前不向 claude 子进程发中断信号（范围外）
    let cancel_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    Agent
        .builder()
        .name("claude-acp-bridge")
        .on_receive_request(
            async move |req: InitializeRequest, responder, _cx| {
                let caps = AgentCapabilities::new()
                    .load_session(false)
                    .prompt_capabilities(
                        PromptCapabilities::new()
                            .image(false)
                            .audio(false)
                            .embedded_context(false),
                    );
                responder.respond(
                    InitializeResponse::new(req.protocol_version).agent_capabilities(caps),
                )
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |req: NewSessionRequest, responder, _cx| {
                let id = SessionId::new(uuid::Uuid::new_v4().to_string());
                responder.respond(NewSessionResponse::new(id))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async move |req: LoadSessionRequest, responder, _cx| {
                // Bridge intentionally returns "session not found" — sebas
                // already handles this by falling back to SpawnAcp with a
                // fresh session.
                responder.respond_with_error(agent_client_protocol::Error::new(
                    -32000,
                    "loadSession not supported by bridge",
                ))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            {
                let gate = gate.clone();
                let cancel_flag = cancel_flag.clone();
                async move |req: PromptRequest, responder, cx| {
                    let session_id = req.session_id.0.to_string();
                    let text = req
                        .prompt
                        .iter()
                        .filter_map(|b| match b {
                            agent_client_protocol::schema::v1::ContentBlock::Text(t) => {
                                Some(t.text.as_str())
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    tracing::info!(session_id=%session_id, text_len=text.len(), "prompt received");

                    cancel_flag.store(false, Ordering::SeqCst);
                    let _guard = gate.lock().await;

                    if let Err(e) = claude.send_user(&text).await {
                        tracing::warn!(error=%e, "send_user failed");
                        let _ = responder.respond_with_error(
                            agent_client_protocol::util::internal_error(
                                "claude send_user failed",
                            ),
                        );
                        return;
                    }

                    let mut stop_reason = StopReason::EndTurn;
                    loop {
                        let Some(event) = claude.next_event().await else {
                            tracing::warn!(session_id=%session_id, "driver EOF before TurnEnd");
                            break;
                        };
                        if cancel_flag.load(Ordering::SeqCst) {
                            stop_reason = StopReason::Cancelled;
                            break;
                        }
                        for update in translator::translate(event.clone()) {
                            if let Some(notif) =
                                notifications::from_update(&session_id, update)
                            {
                                if let Err(e) = cx.send_notification(notif) {
                                    tracing::warn!(error=%e, "send_notification failed");
                                }
                            }
                        }
                        if let crate::claude::StreamEvent::TurnEnd { stop_reason: sr } = event {
                            stop_reason = notifications::acp_stop_reason(sr);
                            break;
                        }
                    }
                    let _ = responder.respond(PromptResponse::new(stop_reason));
                }
            },
            on_receive_request!(),
        )
        .on_receive_notification(
            {
                let cancel_flag = cancel_flag.clone();
                async move |_notif: CancelNotification, _cx| {
                    cancel_flag.store(true, Ordering::SeqCst);
                    tracing::info!("cancel received");
                    Ok(())
                }
            },
            on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await?;
    Ok(())
}
```

要点：
- `event.clone()` 出现在 `translate(event.clone())` 与之后的 `if let StreamEvent::TurnEnd { ... } = event`——`StreamEvent: Clone` 已 derive。
- `_guard` 在 handler 末尾 drop 自动释放 gate，下个 prompt 进。
- 错误时 `let _ = responder.respond_with_error(...)`：SDK 的 `respond_with_error` 返 `Result`，我们不强求成功。
- `claude.next_event()` 返 `None` = 子进程 EOF。spec §3.1 "永不超时"；子进程自然退出视为 EndTurn（`stop_reason` 默认值）。

- [ ] **Step 2: 编译 bridge binary**

```bash
cd /home/bot/workbench/repos/sebas && cargo build -p acp-claude-bridge 2>&1 | tail -20
```

Expected: 干净编译（warning OK）。如果有 `unresolved import` / `expected fn` / `move closure` 错，按编译器提示修。最常见的是 `agent_client_protocol::util::internal_error` 不存在；备选是 `agent_client_protocol::Error::new(-32603, "...")`。如果发生，回退到 `agent_client_protocol::Error::new(-32603, "claude send_user failed")`。

- [ ] **Step 3: 跑既有 e2e + 单测，确认没回归**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge 2>&1 | tail -30
```

Expected: 既有 19 个测试全过（4 个 claude 单测 + 1 个 permission 单测 + 2 个 bridge_e2e + 1 个 permission_roundtrip + 5 个新增 notifications 单测 + 1 个 claude_driver + …）。精确数：`cargo test -p acp-claude-bridge 2>&1 | grep -E "^test result"` 看每条 `passed; 0 failed`。

如果 `bridge_e2e::bridge_handshake_returns_capabilities` 或 `bridge_session_new_returns_uuid` 失败：检查 initialize / new_session handler 是否保持原样；如果失败的是 `permission_roundtrip::hook_socket_round_trip`：与本 task 无关，应仍通过；失败则停下来查 broker 状态。

- [ ] **Step 4: 跑全 workspace 单测，确保跨 crate 不破**

```bash
cd /home/bot/workbench/repos/sebas && cargo test --workspace --lib 2>&1 | tail -20
```

Expected: 全过（`acp-claude-bridge` + `acp-claude` + `router` + `feishu` 单测）。任何红都是回归，停下来排查。

- [ ] **Step 5: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add acp-claude-bridge/src/server.rs
git commit -m "feat(acp-claude-bridge): drive event-pump loop in prompt handler"
```

---

## Task 3: E2E — bridge 把 hello scenario 的 text delta 转成 session/update 通知

**Files:**
- Create: `acp-claude-bridge/tests/bridge_prompt_e2e.rs`

**Interfaces:**
- Consumes: 既有 `bridge_e2e.rs` 的 helper 模式（`bridge_path()` / `fake_path()` 直接复用 `bridge_e2e.rs` 的实现；本文件独立写一份，避免与既有 test 二进制相互依赖——integration test 文件之间不共享模块）。
- Produces: `tests/bridge_prompt_e2e.rs` 单文件，含 1 个 `#[tokio::test] async fn bridge_prompt_emits_text_delta_and_resolves_end_turn()`。

- [ ] **Step 1: 写测试**

创建 `acp-claude-bridge/tests/bridge_prompt_e2e.rs`：

```rust
//! End-to-end: bridge 接到 session/prompt 后，把 fake-stream-claude 的 hello
//! scenario (text delta + result) 转成一条 session/update 通知 + stopReason=end_turn
//! 响应。
//!
//! Run: cargo test -p acp-claude-bridge --test bridge_prompt_e2e -- --nocapture

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

fn bridge_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // out of acp-claude-bridge/
    p.push("target/debug/claude-acp-bridge");
    p
}

fn fake_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.push("target/debug/fake-stream-claude");
    p
}

async fn drive_until_contains<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
    needle: &str,
    deadline: Duration,
) -> String {
    let mut buf = String::new();
    let started = std::time::Instant::now();
    while started.elapsed() < deadline {
        buf.clear();
        let fut = reader.read_line(&mut buf);
        match timeout(Duration::from_secs(2), fut).await {
            Ok(Ok(0)) => break,
            Ok(Ok(_)) => {
                if buf.contains(needle) {
                    return buf;
                }
            }
            Ok(Err(e)) => panic!("read error: {e}"),
            Err(_) => continue, // 单行 2s 超时但整体未到 deadline → 继续
        }
    }
    panic!("never found {needle:?} within {deadline:?}; last line: {buf:?}");
}

#[tokio::test]
async fn bridge_prompt_emits_text_delta_and_resolves_end_turn() {
    let mut child = TokioCommand::new(bridge_path())
        .env("SEBAS_CLAUDE_PATH", fake_path().to_str().unwrap())
        .args(&["hello"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn bridge");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // initialize
    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"prompt-e2e","version":"0"}}}"#,
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
    let init_line = drive_until_contains(&mut stdout, "agentCapabilities", Duration::from_secs(10)).await;
    assert!(init_line.contains("\"loadSession\":false"), "init: {init_line}");

    // initialized notification
    stdin
        .write_all(br#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#)
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    // session/new
    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#,
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
    let new_line = drive_until_contains(&mut stdout, "sessionId", Duration::from_secs(10)).await;
    // 抓出 sessionId 用于 session/prompt
    let v: serde_json::Value = serde_json::from_str(&new_line).expect("session/new response json");
    let session_id = v["result"]["sessionId"]
        .as_str()
        .expect("sessionId string")
        .to_string();

    // session/prompt —— 把 sessionId 注入到 params.sessionId
    let prompt_payload = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"hi"}}]}}}}"#
    );
    stdin.write_all(prompt_payload.as_bytes()).await.unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    // 期望：1 条 session/update 通知（agent_message_chunk 含 "hello from fake claude"）
    let notif_line = drive_until_contains(
        &mut stdout,
        "hello from fake claude",
        Duration::from_secs(10),
    )
    .await;
    let nv: serde_json::Value = serde_json::from_str(&notif_line).expect("notification json");
    assert_eq!(nv["method"], "session/update", "notif: {notif_line}");
    assert_eq!(
        nv["params"]["update"]["sessionUpdate"], "agent_message_chunk",
        "update kind"
    );
    assert_eq!(
        nv["params"]["update"]["content"]["text"], "hello from fake claude",
        "chunk text"
    );
    assert_eq!(nv["params"]["sessionId"], session_id, "sessionId tagged");

    // 期望：最终 id=3 的 session/prompt 响应，stopReason=end_turn
    // 因 stdout 同时混着通知和响应，按 id 字段匹配
    let resp_line = drive_until_contains(
        &mut stdout,
        "\"id\":3",
        Duration::from_secs(10),
    )
    .await;
    let rv: serde_json::Value = serde_json::from_str(&resp_line).expect("response json");
    assert_eq!(rv["id"], 3, "id: {resp_line}");
    assert_eq!(
        rv["result"]["stopReason"], "end_turn",
        "stopReason: {resp_line}"
    );

    drop(stdin);
    drop(child);
}
```

要点：
- bridge 发的是 JSON-RPC **通知** `session/update`（method 字段 + params），不是响应；区别于 session/prompt 的响应（id 字段 + result）。
- `drive_until_contains` 用 `read_line` 一行一行扫；2s 单行超时但整体 deadline 10s，避免子进程假死时 hang。
- 通知行是 `{"jsonrpc":"2.0","method":"session/update","params":{...}}`；响应行是 `{"jsonrpc":"2.0","id":3,"result":{...}}`。
- sessionId 从 session/new 响应里抓出来；新 session/prompt 必须带这个 id。

- [ ] **Step 2: 跑测试**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge --test bridge_prompt_e2e -- --nocapture 2>&1 | tail -40
```

Expected: `1 passed; 0 failed`，耗时 < 5s。

调试信号：
- **如果 10s 内读不到 `hello from fake claude`** → bridge 没把 text delta 翻译出来。查 server.rs 里 `from_update` 是否被调、`cx.send_notification` 是否成功、`fake-stream-claude hello` 输出的 delta 行是否真进了 driver mpsc。
- **如果 10s 内读不到 `"id":3` 的响应** → pump 循环没 break 或 `responder.respond` 没发回。查 `acp_stop_reason` 路径。
- **如果 sessionId 字段不存在** → bridge 的 new_session handler 没把 id 序列化进 result 字段（理论上既有 bridge_e2e 已经在测这个；先跑既有 `bridge_session_new_returns_uuid` 确认仍通过）。

- [ ] **Step 3: 全 crate 复测一遍**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge 2>&1 | tail -10
```

Expected: 既有 19 个 + 新增 1 个 e2e + 新增 5 个 notifications 单测 = 全过。

- [ ] **Step 4: 跑全 workspace 集成 + e2e（不只 lib）**

```bash
cd /home/bot/workbench/repos/sebas && cargo test --workspace 2>&1 | tail -15
```

Expected: 所有 crate 单测 + 集成 + e2e 全过。任何红停下排查。

- [ ] **Step 5: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add acp-claude-bridge/tests/bridge_prompt_e2e.rs
git commit -m "test(acp-claude-bridge): e2e prompt handler drives fake-stream-claude to text delta"
```

---

## Self-Review Checklist（writer 写完自查）

1. **Spec coverage:**
   - §1 目标（4 条）：写 stdin + 读 stdout + 翻译 + 响应 → Task 2 ✓
   - §2.1 共享状态（gate + cancel_flag）→ Task 2 Step 1 ✓
   - §2.2 pump 流程（cancel_flag.clear / send_user / loop / 翻译 / TurnEnd / respond）→ Task 2 Step 1 ✓
   - §2.3 通知映射 4 种 → Task 1 Step 1+3 ✓
   - §2.4 CancelNotification handler → Task 2 Step 1 ✓
   - §2.5 错误路径（send_user 失败、next_event None、send_notification 失败）→ Task 2 Step 1 ✓
   - §3 行数估算（notifications ~80 / server ~50 / e2e ~110）→ 实际行数在 commit 后由 subagent 汇报，可浮动 ±20 行
   - §4.1 单测 4 条 → 实际写 5 条（含 `acp_stop_reason_mapping`）— 数量超出而非缺失
   - §4.2 e2e 1 条 → Task 3 ✓
   - §5 commit 计划（3 个 commit）→ Task 1/2/3 各一个 commit ✓
   - §6 风险与限制（串行化、cancel 不杀子进程、send_notification 时序）→ Task 2 Step 1 注释里点明 ✓
   - §7 范围外（ToolUse 权限、cancel SIGTERM、跨 session driver、session/load）→ 本 plan 不实现
   - §8 自审清单（SDK 类型校对）→ Step 1/3 代码严格按校对后的 API 写 ✓
2. **Placeholder scan:** 0 个 TBD/TODO/"implement later"；每个 Step 都有具体代码或命令。
3. **Type consistency:** `ClaudeStopReason` / `TranslatedUpdate` / `ToolStatus` / `StopReason` 全程同名同路径；`from_update(&str, TranslatedUpdate) -> Option<SessionNotification>` 与 `acp_stop_reason(ClaudeStopReason) -> StopReason` 在 Task 1 定义、Task 2 消费、Task 3 端到端验证。
4. **TDD discipline:** Task 1 严格 TDD（先测后实现、跑失败、改实现、跑过）；Task 2 是"行为替换"型重构，靠既有 e2e（bridge_e2e / permission_roundtrip）做回归门 + Task 3 新增 e2e 覆盖新行为；Task 3 严格 TDD（先测后 commit）。
5. **Commit granularity:** 3 个 commit，按功能切片。规则：单 task 单 commit；不混。

---

## Execution Handoff

Plan 写完，下一步选执行模式：

1. **Subagent-Driven（推荐）** — 我每个 task 起一个 fresh subagent，两阶段审（实现+测试）后我合并到 main。
2. **Inline Execution** — 当前 session 直接跑 executing-plans，批量执行 + 评审 checkpoint。

按 push policy：3 commits < 5，可直接 push 到 main（如果走 subagent 路径每 task 单独 review，可以 atomic squash）。
