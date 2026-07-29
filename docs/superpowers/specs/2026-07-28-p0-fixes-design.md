# sebas P0 修复 — 设计文档

> 日期：2026-07-28
> 状态：待评审
> 作者：Claude（与 cupen 协作）
> 前置：[`2026-07-26-sebas-design.md`](2026-07-26-sebas-design.md)（下称"原 spec"）

## 1. 背景与目标

2026-07-28 对四个 crate（`router/`、`feishu/`、`acp-claude/`、`src/`）做了对照原 spec 的全面体检。核心结论：**测试全绿，但 daemon 在真实环境不可用**——原 spec §1 头号设计目标"权限请求必须飞书按钮显式确认"在三个独立层面同时断裂，且多处生命周期致命点未被测试覆盖（测试盲区源于 fake-claude 与测试 payload 不够保真）。

**目标**：不动架构、不动卡片流式模型，修复六条 P0 缺陷线，让 daemon 在真实环境跑通最小闭环（发消息 → 流式卡片 → 权限按钮 → 完成/取消）。

**验收方式**：全自动化测试（本次无真机 smoke 条件）。测试保真度本身是本设计的一部分（§6）。

**非目标**：卡片流模型重建、HelpText 实际发消息、transient×3 重试、媒体链路、reaction 状态机、死配置接线、dump 顺序/懒恢复、在飞 prompt 串行化、slash 命令补齐、watchdog/idle_kill——全部留待后续批次。

## 2. 缺陷清单（体检证据）

| # | 缺陷 | 证据 |
|---|------|------|
| D1 | 每次 create_session 发**两次** `session/new`：手动发一次得 id A 作路由键，SDK `start_session()` 内部再发一次得 id B；prompt/权限事件全在 B 上 → router 按 B 反查必然 miss，权限卡永远发不出，子进程按 spec"永不超时"永久死等 | `acp-claude/src/manager.rs:282-298` |
| D2 | ButtonCb 的 session_id/request_id 从 `/action/*` 平铺读，与卡片写入位置（`behaviors[].value`，飞书 V2 回包在 `event.action.value.*`）对不上 → 生产解析为 `""` | `feishu/src/events.rs:95-105`；写入侧 `feishu/src/cards.rs:152-156` |
| D3 | decision 从 `action.value.get("decision")` 读，而 `action.value` 是整个 event 对象（真实路径 `/action/value/decision`）→ **点任何按钮都 fallback 成 Deny** | `router/src/router.rs:217-221`；`events.rs:111` |
| D4 | `/cancel` 发完 CancelNotification 直接 `break` → 子进程 SIGKILL，整个 session 上下文丢失（原 spec §5 要求"取消当前 turn，不动 session"） | `acp-claude/src/manager.rs:325-331` |
| D5 | `startup_timeout_secs` 解析后零使用；`create_session` 的 init 等待无超时，且在唯一出站泵内 inline await → 一个子进程握手挂住 = 全部 chat 出站冻结 | `acp-claude/src/manager.rs:96-98`；`src/run.rs:123-139` |
| D6 | 子进程崩溃/EOF 无任何终态事件上行（transport-error 分支实际不可达）→ router 不摘映射、无 ❌ 卡，用户消息撞 "unknown session" 黑洞 | `acp-claude/src/manager.rs:91-93, 397-405`；`src/run.rs:309-317` |
| D7 | tenant_access_token 启动取一次永不刷新（`expires_at` 算好但零读取）→ ~2h 后全部出站 401，用户无感知 | `feishu/src/client.rs:14`；`src/run.rs:53-57` |
| D8 | 双重 spawn 竞态：映射在 spawn 完成后才插入，两条快速消息都看到空表 → 两个子进程，先到者成孤儿 | `router/src/router.rs:181-187`；`src/run.rs:275` |

## 3. 组 1 · 权限链（D1 + D2 + D3 + D4）

### 3.1 消除双重 session/new（D1）

- 删除 `manager.rs` 中手动发送的 `NewSessionRequest`（约 manager.rs:282-287）；只保留 SDK 的 `cx.build_session(cwd).start_session()`，以其返回的 `session.session_id()` 作为**唯一** session id 回传路由层（经现有 `init_tx` 通道）。
- `build_session` 的 cwd 从硬编码 `"."` 改为使用传入的 `work_dir`（现状：配置了 `[acp.claude] work_dir` 时，真正干活的 session 拿的是 `"."`）。
- 效果：路由 id 与权限事件携带的 id 同源，`lookup_key_by_session` 必然命中；`CancelNotification` 自然使用正确 id。

