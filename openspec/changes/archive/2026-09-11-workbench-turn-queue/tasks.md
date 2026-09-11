## 1. 术语先行

- [x] 1.1 `openspec/glossary.md` 新增 **pending submission（待生效提交）** 词条，含两种处置 **staging（并入首条消息）** 与 **queued turn（按序执行的待执行回合）**，并显式消歧 `SessionStatus::Queued`（子进程尚未产出）；验证：词条存在且 `openspec validate workbench-turn-queue` 仍通过。
- [x] 1.2 全仓盘点 `queued` 一词的既有用法，列出需要改口径的 spec 段落清单；验证：清单落到本 change 的 review 记录里（spec 用词更改在 6/7 组任务里落地）。

## 2. core：统一提交路径与 prompt 落点（design D3/D4）

- [x] 2.1 抽出共享 `submit_turn(key, session_id, prompt, priority, origin)`，Feishu `inbound::continue_session` 与 web `TextRoute::Continue` 分支都改调它；验证：`cargo test -p sebas-dispatch` 通过，且新增单测「WORKING 时两条路径都入队、都不发 SendAcp」。
- [x] 2.2 删除 web 路径提交时的 `transcript_push(TurnEntry::prompt)`，prompt 条目一律由 `seed_card` 在开轮时写入；保留入队时的 `publish_updated`（last_active 不变）；验证：新增单测断言「入队后 transcript 无该 prompt，开轮后出现在队尾」。
- [x] 2.3 进程级 e2e 断言时序（忙中提交 → transcript 不出现该提交 → 本轮结束后出现，且本轮 agent 输出位置连续不被切开）；验证：`invoke testsuite-e2e --case <新用例名>` 通过。

## 3. core：pending submission 数据模型（design D1/D2/D7）

- [x] 3.1 `SessionMap` 增 per-key 单调 id 计数器；staging 的 `Vec<String>` 与 `turn_queue` 的 `QueuedTurn` 统一带 id，形成 `PendingSubmission { id, text, position, disposition, priority }` 视图；验证：`state.rs` 单测「入队分配唯一 id、顺序稳定、drain 后 id 失效」。
- [x] 3.2 实现 `remove_pending(id)` 与 `move_pending(id, to_index)`，全部判定与变更在 session map 单写锁内完成；验证：单测覆盖 `Unknown` / `AlreadyStarted` / `PriorityConflict` / `OutOfRange` 四种拒绝与两条成功路径。
- [x] 3.3 并发用例：remove/move 与 `activate`/`drain_queue_if_terminal` 竞态下不产生「已删除的条目仍被投递」或「已在跑的回合被回滚」；验证：`state.rs` 并发单测（tokio 测试）通过。

## 4. 观察面与驱动面（design D6）

- [x] 4.1 `SessionInfo` 增全量 `pending`，快照与每次会话事件都携带；验证：`src/core_channel/tests.rs` 断言 snapshot 与事件里的 pending 内容与顺序。
- [x] 4.2 core channel 新增 remove / move 两个 op，与 in-process 实现共用同一 typed 拒绝；验证：core_channel 测试对同一场景在 in-process 与 detached 两种形态给出相同结果。
- [x] 4.3 webui 的 detached 后端把队列面接上（`session_backend.rs` 的观察/驱动方法）；验证：`sebas-webui` 测试断言 detached backend 能读到 pending 并成功 remove/move。

## 5. 出口：溢出与会话终结（design D5）

- [x] 5.1 `route_text` 满队列分支改为携带拒绝原因的变体；web 路径映射 4xx（409），Feishu 路径发一条提示消息；验证：单测断言第 17 条不再回 200，且已暂存条目不被顶掉。
- [x] 5.2 core 在移除映射前发出 `PendingDropped`（携带被丢弃条目的 id + 文本），`close` 响应带 `discarded_pending: N`；验证：进程级 e2e 断言 terminal error 后观察者收到标注、close 计数正确。

## 6. webui 后端 API（design D5/D6）

- [x] 6.1 `GET /api/summary` 的聚焦会话与 `GET /api/sessions/{key}` 的 payload 带 `pending`（id/文本/顺序/disposition/priority，按投递序）；验证：`openspec` 之外的 webui api 测试断言字段与顺序（沿用既有 `api_json` 测试助手）。
- [x] 6.2 新增 `POST /api/sessions/{key}/pending/{pending_id}/remove` 与 `.../move`（body `to_index`）；验证：api 测试覆盖成功、未知 id 4xx、已开始 4xx、越级越过优先项 4xx。

## 7. 前端：待生效堆叠区（design D8）

- [x] 7.1 新增 `<sebas-pending-stack>` 组件，渲染在 composer 上方：两种处置文案（「将并入首条消息」/「待执行 · 第 N 位」）+ 优先项标记；验证：`pnpm test` 组件单测覆盖顺序、文案、优先渲染。
- [x] 7.2 删除按钮 + HTML5 拖拽排序 + 键盘可达的上移/下移；操作后乐观更新并以服务端返回的 pending 对账，`AlreadyStarted` 静默刷新；验证：组件单测（含「先判后动」不产生非法落点）。
- [x] 7.3 会话终结的未执行提示（一次性 notice，列出被丢弃条目）；验证：组件单测 + 浏览器 e2e 旅程「关闭带队列的会话 → 见提示」。
- [x] 7.4 Rail 关闭确认对话框点名将丢弃的条数；验证：`views/project-rail.test.ts` 新增用例。
- [x] 7.5 提交时的追加语义（堆叠区非空时提交是追加，不覆盖既有条目文本）；验证：`workbench-composer.test.ts` 新增用例。

## 8. 门禁与验收

- [x] 8.1 `cargo fmt --check`、`cargo clippy --all-targets`、`cargo test --workspace` 全绿；验证：三条命令输出无 error。
- [x] 8.2 前端门禁：`pnpm run tsc`（或等价 typecheck）与 `pnpm test` 全绿；验证：命令输出。
- [x] 8.3 进程级 e2e：`invoke testsuite-e2e` 全绿（含本 change 新增用例）；验证：任务输出摘要。
- [x] 8.4 浏览器 e2e：`invoke testsuite-webui-server`（Playwright）堆叠区旅程通过；验证：任务输出摘要。
- [x] 8.5 spec/术语一致性复查：`openspec validate workbench-turn-queue` 通过，且 spec 用词与 glossary 的 pending submission 口径一致；验证：命令输出 + 人工核对清单。
