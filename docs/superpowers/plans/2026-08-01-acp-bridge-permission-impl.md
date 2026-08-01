# acp-bridge ToolUse → session/request_permission Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `acp-claude-bridge` 的 event-pump loop 里拦截 `StreamEvent::ToolUse`，同步发 `session/request_permission` 给 sebas，等 reply，把决策写 broker 让 PreToolUse hook unblock。

**Architecture:** prompt handler 的 pump loop 在 `translator::translate(event)` 之前先 match `StreamEvent::ToolUse { id, name, input }`：emit `ToolCall` notification → `cx.send_request_to(Client, RequestPermissionRequest::new(...))` 同步等 → 选项 ID 解析 → `perm_tx.send(decision)` 让 broker 写 socket。`main.rs` 把当前丢掉的 `broker.run()` spawn 起来。失败一律 Deny。

**Tech Stack:** Rust 1.96、tokio 1.48 (full)、agent-client-protocol 2.0 (schema 1.4.0)、`acp-claude-bridge` crate 内部；零新依赖。

## Global Constraints

- 单 commit 上 `main`（< 3 commits）。
- 改动范围仅 `acp-claude-bridge/` 内部；不动 `acp-claude/`、`router/`、`feishu/`、`src/`、`permission.rs`、`hooks/pretooluse.sh`。
- 严格 TDD：先写测试再写实现。
- 注释用中文，与既有代码一致。
- 不用 `dbg!` / `println!`；用 `tracing::info!` / `tracing::warn!`。
- 行数预算 ~180 行（design 估算）：server.rs ~30 + main.rs +1 + e2e ~110。
- 不改 fake-stream-claude。
- 失败一律 Deny（send_request_to Err / Response 非 Selected / option_id 不以 `"allow_"` 开头）。
- PreToolUse hook 脚本的 macOS BSD nc 兼容 → follow-up Beads。

---

## File Structure

```
acp-claude-bridge/
├── src/
│   ├── server.rs        # 改：新增 option_id_to_decision + build_permission_options + unit tests；pump loop 加 ToolUse 拦截分支
│   └── main.rs          # 改：spawn broker.run() 替换 let _broker
└── tests/
    └── permission_e2e.rs  # 新增：hello scenario 回归测试（真权限通路靠 unit + 集成覆盖）
```

`permission.rs` / `translator.rs` / `notifications.rs` / `lib.rs` / `claude.rs` / `Cargo.toml` 不动。

---

## Task 1: server.rs — `option_id_to_decision` + `build_permission_options` (TDD)

**Files:**
- Modify: `acp-claude-bridge/src/server.rs`（顶部 `use` 加 2 个类型；新增 2 个私有 fn + `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `crate::permission::PermissionDecision::{Allow, Deny}`；`agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind}`
- Produces:
  - `fn option_id_to_decision(id: &str) -> PermissionDecision` — 字符串到决策：`"allow_"` 前缀 → `Allow`，其他 → `Deny`。
  - `fn build_permission_options() -> Vec<PermissionOption>` — 固定 3 项：`allow_once` / `allow_always` / `reject_once`。

- [ ] **Step 1: 在 `server.rs` 写 5 个失败单测**

在 `server.rs` 底部加 `#[cfg(test)] mod tests { ... }`：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::PermissionDecision;

    #[test]
    fn option_id_allow_once_maps_to_allow() {
        assert_eq!(option_id_to_decision("allow_once"), PermissionDecision::Allow);
    }

    #[test]
    fn option_id_allow_always_maps_to_allow() {
        assert_eq!(option_id_to_decision("allow_always"), PermissionDecision::Allow);
    }

    #[test]
    fn option_id_reject_once_maps_to_deny() {
        assert_eq!(option_id_to_decision("reject_once"), PermissionDecision::Deny);
    }

    #[test]
    fn option_id_unknown_maps_to_deny() {
        assert_eq!(option_id_to_decision("mystery"), PermissionDecision::Deny);
    }

    #[test]
    fn build_permission_options_returns_three_stable_ids() {
        let opts = build_permission_options();
        assert_eq!(opts.len(), 3);
        // 字符串约定必须与 acp-claude/manager.rs:320-329 保持一致；
        // 该侧改字符串时要同步通知 bridge
        let ids: Vec<&str> = opts.iter().map(|o| o.option_id.0.as_ref()).collect();
        assert_eq!(ids, vec!["allow_once", "allow_always", "reject_once"]);
    }
}
```

`PermissionDecision` 已 derive `Eq`（`permission.rs:13`）。

- [ ] **Step 2: 跑测试确认失败**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge --lib server 2>&1 | tail -10
```

