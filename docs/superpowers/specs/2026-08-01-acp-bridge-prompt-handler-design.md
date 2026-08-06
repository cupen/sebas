# acp-bridge PromptRequest handler — 设计文档

> 日期：2026-08-01
> 状态：⛔ **已被取代**（2026-08-06，sebas-dk8）—— ACP bridge 已删除，sebas 改为经 `cc-agent-sdk` 直连 claude；见 [`2026-08-06-claude-direct-sdk-refactor-design.md`](2026-08-06-claude-direct-sdk-refactor-design.md)。本文档仅留档。
> 作者：Claude
> 前置：[`2026-08-01-sebas-acp-bridge-design.md`](2026-08-01-sebas-acp-bridge-design.md) §3.1；Plan `2026-08-01-sebas-acp-bridge-impl.md` §Known gaps
> Beads：sebas-x4g（P1：把 PromptRequest handler 接到 event-pump loop）

## 1. 背景与目标

`acp-claude-bridge` 已经把三个任务搭好：stream-json parser + ClaudeDriver 子进程管理 + translator + 权限 broker + ACP server 框架。但 **PromptRequest handler 当前是个桩**——它从 `req.prompt` 抽出文本然后立刻 `responder.respond(PromptResponse::new(StopReason::EndTurn))`，根本没把文本写到 `claude` 子进程的 stdin，也没读 `claude` 的 stdout event 流，更没向 sebas 发 `session/update` 通知。

后果：sebas 端 `acp-claude/manager.rs:488-511` 把 `AcpCommand::CreateSession / ContinueSession` 翻译成 `session/prompt` 发到 bridge，bridge 立刻回答 EndTurn，sebas 立刻认为这一轮结束，**整个 Claude Code 通路等于没接通**。

**目标**：把 PromptRequest handler 改成真的 event-pump loop：
1. 把文本写到 `claude --print` 子进程的 stdin。
2. 从子进程 stdout 一路读 `StreamEvent`。
3. 每个 event 走 `translator::translate`，得到的 `TranslatedUpdate` 翻译成 `SessionNotification` 发回 sebas。
4. 遇到 `TurnEnd` 时取出 `stop_reason`，用对应的 `StopReason` 调 `responder.respond(PromptResponse::new(...))` 结束本轮。

**非目标**：
- Permission flow（spec §4）——`ToolUse` 到达时**不**拦截发 `session/request_permission`，**不**等 sebas 回复，**不**写 broker。直接当普通 `SessionUpdate::ToolCall` 发回。权限回路单独开 Beads。
- `--session-id` resume / 跨 session 状态恢复。
- 多 session 并发；多个 prompt 串行化（同一时刻只允许一个 pump 在读 driver）。
- 子进程取消信号；取消只是让当前 pump 跳出循环。
- 自定义 ACP capabilities（保持 Task 9 的 `loadSession=false` 等）。
- fake-stream-claude binary 改动；它现在的 hello/bash/deny 三个 scenario 都能直接驱动新 handler。

## 2. 设计

### 2.1 进程级状态

`server::run` 在构造 builder 之前新增两个共享句柄：

```rust
let gate: Arc<tokio::sync::Mutex<()>> = Arc::new(Mutex::new(()));
let cancel_flag: Arc<std::sync::atomic::AtomicBool> = Arc::new(AtomicBool::new(false));
```

- `gate` —— 串行化所有 prompt handler；一次只允许一个 pump 在跑（`fake_stream_claude` 是单 turn 单退出，并发反而会乱）。
- `cancel_flag` —— CancelNotification handler 只置位，prompt pump 在每个 event 边界检查。**不**向 claude 子进程发 SIGTERM；本 ticket 不扩到子进程控制。

### 2.2 PromptRequest handler 流程

```text
extract session_id, text from req.prompt (text blocks only)
cancel_flag.store(false)
let _g = gate.lock().await                        // 串行化
if claude.send_user(&text).await.is_err():
    responder.respond_with_error(internal_error)  // gate 释放
    return
let stop_reason = StopReason::EndTurn
loop:
    select {
        Some(event) = claude.next_event() =>
            if cancel_flag.load(): stop_reason = Cancelled; break
            for update in translator::translate(event):
                if let Some(notif) = notifications::from_update(&session_id, update):
                    let _ = cx.send_notification(notif)
            if matches!(event, StreamEvent::TurnEnd { stop_reason: sr }):
                stop_reason = sr.into(); break
        None => break                              // 子进程 EOF
    }
responder.respond(PromptResponse::new(acp_stop_reason(stop_reason)))
```

