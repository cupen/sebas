# sebas — 设计文档

> 日期：2026-07-26
> 状态：待评审
> 作者：Claude（与 cupen 协作）

## 1. 背景与目标

`sebas` 是一个 Rust 实现的个人助理 daemon：将支持 ACP 协议的 agent 接入飞书，让用户能在飞书客户端（私聊 / 群 / 话题）里远程使用这些 agent。

**当前已支持的 agent：** Claude Code。**未来会按需接入其它支持 ACP 的 agent**（如 Codex 等）；架构上为多 backend 预留 `[acp.<name>]` 命名空间。

**当前已支持的平台：** 飞书。其它平台暂不在路线图内。

设计目标：

1. **飞书侧交互最大化** —— 富文本卡片、interactive 按钮、emoji reaction 状态机、媒体转发全部用上，把 agent 的执行过程"直播"到飞书消息流里，而不是等结束后才一次性回复。
2. **本地常驻 daemon** —— 不依赖公网 endpoint，飞书入站走长连接（WebSocket）；用户在自己机器上跑就行。
3. **多 session 隔离** —— 默认按飞书会话（私聊 / 群 / 线程）映射到独立 agent session；同时提供 slash 命令支持手动切换。
4. **安全可观察** —— 所有 agent 权限请求必须用户在飞书端显式按钮确认，**永不超时**。

非目标：

- 多用户 / 多租户
- 飞书以外平台
- 飞书管理后台 / admin web UI

## 2. 总体架构与目录布局

```
sebas/                              ← repo root
├── Cargo.toml                      ← workspace + binary
├── src/                            ← 二进制 main.rs
│   ├── main.rs
│   ├── config.rs                   ← TOML 加载
│   ├── error.rs                    ← 统一错误类型（thiserror）
│   ├── feishu/
│   │   ├── mod.rs                  ← 长连接 client + 事件循环
│   │   ├── cards.rs                ← 卡片渲染
│   │   └── media.rs                ← 媒体上传下载
│   ├── acp/
│   │   ├── mod.rs                  ← 子进程 spawn + stdio 读写
│   │   └── session.rs              ← session 生命周期 + 通知流解析
│   └── router.rs                   ← session 映射 + slash 命令 + 消息分发
├── acp-claude/
│   └── Cargo.toml
├── router/
│   └── Cargo.toml
├── config/
│   └── sebas.toml.example
└── docs/superpowers/specs/
```

> 内部模块作为 sibling crate（目录名 `acp-claude`、TOML section `[acp.claude]`、Rust 标识符 `acp_claude`）被二进制依赖；命名规则为未来扩展 `[acp.codex]` 等留位，当前只 `claude`。

### 进程模型

```
飞书事件 ──►  [src/feishu]  ──mpsc──►  [src/router]  ──mpsc──►  [src/acp]  ──stdin──►  Claude Code 子进程
   ▲                                  │                              │
   │           ◄──mpsc────  路由回包 / 卡片更新            ◄──stdout──  通知流
   └──────────────────────────────────┘
```

### 关键不变量

- 每个 Claude Code 子进程 = 一个 ACP session；生命周期由 `src/acp` 管理
- 每个飞书 chat_id（私聊 / 群 / 线程）= 一个 `SessionKey`，映射到一个 ACP session_id
- 映射存 `Arc<RwLock<HashMap<SessionKey, AcpSessionHandle>>>`，纯内存
- 飞书交互按钮回调（callback payload）携带 `session_id`，router 路由时不必先查表

## 3. 数据流与关键时序

### 3.1 消息信封

```rust
// 飞书 → router
enum FeishuIn {
    Text      { key: SessionKey, text: String, reply_to: Option<String> }
    Media     { key: SessionKey, files: Vec<MediaRef>, caption: Option<String> }
    ButtonCb  { key: SessionKey, action: CardAction }   // payload 含 session_id + request_id
}

// router → 飞书
enum FeishuOut {
    SendCard    { key: SessionKey, card: Card }
    UpdateCard  { key: SessionKey, msg_id: String, card: Card }
    React       { key: SessionKey, msg_id: String, emoji: String }
    Unreact     { key: SessionKey, msg_id: String, emoji: String }
}

// router → acp
enum AcpCommand {
    CreateSession    { key: SessionKey, prompt: String, media: Vec<PathBuf> }
    ContinueSession  { session_id: String, prompt: String, media: Vec<PathBuf> }
    PermissionReply  { session_id: String, request_id: String, decision: Decision }
    Cancel           { session_id: String }
}

// acp → router（notification 流）
enum AcpEvent {
    TextDelta         { session_id, delta }
    ThinkingDelta     { session_id, delta }
    ToolStart         { session_id, tool_name, args }
    ToolProgress      { session_id, tool_name, progress }
    ToolEnd           { session_id, tool_name, result }
    PermissionRequest { session_id, request_id, tool_name, args }
    Finished          { session_id, usage }
    Error             { session_id, error }
}
```