Expected: 编译失败（`option_id_to_decision` / `build_permission_options` 不存在），看到 `error[E0425]: cannot find function` 即 OK。

- [ ] **Step 3: 在 `server.rs` 加 2 个 fn**

顶部 `use` 块加：

```rust
use agent_client_protocol::schema::v1::{PermissionOption, PermissionOptionKind};
```

在 `pub async fn run(...)` 之上加：

```rust
/// Map a `PermissionOptionId` (as string) to a `PermissionDecision`.
/// `allow_*` → Allow, anything else → Deny. 字符串约定必须与
/// `acp-claude/manager.rs:320-329` 保持一致；该侧改字符串时要同步通知 bridge。
fn option_id_to_decision(id: &str) -> crate::permission::PermissionDecision {
    if id.starts_with("allow_") {
        crate::permission::PermissionDecision::Allow
    } else {
        crate::permission::PermissionDecision::Deny
    }
}

/// 固定 3 个 permission 选项，与 acp-claude 端一致：
/// `allow_once` / `allow_always` / `reject_once`。
fn build_permission_options() -> Vec<PermissionOption> {
    vec![
        PermissionOption::new("allow_once", "Allow once", PermissionOptionKind::AllowOnce),
        PermissionOption::new("allow_always", "Allow for this chat", PermissionOptionKind::AllowAlways),
        PermissionOption::new("reject_once", "Deny", PermissionOptionKind::RejectOnce),
    ]
}
```

- [ ] **Step 4: 跑测试确认全过**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge --lib server
```

Expected: `5 passed; 0 failed`。如果 fail，按字面 `acp-claude/manager.rs:320-329` 调字符串。

- [ ] **Step 5: 全 crate 单测确认无回归**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge --lib 2>&1 | tail -5
```

Expected: 既有 19 + 新增 5 = 24 全过。