### 3.2 `/cancel` 语义归位（D4）

- `AcpCommand::Cancel` 分支：发送 `CancelNotification` 后**不再 `break`**，`run_main` 循环继续。
- agent 应答 `StopReason::Cancelled` → 现有 `translate_stop_reason` 已映射为 `Finished`，session 保持存活可继续对话。
- `kill()` / `kill_all()` 的终止语义不变（daemon 关闭仍杀进程）。

### 3.3 ButtonCb 解析归一到 events 层（D2 + D3）

- `CardAction` 增加结构化字段 `decision: Option<String>`；payload 形状知识全部收进 `events.rs`：
  - 主路径：`event.action.value.{session_id, request_id, decision}`（与 `cards.rs:152-156` 写入形状一致，即飞书 V2 卡片回包的真实形状）；
  - 兼容 fallback：旧平铺路径 `/action/session_id`、`/action/request_id`；
  - session_id 缺失时维持现状（默认 `""`，已有 pin 测试），由 router 侧 fail-closed 兜底。
- `router.rs` 改为直接消费 `action.decision`；未知/缺失 decision → 维持 fail-closed Deny。
- **边界决策**：events 层拥有 payload 形状，router 层只拥有策略（默认拒绝）——消除两处各猜一半的现状。
- 现有 `feishu/tests/event_parse_test.rs` 钉的是错误假设形状，按真实 V2 形状改写。

## 4. 组 2 · 生命周期（D5 + D6 + D7）

### 4.1 startup_timeout 接线（D5）

- `SessionManager::new(startup_timeout: Duration)` 持有超时；`src/run.rs` 从 `cfg.acp.claude.startup_timeout_secs`（默认 30s，已在配置中）传入；测试调用点同步更新。
- `create_session` 的 init 等待包 `tokio::time::timeout`。
- 超时/spawn 失败 → 错误经 `dispatch_out` 转为 ❌ 卡（文案含"agent 启动失败/超时，请检查 claude 安装"），不建 session——符合原 spec §4.1 "ACP spawn failure" 行。
- 超时时必须**回收半成品**：`create_session` 持有 `cancel_tx`，超时返回错误前触发它，终止仍在握手的 session 任务与子进程（否则泄漏一个孤儿进程）。

### 4.2 崩溃终态事件（D6）

**不变量：每个 session 要么持续可用，要么以恰好一个 `Error{terminal:true}` 宣告死亡——随后事件通道关闭、`SessionManager` 表无残留。显式 `kill()`/`kill_all()`（daemon 关闭）除外，不产生该事件。**

（`Finished` 是**回合级**事件——agent 每个 StopReason 都会产生，session 继续存活；它不是 session 终态。）

**机制（已核实代码，钉死）：**

- `AcpEvent::Error` 增加 `terminal: bool` 字段（`#[serde(default)]`，旧 fixture 兼容，缺失 = false）。
- 合成点是 `create_session` 里的 **wrapper 任务**（`manager.rs:79-94` 的 `tokio::spawn`）：它 await `run_session(...)`，而 `run_main` 是在 `connect_with` 的 connection future 内被 poll 的——子进程死亡时 SDK 的 `child_wait` 赢得 race、`run_main` future 被 drop，但 **wrapper 任务独立存活并收回控制权**。因此所有退出路径（正常、错误、run_main 被中途 drop）都在 wrapper 处可观察，无需 Drop guard。
- 两个共享标志（`Arc<AtomicBool>`，spawn 前创建）：
  - `expected_exit`：`kill()`/`kill_all()` 在发 cancel 前设置（标志 Arc 存于 `SessionMeta`，kill 从 map 移除条目前先置位）；
  - `terminal_sent`：`run_main` 在任何 exit-bound 发送点置位。
- `run_main` 各退出路径的终态语义：
  - `cancel_rx`（kill）：维持现状（发 `Finished` + break；`expected_exit` 已置位 → wrapper 不再合成）；
  - `/cancel`（§3.2 修复后）：不 break、不手工发 `Finished`；agent 的 `StopReason::Cancelled` 经 `translate_stop_reason` 产生回合级 `Finished`，session 存活；
  - `send_prompt` 失败（manager.rs:338-346, 349-357）：`Error{terminal:true}` + break——session 已坏，诚实宣告；
  - `Refusal`（manager.rs:379-386）：维持现有"杀 session"语义（其合理性问题属后续批次），`Error{terminal:true}`；
  - transport Err（manager.rs:397-405）：`Error{terminal:true}` + break；
  - cmd channel 关闭（None）：break；若属 kill 路径则 `expected_exit` 已置位，否则 wrapper 兜底合成。