### 3.2 卡片更新模型（streaming 的核心）

每条用户消息触发一张 **root card**（带 `msg_id` 引用）。Claude Code 后续的 thinking / tool_call / text_delta 都原地 patch 这张卡，不刷屏：

```
用户: "重构 src/foo.rs"
  ↓ feishu 收到
[card v1, msg_id=X]  🛠 收到任务，准备中…                    ← 初始
  ↓ acp ToolStart
[card v2 patch X]    🛠 收到任务  \n ┄┄┄┄┄  \n 📖 Read foo.rs
  ↓ acp TextDelta
[card v3 patch X]    🛠 收到任务  \n ┄┄┄┄┄  \n 📖 Read foo.rs  \n 💬 "我会先看上下文…"
  ↓ acp ToolEnd
[card v4 patch X]    …加上 Read foo.rs 的输出摘要
  ↓ acp Finished
[card v5 patch X]    ✅ 完成，react ✅
```

实现：router 维护 `session_id → root_msg_id` 的映射。每个 AcpEvent 触发一次 `UpdateCard`，并在第一次 ToolStart / TextDelta 时把 root card 的 emoji 反应从 👀 换成 🚧，Finished 时换成 ✅。emoji 反应**只挂在 root card 一条消息上**，不污染对话流；中间每个 tool 不单独加 emoji。

### 3.3 关键时序

**(a) 首次接触（无 session）：**

1. 飞书消息到达 `feishu` → `FeishuIn::Text`
2. `router` 查 `sessions` 表，未命中
3. `router` 调 `acp.create_session(prompt)` —— 子进程 spawn
4. `router` 在表里建条目，`key → handle`
5. `router` 发 `FeishuOut::SendCard`（初始卡）→ 拿到 `msg_id`
6. `router` 把 `session_id → msg_id` 存起来，供后续 update
7. 子进程持续推送 AcpEvent → router → UpdateCard

**(b) 持续对话（已存在 session）：**

- 命中映射表 → `acp.continue_session(session_id, prompt)`
- 直接进入卡片增量更新循环

**(c) 权限审批：**

1. `AcpEvent::PermissionRequest` 到达
2. router 发 `SendCard`（带 [Allow once] [Allow session] [Deny] 按钮的权限卡）→ 拿 `msg_id`
3. router 存 `permission_msg_id → (session_id, request_id)`
4. 子进程 stdout 暂停读取（stdin 不写，直到用户回复）
5. 用户点按钮 → 飞书回调 `FeishuIn::ButtonCb` → router 查表 → 发 `AcpCommand::PermissionReply`
6. 收到 reply 后继续读 stdout

**(d) Slash 命令：**

- `/new`：router 直接 `acp.create_session()`，不发 ACP prompt
- `/sessions`：router 查表，列当前所有活跃 session
- `/switch <n>`：切换当前 chat 的指向（hybrid 模型 B 入口）
- `/compact`、`/cost`、`/model`、`/cd`、`/resume`：转发给 ACP
- `/cancel`、`/status`、`/help`：router 自己处理

**(e) 重启恢复：**

- 启动时读 `~/.config/sebas/sessions.json`（router 退出时 dump 的映射表）
- **不**主动 respawn 所有 session
- 等飞书消息到达对应 SessionKey 时懒加载：`acp.spawn_resume(session_id)`
- 如果 respawn 失败（session 文件已删等），fallback 到 `create_session`

**(f) 媒体消息：** 用户发图片 / 文件 / 语音时，`caption`（用户打的文字描述）拼进 prompt 一起发：`"<caption>\n\n[attached: <path1>, <path2>]"`。文件先存到 `[media] download_dir`，路径作为附件引用传给 Claude Code（由 Claude Code 自己读文件内容）。语音按原文件转发 —— Claude Code 自行决定是否调用 STT / vision。

## 4. 错误处理 + 测试

### 4.1 错误分类与策略

