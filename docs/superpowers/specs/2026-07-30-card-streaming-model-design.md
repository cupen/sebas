# sebas 卡片流模型重建 — 设计文档

> 日期：2026-07-30
> 状态：待评审
> 作者：Claude（与 cupen 协作）
> 前置：[`2026-07-26-sebas-design.md`](2026-07-26-sebas-design.md) §3.2（下称"原 spec"）；P0 修复已合并到 main（commits 79f98ea/c814fb0/7151557/54ab696）
> Beads：sebas-tz4（P1: 卡片流模型重建）

## 1. 背景与目标

P0 修复让核心闭环（消息 → 卡片 → 权限按钮 → 完成）在代码与测试层面正确了，但**卡片流式模型仍然实现错了**——这是用户体验最显眼的缺口：原 spec §3.2 要求"thinking / tool_call / text_delta 都原地 patch 这张卡，不刷屏"（v1→v5 同卡累积），实际实现是每个事件重建一张空卡整卡替换，历史被冲掉、Finished 时 transcript 清空。

**目标**：把卡片流模型重建为同卡累积，复活被丢弃的事件类型，接上 `[card]` 死配置，并加节流防刷屏/防限流。

**非目标**：真 emoji reaction 状态机（sebas-p3g，独立 bead）；媒体；slash 命令；重启恢复——本设计只动 root 卡的流式路径，permission 卡（独立 SendCard）不动。

## 2. 缺陷清单（main 当前状态）

| # | 缺陷 | 证据 |
|---|------|------|
| C1 | 每个事件 `render_root_card("", &session_id, emoji)` 重建空卡，只 `apply_event` 当前一个事件，整卡 PATCH 替换 → 历史全被冲掉，卡片永远只显示最新一个事件 | `router/src/router.rs:147-164`（共享臂） |
| C2 | `Finished` 时整张卡只剩 `> ` + 分隔线 + `msg_id: ...` + `✅ 完成`，整个执行过程被清空 | 同上 |
| C3 | `ThinkingDelta / ToolProgress / ToolEnd` 落入 `router.rs` 的 `_ => {}` 被丢弃；`feishu/src/cards.rs:174-204` 为它们写的渲染分支是死代码 | `router/src/router.rs`（共享臂只列 TextDelta/ToolStart/Finished/Error）；`cards.rs:174-204` |
| C4 | `[card]` 配置全死：`max_user_text_chars=4000` / `max_tool_output_chars=2000` / `fold_long_output` / `theme_color` 解析后零使用；cards.rs 唯一截断是 ToolEnd 硬编码 200；theme 硬编码 `"blue"`/`"orange"` | `src/config.rs:130-164`；`feishu/src/cards.rs`（无 CardConfig 参数） |
| C5 | terminal Error 臂（P0 Task 9 加的）也重建空卡，丢失死前累积的 transcript | `router/src/router.rs:127-146` |
| C6 | 无节流：text delta 高频时逐条 UpdateCard 会撞飞书发送限流 + 刷屏（原 spec §3.2 "不刷屏"未落实） | 无节流代码 |

## 3. 架构方案（已选 A）

**状态在 router、节流在 pump。**

- router 持 `session_id → CardState`（纯状态），pump 负责节流计时。
- pump 收事件 → `router.apply_event`（只更新状态，不发 Out）→ 重置 150ms debounce → 到点 `router.flush_card`（序列化整卡发 `Out::UpdateCard`）。
- `Finished` / terminal `Error` 立即 flush（最终态必现）。

职责分离：router = 状态，pump = 时序。复用现有 per-session pump（`src/run.rs::spawn_acp_pump`），router 不引入后台任务生命周期。

## 4. CardState 与接口

### 4.1 状态结构（router 新增，平行于 `MsgIdMap`）

```
session_id → CardState {
    user_prompt: String,       // seed 时写入，重渲染引用块用
    status_emoji: String,      // 👀 / 🚧 / ✅ / ❌
    body: Vec<CardElement>,    // 累积的 transcript（append-only，受 §7 限制）
}
```

`CardStateMap`（`Arc<RwLock<HashMap<String, CardState>>>`，同 `MsgIdMap` 的并发模型）。

### 4.2 router 新增/改动方法

