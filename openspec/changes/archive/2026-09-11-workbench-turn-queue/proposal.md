## Why

工作台在 agent 正忙时再发一条消息，**轮次归属是错的、而且看不见**。core 明明有排队能力（`state.rs` 的 `enqueue_turn`/`pop_next_turn`/`queue_len`）且 Feishu 路径已经在用（`inbound.rs` 的 in-flight 检查 → `enqueue_turn` + ⏳），但 web 消息路径 `TextRoute::Continue` 分支没有这个检查：它在你 POST 的那一刻就把 `prompt` 条目写进 transcript，而本轮 agent 的输出还在继续追加——于是第一条答复的尾巴被排到你的插话之后，读起来像第二条答复的一部分。

同时队列对 WebUI **完全不可见**：`TextRoute::Enqueued` 分支只打一行 debug log，session 面既没有队列长度也没有内容。所以「忙中连发三条」的体验是：三条要么消失、要么长在错误的位置上。队列是 core 的既有事实，缺的是把它变成可观察、可管理的东西。

## What Changes

**BREAKING（行为）**：
- `POST /api/sessions/{key}/message` 在队列已满时**不再静默丢弃并回 200**，改为可见拒绝（4xx + 原因）；「队列 16 上限」从日志约定升级为可观察契约。

**行为修复与不变量**：
- **web 消息路径补齐 in-flight 排队**，与 Feishu 路径同一套语义：会话正在 streaming 时入队，不入 agent；排队中的 `prompt` **不写 transcript**，直到它真正开轮才落条目（`emit_turn_card` → `seed_card`）。transcript 因此天然按轮干净。
- `QueuedTurn` 获得**稳定 id**；新增 `remove(queued_id)` 与 `reorder(queued_id, to_index)` 两个驱动操作。重排只在**非优先段**内生效；`/btw` 优先项恒在最前且不可移动。
- **队列进入观察面**：每个 session 的待执行项以 `(id, 文本, 顺序, 是否优先)` 暴露，in-process 与 detached 两种部署形态给出同一份真相。
- composer 上方新增**待执行堆叠区**：FIFO 展示、可拖拽排序、可逐条删除；一条被 drain 走的瞬间离开堆叠区、以 `prompt` 条目出现在对话里，此后对它 remove/reorder **类型化拒绝**（"已经在跑"）。
- **会话终结不静默丢队**：会话走向 FAILED/被关闭时，未执行项就地标注「未执行」并给一次明确提示；Close 确认对话框点名「将丢弃 N 条待执行消息」。
- 术语消歧：`openspec/glossary.md` 引入 **pending turn / 待执行**，与既有 `SessionStatus::Queued`（子进程尚未产出）区分——先改 glossary 再进 spec。

## Capabilities

### Modified Capabilities

- `session-lifecycle`：修订「Turn queue back-pressure while streaming」（明确覆盖所有通道，含 `web`；排队项落 transcript 的时机 = 开轮）与「Double-spawn race protection」（溢出改为可见拒绝）；新增「排队项可寻址（稳定 id / 移除 / 重排）」；修订「Terminal error teardown」（未执行项不静默丢）。
- `core-session-channel`：修订「Session observation methods」（观察面新增 pending queue）与「Session drive methods」（新增 remove / reorder）。
- `webui`：HTTP route surface 修订（队列随 session payload 下发 + 待执行项 remove / reorder 端点）。
- `agent-workbench`：新增「待执行堆叠区（顺序 / 删除 / 拖拽 / 优先项呈现）」；修订 Close 确认文案要求（点名丢弃条数）。

## Impact

- **Rust**：`sebas-dispatch/src/state.rs`（`QueuedTurn` 加 id、remove/reorder）、`sebas-dispatch/src/engine/mod.rs`（web `Continue` 分支补 in-flight 检查、`Enqueued` 分支入可见队列、终结路径标注未执行）、`sebas-webui/src/api.rs` 与 `models.rs`（session 面携带队列 + 两个新 handler）、`src/core_channel/`（观察/驱动协议扩展）。
- **前端**：`api/client.ts`（类型 + 新端点）、`views/workbench-composer.ts`（堆叠区 + 拖拽/删除）、`components/`（新增堆叠区组件）、`views/session-detail.ts` / `project-rail.ts`（Close 文案点名条数）。
- **e2e**：`tests/testsuite-process-e2e`（忙中入队、prompt 落 transcript 时机、remove/reorder、溢出拒绝、终结标注）；`tests/testsuite-webui-browser`（堆叠区拖拽/删除旅程）。
- **文档**：`openspec/glossary.md`（pending turn）。

## Non-goals

- **对话视图**（放行 `prompt` 条目给 SPA、一回合一个气泡、`/` 与 `/sessions/:key` 两个面合一）——下一个 change `workbench-conversation-view` 做；两者有依赖，本 change 先落地。
- **模型选择器与目录读取**——同属下一个 change。
- **`models` 数据结构优化**（`provider.models: Vec<String>` 的顺序=强弱档、`[1m]` 后缀那套）——另开 change。
- **队列持久化**：队列与 transcript 同生命周期（`turn_log` 在内存，`session-persistence` 明确「运行态不由本 store 持久化」），本 change 不改变这一级别。
- **Feishu 侧的队列管理入口**：IM 只入队，不做增删改。
