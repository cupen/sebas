# acp-bridge ToolUse → session/request_permission 设计文档

> 日期：2026-08-01
> 状态：待评审
> 作者：Claude
> 前置：[`2026-08-01-sebas-acp-bridge-design.md`](2026-08-01-sebas-acp-bridge-design.md) §4（"Permission flow"）；[`2026-08-01-acp-bridge-prompt-handler-design.md`](2026-08-01-acp-bridge-prompt-handler-design.md)（prompt handler event-pump loop，本设计的承载层）
> 前置 Beads：sebas-x4g（prompt handler — 已合并到 main，commit `8c15777`）

## 1. 背景与目标

x4g 把 bridge 的 prompt handler 接到了 event-pump loop：每个 `StreamEvent` 翻译成 `SessionNotification` 发回 sebas。但是 `ToolUse` 事件目前直接当普通通知发回，sebas 不拦截——也就是说 se 端的 Feishu 权限卡 UI 不会弹出，用户也无法拒绝 bash 这类工具。这是 sebas ↔ Claude Code 端到端权限流的最后一段空白。

**目标**：pump loop 看到 `StreamEvent::ToolUse` 时：
1. emit `SessionUpdate::ToolCall` 通知
2. 同步 `cx.send_request_to(Client, RequestPermissionRequest::new(...))` 等 sebas 卡片决策
3. 把决策 `PermissionDecision::{Allow, Deny}` 写 `perm_tx`
4. broker 把 "approve"/"deny" 写到 unix socket，hook 脚本 unblock，claude 继续
5. 拒绝时补 emit `ToolCallUpdate { status: Failed }`；允许时不做事，等 claude 自然 emit 的 tool_result 走 translator 闭环

**非目标**：
- `permission.rs` / `hooks/pretooluse.sh` / `acp-claude/` 都不动（broker 早就写好，acp-claude `manager.rs:347-375` 已经能接 `RequestPermissionRequest`）。
- 不改 `fake-stream-claude`。
- Allow session 不发 settings patch（spec §4 明确 v1 不做）。
- `--session-id` resume / cancel / 多 session driver 隔离不在本 ticket 范围。
- macOS BSD nc 不带 `-U` 的可移植性 → follow-up Beads（原 spec Known gaps 已有，本设计继承该限制）。

## 2. 关键决策（已与用户确认）

| # | 决策 | 选择 | 理由 |
|---|---|---|---|
| K1 | ToolUse 拦截点 | 在 pump loop 里同步拦截 | spec 描述 + sebas 侧已用相同模式（`acp-claude/manager.rs:347-375` 同步等 reply），最小 diff |
| K2 | Broker 生命周期 | `main.rs` 启 `tokio::spawn(broker.run())` | broker.run 已写好但 main.rs 之前丢掉了 `_broker` handle；补回 spawn 即可 |
| K3 | 失败兜底 | 出错一律 Deny | 贴 spec "Bridge dies mid-permission → 拒绝" 纵深防御意图 |
| K4 | nc 可移植性 | 不处理，follow-up Beads | macOS 用户当前跑不通；本设计只动 Linux 路径 |

## 3. 架构与数据流

```
pump loop in server.rs:99-121
    │
    ├─── for each event from claude.next_event() ───
    │       │
    │       ├── StreamEvent::ToolUse { id, name, input }  ── new branch
    │       │     │
    │       │     ├── cx.send_notification(SessionUpdate::ToolCall)
    │       │     ├── let decision = match cx.send_request_to(Client, RequestPermissionRequest) {
    │       │     │     Ok(Selected(id)) if id.starts_with("allow_") => Allow,
    │       │     │     _ => Deny,
    │       │     │ }
    │       │     ├── perm_tx.send(decision)       ──→ PermissionBroker.run()
    │       │     │                                       │  writes "approve"/"deny" to socket
    │       │     │                                       ▼
    │       │     │                                  unix socket $XDG_RUNTIME_DIR/sebras-bridge-*.sock
    │       │     │                                       │
    │       │     │                                       ▼
    │       │     │                                  hooks/pretooluse.sh
    │       │     │                                       │  unblocks, exits 0 (approve) or 2 (deny)
    │       │     │                                       ▼
    │       │     │                                  claude --print child
    │       │     │                                       │  continues stream-json
    │       │     │                                       │  user tool_result arrives → existing translator
    │       │     │                                       ▼
    │       │     │                                  StreamEvent::ToolResult → ToolCallUpdate Completed (via translator)
    │       │     │
    │       │     └── if decision == Deny:
    │       │           cx.send_notification(SessionUpdate::ToolCallUpdate { status: Failed, raw_output: "denied by sebas" })
    │       │
    │       ├── StreamEvent::TextDelta / ToolResult / System / TurnEnd  ── unchanged, 走原 translator + from_update
    │       │
    │       └── cancel_flag / EOF  ── unchanged
```