- `pub async fn seed_card(&self, session_id: String, user_prompt: String)`：SpawnAcp 臂发完 root 卡后调用，初始化 CardState（prompt + 👀 + 空 body）。幂等：已存在则保留（防止 SpawnAcp 重入冲掉已累积状态）。
- `pub async fn apply_event(&self, session_id: &str, event: &AcpEvent)`：**纯状态变更**——`apply_event_to_card`（cards.rs 现有逻辑，复活 ThinkingDelta/ToolEnd/ToolProgress）append 进 body + §7 截断/fold/总量控制 + 更新 status_emoji（按 §5 FSM）。**不发 Out。** session 无 CardState 时按需 lazy seed（prompt="" 兜底，防早到事件）。
- `pub async fn flush_card(&self, session_id: &str)`：序列化 `render_accumulated_card(&state)` → 发 `Out::UpdateCard { session_id, card }`。无 CardState 则 no-op。不维护 dirty flag——节流契约（§6）保证 flush 只在"debounce 到点"或"Finished/terminal 即时"被调，不会冗余。
- `pub async fn drop_card(&self, session_id: &str)`：session 死亡/通道关时清 CardState（防无界增长）。terminal Error 处理后调。
- `pub async fn apply_event_to_out(&self, session_id: String, event: &AcpEvent)`：**保留**，语义改为 `apply_event + flush_card` 的薄封装（同步 flush）。供 terminal/Finished 即时路径与旧测试复用——回归零破坏。

### 4.3 render_accumulated_card（cards.rs 新增）

```
pub fn render_accumulated_card(
    user_prompt: &str, session_id: &str, status_emoji: &str,
    body: &[CardElement], theme: &str,
) -> Card
```
构建：header(`{emoji} Claude Code`, theme) + 引用块(`> {user_prompt}`) + 分隔线 + body 各元素 + footer 灰注(`msg_id: {session_id}`)。`render_root_card` 退化为 seed 时的初始卡构建器（不再被每个事件调用）。

## 5. status emoji FSM（header title）

```
👀 ──(首个 TextDelta/ThinkingDelta/ToolStart)──► 🚧 ──(Finished)──► ✅
                                            └──(terminal Error)──► ❌
seed = 👀
```
- 转移在 `apply_event` 内完成（事件驱动，非时序）。
- 已是 🚧/✅/❌ 后不再回退到 👀。
- terminal Error：置 ❌（即便之前是 🚧）。

## 6. 节流契约（pump 侧，`src/run.rs::spawn_acp_pump`）

pump 改造为 select 循环：事件到达 vs debounce 计时。

- **流式事件**（TextDelta / ThinkingDelta / ToolStart / ToolProgress / ToolEnd）：`router.apply_event`（累积）→ 重置 150ms 计时。计时到点 → `router.flush_card`。
- **Finished**：`router.apply_event`（置 ✅）→ 取消计时 → 立即 `router.flush_card`。session 保持存活（spec：Finished 是回合级，非 session 终态）。
- **terminal Error{terminal:true}**：`router.apply_event`（置 ❌ + append 正文）→ 立即 `flush_card` → `router.remove_by_session`（保留 P0 Task 9 清理）→ `router.drop_card` → pump 退出。
- **通道关闭（recv → None）**：`drop_card`（防泄漏）→ pump 退出。
- debounce 计时：`tokio::time::sleep(Duration::from_millis(150))`；用 `Option<Sleep>` + select 的 `pending()` 兜底无计时分支。确切的 async 机制由实现计划钉死，验收以契约为准：**事件即时累积、出站 UpdateCard 在 150ms 内至多一次、Finished/terminal 立即出最终态**。

`dispatch_acp_event`（router 入口）改为调 `apply_event`（不发 Out）；pump 是唯一调 `flush_card` 的地方（除即时路径）。

## 7. 截断 / fold / 总量（接 `[card]` 死配置）

`CardConfig` 从 config 传入 router（`RouterHandle::new(map, card_cfg)` 或字段）。`render_accumulated_card` / `apply_event` 接收 `&CardConfig`。