- wrapper 在 `run_session` 返回后：若 `!expected_exit && !terminal_sent` → 发送 `Error{terminal:true, message: "agent process exited"}`，然后把死 session 从 `SessionManager.inner` 移除（wrapper 持有 `inner` 的 `Arc` 克隆与共享 session_id 槽 `Arc<Mutex<Option<String>>>`——`run_main` 拿到 id 后写入，wrapper 退出时读取）。sender 全部 drop 后通道自然关闭，泵退出。
- run.rs 泵 → router：收到 `Error{terminal:true}` → ① UpdateCard ❌（"agent 已退出，可 /new 重启"，仅当该 session 有 root msg_id）；② 调 `state.remove_by_session`（已存在但零调用的死代码，正式接线）摘除映射。非 terminal Error 维持现有行为（本批不动卡片模型）。
- 兜底：泵看到通道关闭但未收到终态事件 → `warn!` 日志（防御性，理论上不可达）。

### 4.3 TokenManager 按需刷新（D7）

`feishu` crate 新增 `TokenManager`：

```rust
pub struct TokenManager {
    http: reqwest::Client,
    app_id: String,
    app_secret: String,
    token_url: String,              // 默认官方 endpoint；测试指向本地 stub
    state: tokio::sync::Mutex<FeishuToken>,  // access_token + expires_at（提前 60s）
}
```

- `async fn token(&self) -> Result<String>`：`now >= expires_at` 时先重取再返回。单出站泵场景无并发竞争，`Mutex` 足够。
- 出站调用（send_card/update_card/react）改为经 `TokenManager` 取 token。
- 重试策略（简化决策）：出站 API 返回**任何**业务 `code != 0` → 强制刷新 token 并重试**一次**；再失败按原错误上行（维持现有 log-only 路径）。不为鉴权码维护硬编码集合；代价是非鉴权类错误（如限流）会多做一次廉价的 token 获取，可接受。
- 刷新失败（secret 失效）：错误上行走现有路径；启动时首次取 token 失败维持 fatal 退出——原 spec §4.1 的 transient×3 退避属后续批次。
- 启动时的 `fetch_token`（`run.rs:53-57`）由 `TokenManager::new` + 首次 `token()` 调用取代。

## 5. 组 3 · 双重 spawn 竞态（D8）

### 5.1 映射表引入占位态

```rust
// router/src/state.rs
pub enum MappingState {
    Spawning { pending: Vec<String> },  // 上限 16 条；溢出 log warn 并丢弃最新
    Active   { session_id: String },
}
```

- `Mapping.session_id` 改为 `Mapping.state: MappingState`；`last_active_unix` 字段维持现状（其"从不更新"问题属后续批次）。
- `on_text` 未命中 → **同步**插入 `Spawning`（锁内完成，无 await 窗口）→ 再发 `Out::SpawnAcp`。
- 启动期间到达的消息 → push 进 `pending`，**不再**产生第二个 `SpawnAcp`。
- spawn 成功：`run.rs` 拿到 session_id 后调 `router.activate(key, session_id) -> Vec<String>`：
  - 状态翻转为 `Active`，返回 drained pending；
  - pending 拼接成**一条** `ContinueSession` 补发（`"\n" `连接）。逐条补发会撞"并发 prompt 协议违规"（体检 acp §2.4），逐 turn 排队属后续批次的在飞串行化；拼接在 P0 范围内规避了该问题。
- spawn 失败/超时：`run.rs` 调 `router.fail_spawn(key)` 摘除占位（pending 随占位丢弃）；❌ 卡由 §4.1 的错误路径发出。
- `sessions.json` dump 时**过滤** `Spawning` 条目（重启后子进程已死，占位无意义）；restore 出的条目必为 `Active`。
- `session_alive`（权限回调活性检查）语义不变：仅 `Active` 视为存活。启动期间权限回调不可能到达（尚无权限卡发出），无需特殊处理。

### 5.2 时序（修复后）

```
msg1 → miss → insert Spawning{[]} → SpawnAcp(msg1) ──► create_session(msg1) …慢…
msg2 → hit Spawning → pending=[msg2]                    │
msg3 → hit Spawning → pending=[msg2,msg3]               │
                                                    session_id ok
run.rs: activate(key, sid) → Active{sid}, drain [msg2,msg3]
       → ContinueSession("msg2\nmsg3")
```

## 6. fake-claude 重设计（最小但保真）