| 类别 | 触发 | 处理 |
|------|------|------|
| Feishu transient | 网络抖动、429 限流、token 过期 | 指数退避重试 3 次；3 次后 update 当前卡片显示 ❌ + 文字原因 |
| Feishu auth | bot app_secret 失效、bot 被禁用 | 致命：log fatal，daemon 退出，让 systemd 拉起前先修复 secret |
| ACP child crash | 子进程非预期退出（stdout EOF） | router 标记 session 死；飞书卡片 update 为 ❌ + "agent 已退出，可 /new 重启"；**不影响其他 session** |
| ACP child hang | 子进程 5min 无任何 notification | router 发 `Cancel`；3 次 cancel 无响应则 SIGTERM → 5s 后 SIGKILL |
| ACP spawn failure | 找不到 `claude` 二进制、PATH 错 | router 在卡片上显示 ❌ + 安装提示；session 不创建 |
| Mapping miss | 收到 ButtonCb 但 session 已死 | log warn，回复"该会话已结束，操作无效" |
| Channel send fail | router → feishu mpsc 满 / 断 | panic in dev（`RUST_ENV=dev` 或 cfg 标志）；prod log error + 继续 |
| User input | `/switch xxx` 不存在 | 回复 usage hint |
| Permission timeout | — | **永不超时**。子进程 stdout 暂停期间持有 child handle，不做超时清理 |

### 4.2 守护进程生命周期

```rust
// main.rs 伪代码
fn main() -> Result<()> {
    let cfg = Config::load()?;
    let router = Router::new(cfg.clone()).await?;
    router.restore_sessions().await?;     // 读 sessions.json
    let feishu = FeishuClient::connect(cfg).await?;
    let (feishu_tx, feishu_rx) = mpsc::channel(256);

    let acp = AcpClient::new(cfg.clone());

    tokio::select! {
        r = feishu.run(feishu_tx) => error!("feishu exited: {r:?}"),
        r = router.run(feishu_rx, acp) => error!("router exited: {r:?}"),
        _ = signal::ctrl_c() => info!("SIGINT received"),
    }

    router.dump_sessions().await?;          // 退出前落盘
    router.shutdown_children(5).await;      // SIGTERM → 5s → SIGKILL
    Ok(())
}
```

### 4.3 测试

**单测（per crate，源码内 `#[cfg(test)]`）：**

- `cards.rs`：snapshot 测试 —— 同一序列 AcpEvent 渲染成同一张卡（`insta`）
- `router.rs`：mock `FeishuIn` / `AcpEvent`，断言路由 + `FeishuOut` 序列
- `acp/session.rs`：解析 ACP JSON（用录制 fixture）

**集成测试（`tests/`）：**

- `tests/acp_against_canned_binary.rs`：spawn `tests/bin/fake-claude.rs`，按 stdin 命令回放 fixture JSON 流
- `tests/feishu_card_golden.rs`：给定 AcpEvent 序列，断言产出卡片 JSON 匹配 golden

**端到端（手动，README 文档化）：**

- 真飞书 bot + 真 Claude Code，跑在 sandbox 容器
- 不进 CI，跑前手动 smoke

**覆盖率目标（cargo-llvm-cov）：**

- `router/` ≥ 90%
- `cards.rs` ≥ 90%
- 整体 ≥ 80%

### 4.4 Fixture 录制工作流

`tests/fixtures/acp/<scenario>.jsonl` —— 每行一个 JSON-RPC 消息。开发时：

1. 真跑一段交互
2. `sebas record --output fixture.jsonl` 子命令从 stdio 抓包
3. 脱敏后 commit

## 5. Slash 命令

| 命令 | 处理者 | 说明 |
|------|--------|------|
| `/new` | router | 当前 chat 强制开新 ACP session；旧 session 保留但不再路由 |
| `/sessions` | router | 列出所有活跃 session：编号 / chat 名 / 创建时间 / 最后活跃 / 当前任务摘要 |
| `/switch <n>` | router | 把当前 chat 的路由指向第 n 个 session（hybrid 模型 B 入口） |
| `/resume <id>` | router | 加载 Claude Code 历史 session 到当前 chat |
| `/cancel` | router → acp | 取消当前 turn（不动 session） |
| `/status` | router | 显示当前 session 信息：model、累计 token、当前工具、cwd |
| `/compact` | router → acp | 转发 |
| `/cost` | router → acp | 转发 |
| `/model <name>` | router → acp | 转发 |
| `/cd <path>` | router → acp | 转发 |
| `/help` | router | 渲染帮助卡片 |

