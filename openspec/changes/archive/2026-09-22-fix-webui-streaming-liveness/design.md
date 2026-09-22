# Design: Fix WebUI Streaming Liveness

## Context

四路调查（静态三路 + headless Chrome 实测）确立了三层故障点，详见 proposal Why。关键事实：

- WS 传输层已实测通畅：curl 握手后立即收到 `turn.append`；帧形状正确。故障不在传输本身。
- claude 驱动（默认 agent kind）在 `sebas-acp/src/claude/driver.rs:266-281` 未开 `include_partial_messages`，且 `:1114` 显式丢弃 `Message::StreamEvent`——**双层缺口，只开 flag 无效**。
- native 执行体在 webui 面 `src/agent_backend.rs:364-367` 把 TextDelta 只存 `text_buf`，`Finished`/工具边界才整块 flush；同内核在飞书桥 `src/native_dispatch_bridge.rs:85-93` 是逐 delta 的。`subscribe_turn_events`（`:862-868`）只转发 ACP 路。
- 前端 `dashboard.ts:576-579` 对**每个** WS 事件无条件 `refetch()`（5+ 请求），`/api/summary` 内嵌聚焦会话 from=0 全量 transcript（实测 2.8MB / 1074 条），无 gzip。实测浏览器 renderer 546MB RSS / 17.6% CPU 持续；历史实测 Recv-Q 积压 400-518KB。
- 渲染层：`transcript-view.ts:1055-1067` 自动滚动被 unseen seam 卡死（`sticky` 被翻 false 后 early-return）；`:1071-1106` `showSeam` 真假走两个模板字面量导致 Lit 整洞重建（seam 250ms 翻转 = 每 250ms 全量重建）；`:1096,1186` 每帧对全部历史回合重跑 `marked.parse + DOMPurify + hljs.highlightAuto`；`:922-939` `updated()` 不监听 `turnUnits`，纯增量不触发滚动。
- 丢帧不收敛：`api.rs:2261` 把 `Resync` 事件丢弃；前端 `dashboard.ts:91` 的 `sessionEntries` 游标永不作废——core 重启/会话重建后 position 从 0 重计，游标 > 日志长度导致增量**永久**被拒（唯一不收敛路径）。

## Goals / Non-Goals

- Goals：claude token 级流式；native webui 面流式粒度与 IM 面一致；消灭 refetch 放大回路；渲染跟随与增量渲染；丢帧后强制可收敛。
- Non-goals：见 proposal（不动远端节点 150ms 窗、飞书卡、WS 封套协议、压缩中间件）。

## Decisions

### D1. claude partials：flag + StreamEvent 映射一起动

`ClaudeAgentOptions` 加 `include_partial_messages: true`，同时 `map_message` 新增 `Message::StreamEvent` 分支，把 `stream_event` 的 `content_block_delta`（text/thinking delta）翻成 `TextDelta`/`ThinkingDelta`，`content_block_start/stop` 翻成块边界事件。完整 `Assistant` 消息改为**只发非文本块**（tool use 等），文本块跳过以免重复——与 SDK 的 partial 语义对齐。

- 备选：只开 flag（无效，StreamEvent 仍被丢）；保持整块（用户明确要流式）。否决。
- 回归风险：mapper 帧型变化，claude kind 全链路（feishu 卡 / webui / core 通道）需回归测试。

### D2. native 增量转发复用 IM 桥的模式

在 `NativeAgentBackend::pump` 的 `TextDelta` 臂上，除写入 `text_buf` 外，产出轻量 `SessionEvent::Updated` 变体（或复用既有 turn 事件投影），并让 `DualSessionBackend::subscribe_turn_events` 合流 native 一路（复用 `native_dispatch_bridge.rs:85-93` 已验证的逐 delta 模式）。节流复用 engine 侧 250ms 合并窗，不在 backend 层再加窗口。

- 备选：backend 层独立 150ms 窗——多一层窗口语义，否决；直接改 `flush_text` 粒度——仍不实时，否决。

### D3. summary 拆分：瘦 summary + detail 游标拉正文

`/api/summary` 去掉 `active_session.entries`（保留会话元信息与 `active_session_key`）；聚焦会话正文一律走 `/api/sessions/{key}?entries_after=<cursor>`。**BREAKING**（响应形状变化，前端同仓库同步升级，无外部消费者——CLI 走 core channel 面不受影响）。

- 备选：保留但加 `?include=transcript` 参数——留双路径会拖住旧放大回路，否决；gzip 压缩——治标（renderer 仍解析 MB 级 JSON），Non-goal 已排除。

### D4. 前端 refetch 分流 + 节流