`_g` 在 handler 末尾 drop，自动释放下一轮。

### 2.3 通知映射（新增 `notifications.rs`）

`pub fn from_update(session_id: &str, update: TranslatedUpdate) -> Option<SessionNotification>`：

| `TranslatedUpdate` | ACP `SessionUpdate` | 备注 |
|---|---|---|
| `AgentMessageChunk { text }` | `SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(text))))` | `ContentChunk` 套 `ContentBlock`（`agent-client-protocol` v2 schema） |
| `ToolCall { id, title, raw_input }` | `SessionUpdate::ToolCall(ToolCall::new(id, title).raw_input(v))` | `ToolCall` 字段**不**再包 `fields`，直接 `tool_call_id / title / kind / status / content / locations / raw_input / raw_output / meta` |
| `ToolCallUpdate { id, status, raw_output }` | `SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id, ToolCallUpdateFields { status: Some(s), raw_output: Some(o), ..Default::default() }))` | `ToolCallUpdate` 才有 `fields: ToolCallUpdateFields` 扁平结构；`title=None` 缺省，acp-claude 端 `translate_update` 已经兼容 |
| `TurnEnd { .. }` | `None` | 本地消费；`stop_reason` 走 responder |

`StopReason` ↔ ACP `StopReason` 互转放在 `notifications.rs` 同模块里：

```rust
fn acp_stop_reason(s: claude::StopReason) -> agent_client_protocol::schema::v1::StopReason { ... }
```

映射：EndTurn→EndTurn，MaxTokens→MaxTokens，ToolUse→ToolUse，Cancelled→Cancelled，Unknown(_)→EndTurn（保守回退，spec §3.1 表外情形不暴露 raw 字符串）。

### 2.4 CancelNotification handler

```rust
let cancel_flag = cancel_flag.clone();
async move |_notif: CancelNotification, _cx| {
    cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}
```

注释里写明：本 ticket 不向 claude 子进程发中断；下个 Beads 再处理"取消 tool call 进行中"。

### 2.5 错误路径

- `claude.send_user` 失败 → `respond_with_error(internal_error("send_user failed"))` 后 return；gate 自动释放。
- `claude.next_event` 返 `None`（子进程 EOF）→ 跳出循环，按当前 `stop_reason`（默认 EndTurn）响应。
- `cx.send_notification` 失败（sebas 关闭）→ 记 warn，不打断 pump（sebas 重启会重发 session/prompt，pump 跑完即可）。
- `translator::translate` 内部已容错（`StreamEvent::Unknown` → 空 vec），pump 不感知。

## 3. 文件改动

```
acp-claude-bridge/
├── src/
│   ├── server.rs        # 改 PromptRequest/CancelNotification handler；run() 增 gate + cancel_flag
│   ├── notifications.rs # 新增：from_update + acp_stop_reason
│   └── lib.rs           # +1 行：pub mod notifications;
└── tests/
    └── bridge_prompt_e2e.rs  # 新增：hello scenario 跑通，断言一条 agent_message_chunk + 响应 stopReason
```

行数估算：

| 文件 | 增量 |
|---|---|
| `server.rs`（handler 重写 + 共享状态） | ~50 |
| `notifications.rs`（含 4–6 个单测） | ~80 |
| `lib.rs` | +1 |
| `tests/bridge_prompt_e2e.rs` | ~110 |
| **合计** | **~240** |

`Cargo.toml` 不需要新依赖；`tokio::sync::Mutex` 已在 `tokio = { version = "1.48", features = ["full"] }` 里。

## 4. 测试

### 4.1 单元测试（`notifications.rs`）

4 条：
1. `agent_message_chunk_to_session_notification` —— 断言 `SessionUpdate::AgentMessageChunk` 里 `content.content` 是 `ContentBlock::Text` 且 `text == "hi"`。
2. `tool_call_preserves_id_and_title` —— `ToolCall::new(id, title).raw_input(v)` 的 `tool_call_id.0 == id` + `raw_input == Some(v)`。
3. `tool_call_update_with_output` —— `ToolCallUpdateFields { status: Some(Completed), raw_output: Some(o), .. }` 透传。
4. `turn_end_returns_none` —— 本地消费，函数返 `None`。

### 4.2 E2E（`tests/bridge_prompt_e2e.rs`）