**动机**：三条权限链 bug 共同活在"假得足够像"的测试盲区——固定 session id 掩盖 D1，错误形状的 pin 测试掩盖 D2/D3，无权限流的 fake 让 D3 从未被端到端触发。原则：**sebas 对 agent 行为做的每一个假设，fake 都必须能诚实兑现；用不到的一概不建**（不建通用脚本引擎——那是原 spec §4.4 record/replay 批次的事）。

### 6.1 核心行为（无条件保真）

| 行为 | 保真点 |
|---|---|
| `initialize` | 回正确的 protocolVersion + agentCapabilities（`loadSession=false`） |
| `session/new` | **每次返回全局唯一 id**（进程内计数器）；记录 cwd；单进程支持多 session（id → 状态 map） |
| `session/prompt` | 回合边界保真：update 是**通知**（无 id）；回合终止是 **prompt 请求的 response**（带 stopReason） |
| 路由完整性 | 所有 update/response 携带**所属 session 的 id** |
| `session/cancel` | 有在飞回合 → 以 `cancelled` 应答该 prompt 请求，session 保持可用；无在飞回合 → 静默忽略 |
| 能力尊重 | sebas 声明 fs/terminal 能力为 false → fake 永不发这两类请求 |

### 6.2 严格/宽容规则

- 对**请求**严格：收到不认识的 request → 回 JSON-RPC MethodNotFound 错误（sebas 一旦发出意外请求，测试当场失败而非静默超时）。
- 对**通知**宽容：不认识的 notification 静默忽略（符合 JSON-RPC 语义）。

### 6.3 回合内行为（prompt 内容触发）

真实 agent 是"根据任务中途决定要不要权限"，故用 prompt 内容触发（比 argv 场景开关更接近真实语义）：

| prompt | 行为 |
|---|---|
| 其它（默认） | `hello `/`world` 两个 chunk + `end_turn`（现有测试保持绿） |
| `perm` | 发 `session/request_permission`（options 用 claude 真实约定的 `allow_once`/`allow_always`/`reject_once` 作 option_id），**回合阻塞等回复**，收到 decision 后继续走完回合 |
| `crash` | 发完一个 chunk 后**进程 exit(2)**，无协议告别（真实崩溃就是无告别死亡） |

### 6.4 进程级行为（env 开关，仅两个必要的）

| env | 行为 | 服务的测试 |
|---|---|---|
| `FAKE_CLAUDE_HANG_ON_INIT=1` | 永不应答 initialize | startup_timeout（发生在任何 prompt 之前，prompt 触发不了） |
| `FAKE_CLAUDE_DELAY_NEW_MS=<n>` | session/new 延迟 n ms 应答 | 双重 spawn 竞态的确定性复现（不靠时序运气） |

### 6.5 Journal（黑匣子记录仪）

`FAKE_CLAUDE_JOURNAL=/path.jsonl` → fake 把**收到的每一条消息**（request / notification / response）追加写为 JSON 行（方法名 + 关键参数）。测试从 journal 直接断言协议事实：

- session/new **恰好一条** 且 cwd 正确（D1、work_dir 错配一起抓）；
- CancelNotification 的 session_id **与回合 id 一致**（id 分裂类 bug 通杀）；
- permission response 的 option_id == 按钮 decision（端到端：按钮 → router → ACP → agent）。

断言不编码进协议消息（不搞"回显 decision"之类的失真伎俩），协议保持干净。

### 6.6 明确不模拟

LLM 行为、对话历史（每个 prompt 独立回合）、usage/token、modes/plans、fs/terminal handler、多进程并发。

### 6.7 实现形态

保持 `tests/bin/fake-claude.rs` 手撸 JSON-RPC stdio（~150 行 → 预计 ~300 行），不引入 SDK 依赖到测试二进制：SDK agent 侧类型能换编译期保真，但手撸已工作且零依赖，journal 提供运行期保真，权衡后不值得。

### 6.8 已知残余失真（显式记录，后续批次处理）

- 权限 option_id 用 claude 约定字符串：掩盖了 sebas 硬编码 option_id、未读 `request.options` 的问题（体检 acp §2.3）。修复该问题需把 options 列表透传到权限卡，属后续批次；届时 fake 增加"自定义 option_id"模式让该失真暴露。
- SIGTERM 集成测试（`tests/sigterm_cleanup_test.rs`）继续复用本 fake，保持绿。

## 7. 错误处理矩阵（本批改动后）