`onWsEvent` 按事件类型分流：`turn.append` → 只更新 focused 会话缓冲（帧已含增量，无需请求）；`session.created/updated/removed` → 节流（≥500ms 合并）刷新 sessions/summary 列表；`permission.requested` → 审批卡局部刷新。节点/项目/分支等列表降频到后台轮询或事件驱动，不再跟随每帧。`loadFocused` 加请求代际（`fetchSeq`）防迟到响应回退——沿用 `project-rail.ts:349` 已有模式。

### D5. 渲染三修

1. 滚动：`applyAutoScroll` 在"用户从未上滚"时直接贴底（以 ref/判断替代 `scrollTop≈seamTop-h/2` 误判），`updated()` 增加 `turnUnits` 依赖；提交滚动用 `scrollTop = scrollHeight`（去掉 `scroll-behavior: smooth` 或改 auto）。
2. seam：`showSeam` 真假统一进**同一个**模板字面量（seam 节点恒渲染、用 hidden class 控制显隐），消除 Lit 整洞重建；二级折叠 `details` 的 open 态持久化（foldOpen Map 同款）。
3. markdown：流式中的当前条目用纯文本增量渲染（`textContent` append），条目定稿（turn 结束/条目闭合）后才 `renderMarkdown`；历史条目靠 Lit 模板身份 + `repeat` 稳定复用，不再每帧重解析。
4. 折叠 affordance 轻量化（spec: agent-workbench Δ）：大折叠与二级折叠的 collapsed 态渲染为**单行行内 link**（含类型 glyph + 摘要 + 计数），不再用整块 `<details>` 卡框/按钮块。展开/收起走同一行点击切换；展开后的二级条目仍保持每条目一张 inline 折叠（维持既有 agent-workbench 的 run 聚合语义——这次只改外观与交互成本，不改结构）。
5. 展开截断 + 「查看全部」隔离弹层：展开二级条目时若 `entry.content` 超过阈值（`TRUNCATE_LINES ≈ 40` 或 `TRUNCATE_CHARS ≈ 8_000`，取先到），body 只渲预览部分 + 明示「已截断（省略 N 行 / N 字符）」+「查看全部」按钮；后者打开独立 `wa-dialog`（会话滚动容器之外），渲完整 `renderedHtml`，关闭即销毁 DOM。阈值在实现侧常量，后续可调。

### D6. 丢帧收敛：Resync 传播 + 游标世代作废

- `api.rs` WS 循环遇 `Lagged` 时（turn 与 session 两路）改发 `session.resync` Notification 给浏览器，替代静默 continue；core channel 面同样把 `Lagged` 转成对客户端的 resync 信号（保留现有断连路径为最后手段）。
- 前端收到 `session.resync` 或检测到快照 `entries[0].position` 与本地游标矛盾（游标 > 快照最大 position，即世代回绕）时，清 `sessionEntries[key]` 与 `streamEntries`、游标置空、全量重取。判定用单调性矛盾（简单、无协议改动），不引入显式世代号。
- 备选：协议加 generation 字段——需动 webui-ws-rpc 契约，超出 Non-goal 边界，否决。

## Risks / Trade-offs

- [claude partials 开启后 mapper 处理新帧型引入回归] → 先补 mapper 单测（StreamEvent 各子型），再跑 claude kind e2e（feishu 卡 + webui transcript 对账一致性）。
- [summary 拆分破坏前端旧缓存] → 前后端同仓同发布；`index.html` 加 no-cache 头已有既设资产版本机制兜底。
- [D2 native 转发在高 delta 率下加剧 broadcast 压力] → 复用 engine 250ms 合并窗已限流；`Lagged => resync` 保证慢消费者不静默丢字。
- [D5.3 纯文本渲染期间代码块/富文本样式暂缺] → 仅限流式中的当前条目，定稿即换 markdown 渲染；与主流聊天 UI 行为一致。
- [D5.4 link 化 affordance 改变现有折叠的外观] → 视觉属 UI 演进，`process run` 聚合语义不变；回归测试覆盖点击交互（spec scenario: clicking the affordance toggles the fold）。
- [D6 单调性判定在合法截断场景误判] → 截断（transcript_drop）恰好也是需要作废游标的场景，误判方向与目标一致，可接受。

## Migration Plan

1. 后端先行（D1/D2/D3/D6 后端侧）+ 前端同步分支，同仓一起发。
2. 灰度验证顺序：native 会话流式 → claude 会话流式 → 大 transcript 会话卡顿观感。
3. 回滚：单 revert（前后端同 commit 粒度）即可回到整块模式，无持久化格式变更。

## Open Questions

（无——渲染细节以 D5 为准，如实现中发现 Lit 模板身份问题再在 tasks 内细化。）