**SendRequest 细节**：
- `agent_client_protocol::schema::v1::RequestPermissionRequest::new(session_id, tool_call, options)` —— **3 参数**（SDK 1.4.0 `client.rs:675`）：`session_id` 来自 `req.session_id`（pump loop 持有），`tool_call` 字段类型是 **`ToolCallUpdate`**（注意：不是 `ToolCall`，虽然名字像"update"实际是 RequestPermissionRequest 用的 type），`options: Vec<PermissionOption>` 是给用户的可选列表。
- `tool_call` 构造：`ToolCallUpdate::new(id, ToolCallUpdateFields { title: Some(name), raw_input: Some(input), ..Default::default() })`。
- `options` 构造：固定 3 项（贴 acp-claude 端 `manager.rs:347-375` 已有的"3 options"模式）：
  ```rust
  vec![
      PermissionOption::new("allow_once",   "Allow once",          PermissionOptionKind::AllowOnce),
      PermissionOption::new("allow_always", "Allow for this chat", PermissionOptionKind::AllowAlways),
      PermissionOption::new("reject_once",  "Deny",                PermissionOptionKind::RejectOnce),
  ]
  ```
- `cx.send_request_to(Client, ...)` 返 `Result<RequestPermissionResponse, Error>`；需要 `await` + 调 `.block_task()`（参考 `acp-claude/manager.rs:423`）。
- `RequestPermissionResponse.outcome` 是 `RequestPermissionOutcome` 枚举：可能 `Selected(SelectedPermissionOutcome { option_id: PermissionOptionId })` 或 `Cancelled`。本设计只识别 `Selected`（含 `option_id` 以 `"allow_"` 开头 → Allow），其他一律 Deny。
- `option_id` 字符串约定来自 `acp-claude/manager.rs:320-329`：`AllowOnce → "allow_once"` / `AllowSession → "allow_always"` / `Deny → "reject_once"`。**关键**：两个允许项都以 `"allow_"` 开头，匹配简单。

## 4. 文件改动

```
acp-claude-bridge/
├── src/
│   ├── server.rs        # pump loop 加 ToolUse 拦截分支（~30 行）
│   └── main.rs          # +1 行：spawn broker.run()
└── tests/
    └── permission_e2e.rs  # 新增（~150 行）
```

**`server.rs` 关键改动**：把 `for update in translator::translate(event.clone())` 之前的 `event.clone()` 拆出，先在 `event` 本身上 match ToolUse 分支，剩余 event 类型走原 translator。`session_id` 是 prompt 闭包顶部的 `req.session_id.0.to_string()`，需要把它从 outer scope 带到 ToolUse 闭包（已经在，闭包顶层就有）。

**`main.rs` 改动**：把 `let (_broker, perm_rx) = permission::PermissionBroker::bind().await?;` 拆成两步：
```rust
let (broker, perm_tx) = permission::PermissionBroker::bind().await?;
tokio::spawn(broker.run());
server::run(claude, perm_tx).await
```