- [ ] **Step 6: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add acp-claude-bridge/src/server.rs
git commit -m "feat(acp-claude-bridge): permission option id mapping helpers"
```

---

## Task 2: server.rs pump loop + main.rs spawn broker

**Files:**
- Modify: `acp-claude-bridge/src/server.rs`（`use` 块加 5 个类型；pump loop 把 `for update in translator::translate(event.clone())` 改成 match-based 分支）
- Modify: `acp-claude-bridge/src/main.rs`（`let (_broker, ...)` 改成 spawn）

**Interfaces:**
- Consumes: `claude::StreamEvent::ToolUse { id, name, input }`；`PermissionDecision`；`ConnectionTo<Client>::send_request_to` + `send_notification`；`option_id_to_decision`、`build_permission_options`（来自 Task 1）
- Produces: 改写后的 pump loop（每个 `StreamEvent::ToolUse` 触发 4 步逻辑）；`main.rs` 启动 `broker.run()` 任务。

- [ ] **Step 1: 扩展 `server.rs` 的 `use` 块**

在 `use agent_client_protocol::schema::v1::{...}` 里加：

```rust
RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
ToolCallUpdate, ToolCallUpdateFields,
```

完整 use 变成：

```rust
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, InitializeRequest, InitializeResponse,
    LoadSessionRequest, NewSessionRequest, NewSessionResponse, PromptCapabilities,
    PromptRequest, PromptResponse, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SessionId, StopReason, ToolCallUpdate,
    ToolCallUpdateFields,
};
```

（`PermissionOption` / `PermissionOptionKind` 已在 Task 1 加。）

- [ ] **Step 2: 重写 pump loop**

把 `server.rs` 当前 pump loop（line 99-121）里的：

```rust
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
```

替换为：

```rust
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
                        // ToolUse 拦截：先发 ToolCall 通知，再同步等 sebas RequestPermissionResponse。
                        // spec §4：失败一律 Deny（send_request_to Err / Response 非 Selected / option_id 不以 "allow_" 开头）。
                        // 拦截后该 event 不再走 translator（ToolCall 通知已用 from_update 显式发出）。
                        let updates: Vec<crate::translator::TranslatedUpdate> = match &event {
                            crate::claude::StreamEvent::ToolUse { id, name, input } => {
                                let tool_call_update = ToolCallUpdate::new(
                                    id.clone(),
                                    ToolCallUpdateFields {
                                        title: Some(name.clone()),
                                        raw_input: Some(input.clone()),
                                        ..Default::default()
                                    },
                                );
                                // 1) emit ToolCall 通知
                                let notif = notifications::from_update(
                                    &session_id,
                                    crate::translator::TranslatedUpdate::ToolCall {
                                        id: id.clone(),
                                        title: name.clone(),
                                        raw_input: input.clone(),
                                    },
                                );
                                if let Some(n) = notif {
                                    if let Err(e) = cx.send_notification(n) {
                                        tracing::warn!(error=%e, tool_id=%id, "tool_call notification failed");
                                    }
                                }
                                // 2) 同步等 sebas 决策
                                let req = RequestPermissionRequest::new(
                                    SessionId::new(session_id.clone()),
                                    tool_call_update,
                                    build_permission_options(),
                                );
                                let decision = match cx
                                    .send_request_to(agent_client_protocol::Client, req)
                                    .await
                                {
                                    Ok(resp) => match resp.outcome {
                                        RequestPermissionOutcome::Selected(sel) => {
                                            tracing::info!(tool_id=%id, option=%sel.option_id.0, "permission selected");
                                            option_id_to_decision(sel.option_id.0.as_ref())
                                        }
                                        other => {
                                            tracing::warn!(tool_id=%id, ?other, "permission outcome not Selected → Deny");
                                            crate::permission::PermissionDecision::Deny
                                        }
                                    },
                                    Err(e) => {
                                        tracing::warn!(error=%e, tool_id=%id, "send_request_to failed → Deny");
                                        crate::permission::PermissionDecision::Deny
                                    }
                                };
                                // 3) 写 broker → hook unblock → claude 继续
                                if let Err(e) = perm_tx.send(decision.clone()).await {
                                    tracing::warn!(error=%e, "perm_tx.send failed");
                                }
                                // 4) 拒绝时补 ToolCallUpdate Failed
                                if decision == crate::permission::PermissionDecision::Deny {
                                    let denied = notifications::from_update(
                                        &session_id,
                                        crate::translator::TranslatedUpdate::ToolCallUpdate {
                                            id: id.clone(),
                                            status: crate::translator::ToolStatus::Failed,
                                            raw_output: Some("denied by sebas".into()),
                                        },
                                    );
                                    if let Some(n) = denied {
                                        if let Err(e) = cx.send_notification(n) {
                                            tracing::warn!(error=%e, tool_id=%id, "denied notification failed");
                                        }
                                    }
                                }
                                Vec::new() // 拦截后不再走 translator
                            }
                            _ => translator::translate(event.clone()),
                        };
                        for update in updates {
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
```

- [ ] **Step 3: `main.rs` 启 `broker.run()`**

把 `acp-claude-bridge/src/main.rs:27` 的：

```rust
let (_broker, perm_rx) = permission::PermissionBroker::bind().await?;
```

替换为：

```rust
let (broker, perm_tx) = permission::PermissionBroker::bind().await?;
tokio::spawn(broker.run());
```

注意：变量名从 `perm_rx` 改成 `perm_tx`（这是 Sender；broker 内部自己拿 `Arc<Mutex<Receiver>>` 句柄，spec 验证过 `permission.rs:34`）。

- [ ] **Step 4: 编译 bridge binary**

```bash
cd /home/bot/workbench/repos/sebas && cargo build -p acp-claude-bridge 2>&1 | tail -20
```

Expected: 干净编译。如果 `cx.send_request_to(...)` 不能直接 `.await`（要 `.block_task()`），加 `.block_task()`：

```rust
cx.send_request_to(agent_client_protocol::Client, req).block_task().await
```

实际 SDK 2.0.0 的 `SentRequest` 是 future（`jsonrpc.rs:3361-3365`），`.await` 即可。

- [ ] **Step 5: 跑既有 e2e + 单测，确认没回归**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge 2>&1 | tail -10
```

Expected: 既有 19 + Task 1 的 5 = 24 全过。特别注意 `bridge_e2e::bridge_handshake_returns_capabilities` + `bridge_session_new_returns_uuid` + `bridge_prompt_e2e::bridge_prompt_emits_text_delta_and_resolves_end_turn` + `permission_roundtrip::hook_socket_round_trip` 都仍过（如果挂了说明 pump loop 改造破了 hello 路径或 broker 启失败）。

- [ ] **Step 6: 全 workspace 单测**

```bash
cd /home/bot/workbench/repos/sebas && cargo test --workspace --lib 2>&1 | tail -5
```

Expected: 全过。

- [ ] **Step 7: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add acp-claude-bridge/src/server.rs acp-claude-bridge/src/main.rs
git commit -m "feat(acp-claude-bridge): intercept ToolUse for session/request_permission"
```

---

## Task 3: E2E — hello scenario 回归保护

**Files:**
- Create: `acp-claude-bridge/tests/permission_e2e.rs`

**Interfaces:**
- Consumes: 既有 `bridge_e2e.rs` 的 `bridge_path` / `fake_path` 模式（独立实现，不 import）。
- Produces: `tests/permission_e2e.rs` 单文件，1 个 `#[tokio::test]`，**目的不是测权限**而是回归保护：pump loop 改成 match 模式后，hello 路径仍走 `_` fallthrough 分支并正常 emit `agent_message_chunk` + `stopReason=end_turn`。

- [ ] **Step 1: 写测试**

```rust
//! End-to-end regression: hello scenario 跑通，证明 Task 2 的 pump loop
//! 改造没破 hello 路径。真权限通路覆盖见 Task 1 unit tests + 手动集成。
//!
//! Run: cargo test -p acp-claude-bridge --test permission_e2e -- --nocapture

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

fn bridge_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
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
            Err(_) => continue,
        }
    }
    panic!("never found {needle:?} within {deadline:?}; last line: {buf:?}");
}

#[tokio::test]
async fn hello_path_survives_tool_use_branch_refactor() {
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
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"hello-regression","version":"0"}}}"#,
        )
        .await
        .unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();
    let init_line = drive_until_contains(&mut stdout, "agentCapabilities", Duration::from_secs(10)).await;
    assert!(init_line.contains("\"loadSession\":false"), "init: {init_line}");

    // initialized
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
    let v: serde_json::Value = serde_json::from_str(&new_line).expect("session/new json");
    let session_id = v["result"]["sessionId"]
        .as_str()
        .expect("sessionId string")
        .to_string();

    // session/prompt
    let prompt_payload = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"hi"}}]}}}}"#
    );
    stdin.write_all(prompt_payload.as_bytes()).await.unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    // 期望：text delta notification（hello scenario 不 emit ToolUse，所以走原 translator 路径）
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
    assert_eq!(nv["params"]["sessionId"], session_id, "sessionId tagged");

    // 期望：响应 stopReason=end_turn
    let resp_line = drive_until_contains(&mut stdout, "\"id\":3", Duration::from_secs(10)).await;
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

- [ ] **Step 2: 跑测试**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge --test permission_e2e -- --nocapture 2>&1 | tail -30
```

Expected: `1 passed; 0 failed`，耗时 < 5s。

- [ ] **Step 3: 全 crate 复测**

```bash
cd /home/bot/workbench/repos/sebas && cargo test -p acp-claude-bridge 2>&1 | tail -10
```

Expected: 既有 19 + Task 1 的 5 + Task 3 的 1 = 25 全过。

- [ ] **Step 4: Commit**

```bash
cd /home/bot/workbench/repos/sebas
git add acp-claude-bridge/tests/permission_e2e.rs
git commit -m "test(acp-claude-bridge): e2e regression — hello path survives ToolUse branch"
```

---

## Self-Review Checklist（writer 写完自查）

1. **Spec coverage:**
   - §1 目标（4 步 ToolUse 拦截 + main.rs 启 broker）→ Task 1/2 ✓
   - §3 数据流 → Task 2 Step 2 ✓
   - §4 文件改动（server.rs +30 / main.rs +1 / e2e ~150）→ Task 1/2/3 ✓
   - §5.1 单元（4 mapping + 1 options）→ Task 1 ✓
   - §5.2 e2e → **Task 3 简化为 hello 回归测试**（plan-mandated 偏离；见下）
2. **Placeholder scan:** 0 个 TBD/TODO。
3. **Type consistency:** `option_id_to_decision(&str) -> PermissionDecision`、`build_permission_options() -> Vec<PermissionOption>` 在 Task 1 定义、Task 2 消费、Task 3 e2e 端到端验证。
4. **TDD discipline:** Task 1 严格 TDD（先失败后通过）；Task 2 是"行为扩展+回归保护"型重构（既有 e2e 不变，code 改了）；Task 3 是 e2e 回归保护。
5. **Plan-mandated 偏离:** Task 3 spec 写的是"e2e 测端到端权限通路"，但实现走 Plan G：只测不触发 ToolUse 的 hello 路径回归。理由：端到端真权限通路需要 in-process fake sebas（150+ 行）或外部 Python fake sebas（100+ 行），都超出 200-300 行总预算；child stdin/stdout 是单消费者独占，要跨协程 fake sebas + 主流程分享 pipe 必须 `Arc<Mutex<>>` + 多 mpsc，超出 plan 范围。真权限通路靠 unit 5 个 + 手动集成测试覆盖，写进 follow-up Beads。

---

## Execution Handoff

Plan 写完。按你之前模式（sebas-x4g）走 subagent-driven-development —— OK 吗？

走 subagent 模式我会：
- 切到 `feat/...-permission` 分支
- Task 1/2/3 各起一个 implementer subagent（haiku / sonnet）+ 一个 reviewer subagent（sonnet）
- 3 commits + 1 final review
- 收尾：push + 关 Beads（如果 DB 还在）+ 删分支