| 场景 | 行为 |
|---|---|
| spawn 失败 / 握手超时 | ❌ 卡 + 不建 session + 占位摘除（§4.1、§5.1） |
| 子进程崩溃 / EOF | 合成 terminal Error → ❌ 卡（"agent 已退出，可 /new 重启"）+ 摘映射（§4.2） |
| `/cancel` | 取消当前 turn，session 保留（§3.2） |
| 权限按钮（allow/deny） | 正确路由 + 正确 decision；未知/缺失 decision → Deny（§3.3） |
| 权限回调打到死 session | 维持现状（warn log；用户反馈卡属后续批次） |
| token 过期 | 透明刷新；出站 code != 0 → 强制刷新重试一次；再失败走现有 log 路径（§4.3） |
| 启动期连发消息 | 排队（≤16），spawn 完成后拼接补发；溢出 log warn 丢弃最新（§5.1） |

## 8. 测试矩阵（验收标准）

| 组 | 测试 | 断言 |
|---|---|---|
| 1 | acp-claude 集成：create_session 后事件 id 一致性 | 返回 id == 后续事件携带 id；journal 中 session/new 恰一条且 cwd 正确 |
| 1 | acp-claude 集成：prompt `perm` + 模拟 PermissionReply | fake 回合阻塞后完成；journal 中 option_id 与所发 decision 一致 |
| 1 | acp-claude 集成：/cancel | cancel 后同 session 再发 prompt 仍正常完成（无 break）；journal 中 CancelNotification id 正确 |
| 1 | feishu 单测：V2 真实形状 ButtonCb | session_id/request_id/decision 解析正确；旧错误形状 pin 测试改写 |
| 1 | router 单测：ButtonCb allow_once | 产出 `PermissionReply(allow_once)`，非 fallback Deny |
| 2 | acp-claude 集成：HANG_ON_INIT | `create_session` 在超时内报错（测试用短超时） |
| 2 | acp-claude + run 集成：prompt `crash` | 收到 `Error{terminal:true}`；manager 表无残留 |
| 2 | router/run 单测：terminal Error | 映射摘除 + 有 msg_id 时产出 ❌ UpdateCard |
| 2 | feishu 单测：TokenManager（本地 TCP stub） | 过期触发重取；code != 0 触发强制刷新+单次重试；连续失败错误上行 |
| 3 | router 单测：占位态流转 | 连发两消息只发一个 `SpawnAcp`；`activate` drain pending；`fail_spawn` 摘除 |
| 3 | 集成（DELAY_NEW）：竞态窗口内两条消息 | journal：1× session/new + 2× session/prompt（第二条为拼接的 ContinueSession） |
| — | 全量回归 | `cargo test --workspace` 全绿（含 sigterm opt-in 测试） |

## 9. 交付分组

三个 commit，按仓库惯例走独立分支（如 `fix/p0-critical-chain`）：

1. `fix(acp-claude,feishu,router): repair permission chain (single session/new, ButtonCb parsing, /cancel semantics)` — 含 fake-claude 重设计（§6 全部）与本组测试
2. `fix(acp-claude,feishu): session lifecycle (startup timeout, terminal event, token refresh)` — 含本组测试
3. `fix(router): eliminate double-spawn race with Spawning placeholder` — 含本组测试

fake-claude 重设计放在 commit 1：权限链测试依赖它，且它是所有后续测试的地基。

## 10. 风险与备注

- **按钮回调真实形状未经真机验证**：主路径按飞书 V2 文档形状（`event.action.value.*`），兼容 fallback 保留旧平铺路径对冲文档与现实漂移；真机验证时若两者皆失，events 层单点收口，改动面小。
- **pending 拼接的 UX**：多条消息合成一条 prompt，agent 视角是一次追问；可接受，后续在飞串行化批次再细化。
- **SDK 行为假设**：§3.1 依赖 `ActiveSession::session_id()` 公开可取、`build_session(cwd)` 的 cwd 会进入 `NewSessionRequest`（体检已对照 SDK 源码确认 `start_session` 内部仅发一次 `session/new`）；编码时若 SDK 实际 API 有出入，以"全链路单一 session id"不变量为准调整取 id 位置。
- 已核实并排除的旧疑虑：`kill()`/`kill_all()` 本就从 manager 表移除条目（`manager.rs:122, 194`）；evt 通道的 sender 仅有 `run_main` 与权限回调闭包两处，session 任务结束时全部 drop，通道关闭时机可靠。
- 本批过后仍存在的 P1 清单（卡片流模型、HelpText 发消息、重启懒恢复、slash 命令、媒体、reaction 状态机等）已在体检报告中列出，建议转为 beads issues 跟踪。