**不动的文件**：
- `permission.rs`：`PermissionBroker::run()` 已写好，`decisions: Arc<Mutex<mpsc::Receiver<PermissionDecision>>>` 自动串行化多个 permission 请求（lock + recv 一对一）。
- `hooks/pretooluse.sh`：已写好，approve/deny exit code 都对。
- `acp-claude/`：sebas 端 `RequestPermissionRequest` handler 已写好，session_id/request_id 映射已就绪。

**行数估算**：

| 文件 | 增量 |
|---|---|
| `server.rs`（pump loop + 工具调） | ~30 |
| `main.rs` | +1 |
| `tests/permission_e2e.rs` | ~150 |
| **合计** | **~180** |

`Cargo.toml` 不需要新依赖（`tokio::sync::mpsc` 已经在 `tokio = { version = "1.48", features = ["full"] }`）。

## 5. 测试

### 5.1 单元测试

`server.rs` 内 `fn option_id_to_decision(id: &str) -> PermissionDecision`（私有 fn + `#[cfg(test)] mod tests`），4 case：
- `"allow_once"` → `Allow`
- `"allow_always"` → `Allow`
- `"reject_once"` → `Deny`
- `"mystery"` → `Deny`

外加 1 个 `fn build_permission_options() -> Vec<PermissionOption>` 单元测试断言 3 项、`id` 字符串稳定（防止 `acp-claude` 端字符串约定同步时漏掉）。

### 5.2 E2E（`tests/permission_e2e.rs`）

**Setup**：
- 起 bridge（`SEBAS_CLAUDE_PATH=./target/debug/fake-stream-claude bash`）—— bash scenario emit 一个 `content_block_start` tool_use + tool_result + result。
- 同时起一个 **fake sebas**（轻量 shell 脚本 or Python one-liner）：读 bridge 的 `session/request_permission` JSON-RPC request，把第一 option 改成 `"allow_once"` 然后写回 `{"jsonrpc":"2.0","id":<req_id>,"result":{"outcome":{"outcome":"selected","optionId":"allow_once"}}}` 到 stdout。
- bridge 同时连 fake-stream-claude + fake sebas（fake sebas 通过... **等等**——bridge 是 agent 端，fake sebas 是 client 端，bridge 的 `cx.send_request_to(Client, ...)` 走的是 bridge 已经持有的 `ConnectionTo<Client>`，即 sebas 端已经在的反向连接）。

**问题点**：bridge 的 `ConnectionTo<Client>` 是 stdio 上的同一根 pipe。ACP JSON-RPC 多路复用——bridge 通过 `cx.send_request_to(Client, ...)` 发出的 request ID 会和 fake sebas 通过 stdin 收到的 ID 对得上，fake sebas 把 response 写回 stdout 就能路由回 bridge 在 await 的 future。

**E2E 步骤**：
1. bridge 子进程启动
2. bridge 写 `initialize` → 读 response
3. bridge 写 `initialized`
4. bridge 写 `session/new` → 读 response
5. bridge 写 `session/prompt`
6. fake-stream-claude bash scenario 触发 tool_use → bridge 拦截 → bridge 发 `session/request_permission` 到 stdout
7. **fake sebas 端**（外部脚本读 bridge 的 stdout，看到 `method == "session/request_permission"`，构造 response 写到 bridge 的 stdin）
8. bridge 收到 response，perm_tx.send(Allow) → broker 写 "approve" 到 socket
9. hook 写 approve，claude 继续 emit tool_result
10. bridge pump loop emit `SessionUpdate::ToolCallUpdate { status: Completed }`（via translator from user tool_result）
11. claude emit result → bridge pump 退出 + respond session/prompt with EndTurn
12. 测试断言：桥端至少 2 条 `session/update` 通知（tool_call + tool_call_update status=completed）+ 最终响应 stopReason=end_turn

**fake sebas 端实现**（独立 Rust binary or 简单 Python 脚本）：
- 选 Python：避免新增 Rust binary 编译时间，测试只在 Linux 跑（已决定 nc 兼容性 follow-up）。
- Python 3 一行 reader，监听 bridge stdout 的 JSON-RPC stream，filter `method == "session/request_permission"`，写回 canned response。

