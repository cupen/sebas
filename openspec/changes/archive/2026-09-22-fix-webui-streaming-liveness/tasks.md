# Tasks: Fix WebUI Streaming Liveness

## 1. claude 驱动 partials（D1）

- [x] 1.1 `sebas-acp/src/claude/driver.rs`：为 `ClaudeAgentOptions` 设 `include_partial_messages: true`，并确认 SDK 子进程命令行带 `--include-partial-messages`（单测断言 launch 参数）
- [x] 1.2 `map_message` 新增 `Message::StreamEvent` 分支：`content_block_delta` → `TextDelta`/`ThinkingDelta`，`content_block_start/stop` → 块边界事件；带 mapper 单测覆盖各子型（verify: `cargo test -p sebas-acp claude`）
- [x] 1.3 完整 `Assistant` 消息改为跳过文本块（只翻 tool use 等非文本块）避免与 partial 重复；补"同一段文本 partial+final 不产生重复 transcript 内容"的单测（verify: `cargo test -p sebas-acp`）

## 2. native 增量转发（D2）

- [x] 2.1 `src/agent_backend.rs`：`pump` 的 `TextDelta` 臂在写 `text_buf` 的同时产出实时 turn 事件；`DualSessionBackend::subscribe_turn_events` 合流 native 路（verify: 现有 native backend 测试 + 新增"pump 期间可见增量事件"测试）
- [x] 2.2 对账测试：同一 native 会话在 webui 面与 `native_dispatch_bridge` 面的 transcript 内容一致（verify: `cargo test --test '*' native` 或既有桥测试套件）

## 3. summary 拆分与 refetch 分流（D3/D4）

- [x] 3.1 `sebas-webui/src/api.rs`：`/api/summary` 移除 `active_session.entries`（保留会话元信息 + `active_session_key`）；更新对应 handler 测试（verify: `cargo test -p sebas-webui`，断言 summary 响应不含 entries 且 KB 级）
- [x] 3.2 `frontend/src/views/dashboard.ts`：`onWsEvent` 按事件类型分流——`turn.append` 只更新 focused 会话缓冲；`session.*` 节流 ≥500ms 刷新列表；`permission.requested` 局部刷新；`loadFocused` 加 `fetchSeq` 代际防回退（verify: vitest 事件分流单测）
- [x] 3.3 前端适配 summary 形状变化，聚焦会话正文走 detail 游标路径（verify: `npm run build` + 既有前端测试通过）

## 4. 渲染三修（D5）

- [x] 4.1 `transcript-view.ts` 滚动修复：贴底跟随判定重写（不依赖 seam 相对位移误判）、`updated()` 增加 `turnUnits` 依赖、滚动提交改瞬时模式（verify: 新增 sticky/scrollTop 单测——此前零覆盖）
- [x] 4.2 seam 模板统一：`showSeam` 真假合并为同一模板字面量，seam 节点恒渲染 + hidden class 切换；二级折叠 `details` open 态持久化（verify: vitest 断言流式中 seam 翻转不触发整块重建、展开态保留）
- [x] 4.3 markdown 增量渲染：流式中的当前条目纯文本 append，条目定稿后一次性 `renderMarkdown`；历史条目经 `repeat` 稳定复用（verify: vitest mock renderMarkdown 计数——长会话流式期间调用次数不再随帧线性增长）
- [x] 4.4 折叠 affordance 轻量化（D5.4）：`transcript-view.ts` 中 process fold 与第二级 fold 的 collapsed 呈现改为单行行内 link（glyph + 标题 + 计数），点击切换展开/收起；移除按钮块/卡框外观（verify: vitest 断言折叠态 DOM 中 link 控件存在且无 `details.process-fold` 大面积容器样式）
- [x] 4.5 展开截断 + 「查看全部」隔离弹层：新增 `truncateHtml(entry): { preview, truncated, omittedLines, omittedChars }`；展开条目如超阈值（`TRUNCATE_LINES ≈ 40` 或 `TRUNCATE_CHARS ≈ 8_000`）渲染预览 + “已截断”明示 + “查看全部”按钮；点击弹 `wa-dialog` 渲完整 renderedHtml，关闭即卸载容器外的 DOM（verify: vitest 覆盖截断逻辑分支、按钮出现条件、弹层开启/关闭 DOM 生命周期）