**前缀冲突处理：** 用户消息以 `/` 开头才被识别为命令。**转义：** 消息以 `//` 开头 → 去掉一个 `/` 后透传给 Claude Code。

**反馈卡片：** 命令执行结果统一用一张单行小卡片回复（不同于普通任务卡片样式）。

## 6. 配置

**设计原则：只有 `app_id` / `app_secret` / `owner_id` 这 3 个字段是必填的；其它所有字段都必须有默认值，用户不写就能跑。** 任何实现阶段的字段都必须满足这条 —— 不允许出现"没填就用不了"或"用 panic 提示缺配置"的情况，缺了就走默认值。

### 6.1 最小配置

```toml
[feishu]
app_id = "cli_xxx"
app_secret = "..."
owner_id = "ou_xxx"
```

3 行就能跑起来。

### 6.2 完整字段与默认值

| 字段 | 默认值 | 备注 |
|------|--------|------|
| `[feishu] app_id` | **必填** | 也可 `SEBAS_FEISHU_APP_ID` |
| `[feishu] app_secret` | **必填** | 也可 `SEBAS_FEISHU_APP_SECRET` |
| `[feishu] owner_id` | **必填** | 单用户鉴权 |
| `[feishu] allowed_chat_types` | `["private", "group"]` | |
| `[acp.claude] path` | `"claude"` | 二进制名或绝对路径 |
| `[acp.claude] args` | `[]` | 透传给 claude 的额外参数 |
| `[acp.claude] sessions_dir` | `"~/.claude/sessions"` | Claude Code 自身 session 目录 |
| `[acp.claude] work_dir` | sebas 启动时 cwd | |
| `[acp.claude] startup_timeout_secs` | `30` | |
| `[acp.claude] idle_kill_secs` | `172800` | 48 小时空闲自杀；任意 AcpEvent / FeishuIn 都会重置计时器 |
| `[router] state_file` | `"~/.config/sebas/sessions.json"` | |
| `[router] channel_buffer` | `256` | |
| `[router] max_concurrent_sessions` | `32` | 超出时新飞书消息回 "系统繁忙" 卡片，不创建新 session |
| `[card] theme_color` | `"blue"` | |
| `[card] max_user_text_chars` | `4000` | |
| `[card] max_tool_output_chars` | `2000` | |
| `[card] fold_long_output` | `true` | |
| `[media] download_dir` | `"~/.cache/sebas/downloads"` | |
| `[media] max_file_size` | `52428800` | 50 MB |
| `[log] level` | `"info"` | trace / debug / info / warn / error |
| `[log] file` | `null` | null=stderr；绝对路径则写文件 |

> **飞书入站走长连接（WebSocket）**，无需 `verification_token` / `encrypt_key` —— 这两个是 webhook 模式才用得到的。

### 6.3 优先级与 env vars

优先级：CLI flags > env vars > TOML > 默认。

env vars（仅覆盖敏感字段）：

- `SEBAS_FEISHU_APP_ID`
- `SEBAS_FEISHU_APP_SECRET`
- `SEBAS_LOG_LEVEL`

### 6.4 启动校验

`Config::load()` 阶段：

1. 文件存在性 + toml 解析
2. 必填字段检查（`app_id` / `app_secret` / `owner_id`）
3. 路径展开（`~` → `$HOME`）+ 目录可写性检查
4. 子进程二进制可达（`which claude` 或绝对路径）
5. 任何一步失败 → 友好错误退出，**不** panic

## 7. 待定 / 后续

> **已完成**：飞书长连接（WebSocket）事件循环已与 `dispatch_out` / `SessionManager` 全链路接线 —— 入站消息会真正 `create_session` + 派发 ACP 命令，ACP 事件经 per-session pump 回流刷新卡片，关闭时 `kill_all` 收尾。此前的「WS 循环待接线」TODO 不再适用。

- `sebas record` 子命令本身的设计（fixture 录制工具）—— 等 ACP 客户端稳定后再细化
- 飞书群聊 @ 机器人 的具体消息格式（@ 消息 vs 普通消息的 payload 差异）—— 实现阶段确认
- Claude Code ACP 子命令的精确协议（`claude --acp` 还是其它）—— 实现前从 Claude Code 文档 / 实测确认
- 多用户 / 配额等目前不在范围，但 SessionKey 已预留 user_id 字段，将来扩展不需要破坏性改动