**Or simpler**：不用 fake sebas，把 `permission_e2e.rs` 直接调 `acp-claude` SDK 的 `Client.builder()` 起一个 in-process fake client。这条路更纯 Rust，但需要把 `acp-claude` 加进 dev-dependencies + 编译时间↑。**建议先尝试 Python fake sebas，commit 后再视需要换**。

### 5.3 不测

- 真实 Claude Code CLI（仍由 fake-stream-claude 覆盖）
- cancel / 多 session 隔离 / 跨 session 持久化
- Allow session → settings patch（spec v1 不做）
- macOS nc 兼容

## 6. Commit 计划

单 commit：

```
feat(acp-bridge): intercept ToolUse for session/request_permission
```

范围：仅 `acp-claude-bridge/` 内部 2 文件 + 新 e2e 测试。`cargo test -p acp-claude-bridge` 全绿（既有 25 + 新增 e2e 1）。

按 push policy：1 commit 上 `main`（< 3 commits）。

## 7. 风险与限制

- **fake sebas 端实现选型**：Python one-liner vs in-process Rust fake client。Python 简单但跨平台（macOS BSD nc）的问题我们没解决；如果用户跑 macOS，Python 端 OK（Python stdlib socket 没有 nc 限制）。**实际限制：e2e 测试只在 Linux 跑**。可在测试开头加 `#[cfg(target_os = "linux")]` 跳过其他平台。
- **`cx.send_request_to` 阻塞 pump loop**：sebas 等用户点卡片这期间，claude 子进程的 stream-json 继续在 mpsc 里堆积（容量 64）。正常 tool_use 不会瞬间塞 64 个 event，**风险低**。如果堆积满了，driver 的 `tokio::spawn` line reader 会在 `tx.send(ev).await` 上 await，反压到子进程 stdout，子进程自然 block。可接受。
- **`Cancelled` outcome 处理**：spec 没列举。设计为 `Deny`。如果未来 ACP spec 加 `Cancelled` 走不同路径，需要重新审视。
- **`option_id` 解析的健壮性**：依赖 `acp-claude/manager.rs:320-329` 的字符串约定。如果 acp-claude 改字符串而不同步通知 bridge，permission 决策会变 Deny。**加 doc 注释明确约定**。

## 8. 范围外（next Beads）

- Allow session → follow-up settings patch
- macOS BSD nc 兼容 → 重写 hook 脚本（Python）
- cancel mid-permission → tool call 取消
- 跨 session permission cache（同一 user 在同一 session 重复允许同一工具不必再问）

## 9. 自审清单

- 范围：单 ticket、单 commit、~180 行；不需分解。
- 占位符：无；每节都有具体类型/字段/行数。
- 类型一致：与 `acp-claude/manager.rs:320-329` 的 `option_id` 字符串约定一致（`"allow_"` 前缀匹配）；与 `permission.rs::PermissionDecision` 枚举对齐（`Allow` / `Deny`）。
- SDK 类型校对（writer 自审修正）：
  - `RequestPermissionRequest::new` 实际 3 参数：`(session_id, tool_call: ToolCallUpdate, options: Vec<PermissionOption>)`（SDK 1.4.0 `client.rs:675`），不是 2 参数。
  - `tool_call` 字段类型是 `ToolCallUpdate`（不是 `ToolCall`），构造走 `ToolCallUpdate::new(id, ToolCallUpdateFields { title, raw_input, ..Default })`。
  - `RequestPermissionResponse.outcome` 是 `RequestPermissionOutcome`（单层嵌套，不是双层）；`#[serde(tag = "outcome", rename_all = "snake_case")]` 让 Selected variant 在线协议上是 `{"outcome": "selected", "optionId": "..."}`。
  - `PermissionOptionKind` 枚举：`AllowOnce | AllowAlways | RejectOnce | RejectAlways`（4 个变体）。本设计只用前 3 个。
- 测试：5 unit (4 mapping + 1 options) + 1 e2e，刚好覆盖 happy path + mapping 边界 + options 稳定性。
- 决策：4 个 K1-K4 已与用户确认。