- **单元素截断**：append 后，单个 markdown 元素文本 > `max_user_text_chars`(4000) → 截断到上限 + 追加灰注 `(已折叠 N 字)`。`fold_long_output=true` 时启用；`false` 时不截断（但仍有 §总量兜底）。
- **fold 语义（tool result）**：`ToolEnd.result` > `max_tool_output_chars`(1024，软上限) 且 `fold_long_output=true` → 装进原生 `collapsible_panel`（默认折叠），完整内容保留；`max_tool_output_chars=0` → 完全不输出 tool call 结果内容；代码硬上限 10240，超出才在面板内截断 + 灰注。面板需飞书客户端 V7.9+。
- **总量上限**：body 累积字符 > 24000（飞书 interactive 卡 ~30KB 内容上限留余量）→ 从最旧的非 divider 行丢弃，直到回到预算内。divider（`CardElement::Hr`）随其后的内容一起丢弃。
- `theme_color`：`render_accumulated_card` 用它替代硬编码 `"blue"`；权限卡 `"orange"` 保留（独立卡路径）。

## 8. terminal Error 并入累积模型

P0 Task 9 的 terminal Error 臂当前重建空卡（C5）。改为：走 `apply_event`（append `❌ {message}` + 置 ❌）→ `flush_card` → `remove_by_session` → `drop_card`。死前累积的 transcript 保留可见（spec §4.1 "可 /new 重启" 的 ❌ 卡带上下文，UX 更好）。

非 terminal Error（目前无产生路径，Task 7 后 run_main 所有 Error 都 terminal:true）维持现有 🚧 共享臂行为——本设计不为其单独处理。

## 9. 测试策略（全自动化，无真机）

- **累积单测**（router）：seed("hi") → 连发 TextDelta("a")/ToolStart/ThinkingDelta/ToolEnd/ToolProgress → 断言 `apply_event` 期间无 Out → `flush_card` 产**一张** UpdateCard，正文含全部事件渲染、emoji 为 🚧。
- **FSM 单测**：seed 👀 → TextDelta → 🚧 → Finished → ✅；seed → terminal Error → ❌。
- **节流单测**（pump 集成）：fake-claude `stream` 模式连发 5 个 TextDelta → 断言 150ms 窗口内只发 1 个 UpdateCard（含 5 段正文）→ Finished 后立即再发一个（✅）。
- **截断/fold 单测**：单元素 > 4000 / ToolEnd > 2000 / body 总量 > 24000 各自的截断 + 灰注 + 丢旧行为。
- **terminal 保留 transcript 单测**：累积若干事件 → terminal Error → 断言 ❌ 卡正文含死前 transcript + 错误正文。
- **回归**：`apply_event_to_out` 同步语义保留 → 现有 `router_test`/`e2e_test`/`terminal_error_test` 零改动通过。
- **全量**：`cargo test --workspace` 全绿 + SIGTERM opt-in。
- fake-claude 加 `stream` prompt：连发 N 个 `"chunk{i} "` TextDelta + `end_turn`。

## 10. 不做（防蔓延）

- 真 emoji reaction（👀→🚧→✅ 作为飞书 reaction，而非标题文字）—— sebas-p3g。
- 媒体链路、slash 命令补齐、重启恢复、ACP watchdog、飞书事件去重/群聊@——各有独立 bead。
- permission 卡（独立 `Out::SendCard`）的渲染路径不动。
- 卡片 V2 collapsible 元素（不存在原生支持，用截断 + 灰注兜底）。

## 11. 风险与备注

- **节流的 async 机制**（`Option<Sleep>` + select）留给实现计划钉死，验收以 §6 契约为准。
- **CardState 与 MsgIdMap 的生命周期对齐**：terminal Error 时两者都清；正常 session 死亡（通道关）两者都清。`drop_card` + `remove_by_session` 配对调用。
- **lazy seed 的 prompt 兜底**：事件可能在 SpawnAcp 臂的 `seed_card` 之前到达（pump 启动早于 root 卡发送？实际 run.rs 顺序是 root 卡发送后才起 pump，故 seed 先于事件——但 lazy seed 仍作为防御）。
- **body 总量预算 24000** 是经验值（飞书卡 ~30KB 上限留 20% 余量）；真机若仍超限，§7 的丢旧策略保证不炸（最坏退化成只留近期几行）。
- 本设计过后，sebas-tz4 关闭；原 spec §3.2 的 v1→v5 模型成立。