## 5. 丢帧收敛（D6）

- [x] 5.1 `sebas-webui/src/api.rs` WS 循环：turn/session 两路 `Lagged` 改发 `session.resync` Notification 后继续连接（verify: ws_test 用慢消费者注入 Lagged，断言收到 resync 且连接不断）
- [x] 5.2 core channel 面：`src/core_channel/server.rs` 合并器 `Lagged` 转发 resync 信号给客户端（verify: core_channel 测试）
- [x] 5.3 前端 `dashboard.ts`/`transcript-view.ts`：处理 `session.resync`；游标单调性矛盾检测（游标 > 快照最大 position）→ 清缓冲、游标置空、全量重取（verify: vitest 模拟 core 重启场景收敛）
- [x] 5.4 前端 `ws.ts` 白名单加入 `session.resync`（verify: vitest）

## 6. 端到端验收

- [x] 6.1 e2e：claude 会话流式——webui transcript 在回合进行中分多次帧到达（非一次整块），feishu 卡内容与 transcript 对账一致（verify: testsuite e2e 跑 claude kind）
  - `claude_turn_streams_multiple_frames_to_the_webui`（testsuite_e2e_test.rs，fake-claude 新增 "drip" 场景）：WS 订阅下多帧到达、首帧时刻回合仍在飞（working + 已见首块）、帧序与 transcript 对账逐字各一次（partial+final 无重复，1.3 的进程级证据）。feishu 卡面对账：沙箱无飞书面，由进程内 card_stream_e2e_test（卡面 5 chunk 合帧 + ✅ 收尾）与本用例（transcript 面）共同钉住——两侧消费同一驱动事件流。
- [x] 6.2 e2e：native 会话在 webui 面逐 delta 到达（verify: testsuite process e2e）
  - `native_turn_streams_deltas_to_the_webui`：新增本地 SSE 假上游（tests/support spawn_sse_stub_upstream，4×400ms text_delta），detached 拓扑下经 native pump → 双执行体合流 → core 通道合并器 → webui WS；多帧先于「🗒 turn summary」收尾帧到达、首 delta 时刻快照只有已到部分，transcript 与帧序逐字各一次。IM 桥面粒度一致性由 agent_backend 对账单测（2.2）承载。
- [x] 6.3 性能验收：大会话（≥1000 条 transcript）流式 10s，headless Chrome 采样——summary 响应 <100KB、无 long task >200ms、refetch 不随流式帧数线性增长（verify: /tmp/sebas-probe4.js 探针脚本复跑，指标入 testsuite-webui-browser）
  - HTTP 级已钉（`summary_stays_small_while_transcript_is_large`，fake-claude 新增 "flood" 场景 1200 delta）：1201 条 transcript 下 summary 响应 <100KB 且不含 entries；detail 游标路径（entries_after）只回尾部条目、响应有界。「turn.append 零 refetch / session.* ≥500ms 节流」由 vitest（dashboard.test.ts 的 3.2 分流用例）在组件级钉住。
  - 浏览器级实测（主 agent，/tmp/sebas-probe6.js 改写版 + detached 沙箱 9897 + headless Chrome）：1200 条 flood 会话聚焦下 summary **1.5KB**、无 entries 键；drip 流式窗口 **long task 0 个**（阈值 >200ms）；fetches 60→65 后走平（3 个 turn.append 帧 + 节流刷新，无逐帧风暴）。原 probe4 硬编码操作员实例 9797 且会向真实会话发消息，不可复用，已重写为沙箱版。fold link 轻量形态（BUTTON.fold-link）、点击开合、未读缝渲染均经真实浏览器旅程确认；「查看全部」弹层的分支与 DOM 生命周期由 vitest 钉住（沙箱场景无天然超阈值条目，浏览器级弹层交互未单测）。
