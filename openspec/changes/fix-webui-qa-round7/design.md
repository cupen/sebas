## Context

第七轮 GUI 验收（缺陷账本：`C:/Users/cupen/AppData/Local/Temp/sebas-qa-shots/`，含 harness/results.jsonl 与 93 张截图；沙箱 9879/10879 由主会话管理）定位了两个 P1、一个 P3 与三项打磨点。代码事实（file:line 均已核实）：

- **长文本撑破面板**：正文折行选择器只覆盖 `p, pre, ul, ol, h1-h3` 七种标签（`transcript-view.ts:1070-1072`），用的是不参与 min-content 计算的 `overflow-wrap: break-word`；GFM 开启后表格（`components/markdown.ts:12-15`）无任何规则；meta 行 `.author` 无收缩守卫（`transcript-view.ts:1021-1024`），兄弟 `.time`/`.receipt` 均 nowrap。外层约束链（main/outlet/turn-block 的 min-width:0 + max-width）完备，缺口集中在内容层。
- **set_config 拒绝回合停滞**：webui `POST /sessions/{key}/model` 是 fire-and-forget（`session_backend.rs:669-692`）；fake-acp 拒绝后 driver 只发 `AcpEvent::Error { terminal: false }`、不发 Finished（`sebas-acp/src/acp_driver/mod.rs:354-374`）；调度引擎 FSM 只有 `Finished → DONE` 一条收尾边，非终态 Error 还会把 SEED 推成 WORKING（`sebas-dispatch/src/engine/mod.rs:2774-2791`、`:1407-1416`）→ 回合只能等 600 秒 watchdog（`src/run.rs:387-405`，`engine/mod.rs:2581-2683` 强收并注入停滞条目）。虚假「操作者中断」：`cancelled_turns` 是 per-session 无回合身份的 HashSet（`engine/mod.rs:338`、`:2088-2095`），陈旧标记会被任意后续回合的 Finished 消费（`:1407-1416`）。
- **WS 重连噪音**：闸机制已在（`setAuthGated`，`ws.ts:394-404`），但依据是 app-shell 鉴权状态迁移而非升级失败本身；升级被拒在客户端只表现为 close 且 `everOpened=false`，走固定 30s 慢梯反复重试（`ws.ts:489-501`），模块装载即急连首连（`shared-ws.ts:13-29`）。
- **打磨点**：`MODE_OPTIONS` label 英文（`mode-vocabulary.ts:47-52`）；浅色 composer 常态边框观感近似错误色（`workbench-composer.ts:1063-1075`，实现期核实实际生效变量）；时间戳位置两侧不一致（`renderOperatorUnit` :1962-1970 vs `renderAgentUnit` :1990-1994）。

## Goals / Non-Goals

Goals：两个 P1 修复且各具进程级/浏览器级回归断言；WS 噪音消除；P3 三项打磨；全部进 COVERAGE 账本。
Non-Goals：见 proposal（agent 自广告文案、watchdog 时长、通用 ACP usage、移动端）。

## 关键决策

- **D1 长文本**：约束补在内容层三处——① `.body` 整体升级为 `overflow-wrap: anywhere`（参与 min-content 收缩，才是真折行；备选「只扩标签选择器到 table/img」被否：break-word 不解决固有宽度）；② markdown `table` 补 `display: block; overflow-x: auto; max-width: 100%`（保表格语义、横向滚动而非撑破）；③ `.meta .author` 补 `min-width: 0` + 省略号守卫。验收口径：2100+ 字符无空格消息 scrollWidth ≤ 视口 + 余量，重渲染后不回漂。
- **D2 set_config 拒绝终态化**：修复在状态语义层，不许 UI 超时遮罩。拒绝回执（driver 的 Error 事件）按成因分类：**无真实回合在跑时**（typical：拒绝发生在占位/SEED 窗口），引擎不得推进相位，直接以失败终态收尾该占位回合；**打断真实回合时**，该回合以失败终态收尾。实现落点二选一由实现期定（driver 补发 terminal 回执，或引擎把该类 Error 视为终态边），验收钉行为：拒绝后远短于 600s 收尾、composer 恢复、watchdog 未触发。`cancelled_turns` 从 per-session 集合改为按回合身份（turn id / 回合序）关联消费，陈旧标记自然过期，不注入后续回合。
- **D3 WS 重连闸联动**：升级失败（close 且 `everOpened=false`）视作疑似未认证：暂停重连梯进入静默等待，等待态的解除条件 = app-shell 鉴权闸解除（既有 `setAuthGated(false)` 路径）。备选「识别 401 状态码」被否：升级拒绝在 WS API 里拿不到 HTTP 状态，只有 close 事件。既有「曾打开过 → 指数退避」自愈路径不动。
- **D4 打磨**：`MODE_OPTIONS` label 改双语（如 `Ask · 逐次询问`）或中文主 label + 英文副标注，实现期按视觉密度二选一，不改发送值与门控（round6 D6 的等价标注语义保留）；composer 常态边框先核实实际生效变量再改值，验收口径=浅色常态不得呈现错误语义色（红棕系）；时间戳统一到单一约定（用户气泡内 vs 行中，实现期按现有视觉惯性选一，两侧一致即可）。`/goal` tooltip 英文属 fake 桩自广告内容，不改（记录为「按设计」）。

## 风险

- D2 的 Error 分类可能牵出 core-session-channel / sebas-node 的 ack 语义（`sebas-node/src/body.rs:844-852` 同样只认 Finished 清 turn_active）；若根因跨 crate，修复面在本 change 内闭环，不改 wire。
- `overflow-wrap: anywhere` 对 CJK 与代码块的影响需浏览器级回归：代码块 `pre` 保持既有 nowrap/滚动语义不回退。
- cancelled_turns 语义收紧后，真实「审批等待中点停止」路径必须仍能如实标注中断——回归用例要覆盖真中断与假中断两面。