照搬 `tests/bridge_e2e.rs` 的 helper 套路（`ensure_bridge_built` / `bridge_path` / `fake_path`），加一条 `#[tokio::test]`：

`bridge_prompt_emits_text_delta_and_resolves_end_turn`：
1. 启动 bridge，`SEBAS_CLAUDE_PATH=./target/debug/fake-stream-claude hello`。
2. 发 initialize → 读响应（assert `agentCapabilities`）。
3. 发 initialized。
4. 发 session/new → 读响应（assert `sessionId` 存在）。
5. 发 `session/prompt` 带一段纯文本。
6. 读若干行 stdout，断言：
   - 至少一条 `session/update`，`update.sessionUpdate == "agent_message_chunk"`，`update.content.text == "hello from fake claude"`。
   - 一条 `result` 类型的 session/prompt 响应，含 `stopReason: "end_turn"`（`agent-client-protocol` 2.0 schema 的 JSON 字段名）。
7. 写超时保护（`tokio::time::timeout(Duration::from_secs(10), ...)`），避免子进程假死把 CI 拖住。

`fake-stream-claude hello` 的 4 行输出（init / content_block_delta / result）刚好覆盖：system event 被 translator 吞掉（不动）、text delta 触发 `AgentMessageChunk` 通知、result 触发 `TurnEnd` 退出 pump。

### 4.3 不测

- 真实 `claude --print` 子进程 —— 仍受 `fake-stream-claude` 覆盖；e2e 真接通留给 sebas 主仓的 integration test。
- bash / deny scenario —— 留到权限 ticket。
- 并发 prompt —— 串行化是设计目标，不需要正面测。

## 5. Commit 计划

单 commit：

```
feat(acp-claude-bridge): drive event-pump loop in prompt handler
```

范围：仅 `acp-claude-bridge/` 内部 4 个文件 + 新测试。`cargo test -p acp-claude-bridge` 全绿（19 → 20+ 测试，bridge_prompt_e2e 新增的 e2e 是单 test 函数；unit tests 4 条加在 notifications.rs）。

按 push policy：1 个 commit 上 `main`（< 3 commits；sebas 仓库的主仓规则）。

## 6. 风险与限制

- **串行化不解决多 session 干扰**。如果两个不同 `session_id` 同时下发 prompt，第二个会等第一个跑完才进 pump；这是单 `claude --print` 子进程的固有限制，不是本 ticket 范围。
- **Cancel 不杀子进程**。用户发 `/cancel` 之后，本轮 pump 会立刻 break 返 `StopReason::Cancelled`，但 claude 子进程可能仍在写后续 event 到 mpsc（容量 64）。这些 event 不会进任何 `SessionNotification`（handler 已经退出），但会塞 mpsc 直到子进程 EOF 才被清空；下个 prompt 接管 driver 后会被新 `send_user` 触发的新一轮覆盖。可接受；下个 ticket 修。
- **`cx.send_notification` 在 handler 内调用的顺序敏感**。SDK 文档要求 `send_*` 在 `connect_with` 的 future 内调用；handler 闭包就是在这个 future 里，所以合法。

## 7. 范围外（next Beads）

- ToolUse → `session/request_permission` → broker → PreToolUse hook 端到端接通（spec §4）。
- `session/cancel` 向 claude 子进程发 SIGTERM/SIGINT。
- 跨 session 的 driver 隔离（每 session 一个 claude 子进程）。
- bridge 进程内 driver 复用：`session/load` 真正落地。

## 8. 自审清单（writer 写完检查）

- 范围：单 ticket、单 commit、~240 行；不需分解。
- 占位符：无；每节都有具体类型/字段/行数。
- 类型一致：`claude::StopReason`、`TranslatedUpdate::TurnEnd` 与既有的 `acp-claude-bridge` 模块签名一致。
- SDK 类型校对（writer 自审修正）：
  - `SessionUpdate::AgentMessageChunk` 的 payload 是 `ContentChunk`（不是 `TextContent`），内层才是 `ContentBlock::Text(TextContent::new(text))`。
  - `ToolCall` **不**有 `fields` 包装；构造走 `ToolCall::new(id, title).raw_input(v)`。
  - `ToolCallUpdate` 才有 `fields: ToolCallUpdateFields` 扁平结构。
  - `ToolCallId(pub Arc<str>)`，`id.into()` 可从 `&str` / `String` 构造。
- 测试：1 e2e + 4 unit，刚好覆盖 happy path + TurnEnd 边界。
