## Context

本变更同时触碰前端（`project-rail` / `session-detail` / `workbench-composer`）、webui 后端（`sebas-webui/src/api.rs`）与 core 侧（`src/agent_backend.rs` 的 ACP spawn 路径），且要把一个已存在但语义模糊的字段（create-session 的 `model`）从「silent no-op」升级为对 ACP 真实生效——属于跨层、带歧义消解的中等复杂度变更，值得先定调再动手。

现状关键事实（调研确认，未凭记忆）：

- 项目移除与会话关闭的**后端端点均已存在**（`projects_remove` @ `sebas-webui/src/api.rs:735`、`close_session` @ :548）；前端 `api.projects.remove` 与 `api.closeSession` 封装也已存在，但 rail 组件从未调用。
- 占位会话「发不出消息」的根因是 focus 指针：`create_session` 后端会 `set_focus`（`api.rs:496`），但**前端 rail 创建占位后直接 `navigate(/sessions/{key})`，而 dashboard 的 composer 模式由 summary 的 `active_session_key` 驱动**——深链页（session-detail）本身不持有 composer，操作员回到工作台时若 focus 未对齐，composer 停在创建模式，把首条消息当新会话 spawn。
- `POST /api/sessions` 的 `model` 字段对 ACP 当前是「silent no-op」（注释见 `api.rs:451-454`）；native 侧 `create_placeholder` 已会 `set_session_model`（`src/agent_backend.rs:782`），ACP 侧没有对应路径。

## Goals / Non-Goals

**Goals:**

- rail 内完成项目/会话的删除闭环（复用既有端点，不新增后端写操作）。
- 占位会话创建后 composer 立即处于跟随模式，首条消息必达该会话。
- ACP 会话创建时选定模型在**首回合之前**生效（经 `session/set_config_option` 下发），被拒时如实报错。

**Non-Goals:**

- 不新增会话「硬删除」语义；close 即终止+移除映射，与既有 spec 一致。
- 不重构 focus 机制本身（仍是 display-only 指针，不影响路由）。
- 不为无模型面的 agent（claude code）伪造下拉。
- 不改动归档/retention、native 模型路径、provider catalog 数据源。

## Decisions

### D1：删除入口放在 rail 行内，确认弹窗复用 wa-dialog 模式

- **选择**：项目行/会话行各加一个 hover-reveal 的 `.row-action` 风格按钮；确认用与「Add project」一致的 `wa-dialog` 内联弹窗，错误就地渲染在弹窗里。
- **备选**：跳到独立 `/sessions` 列表页删除——违背「工作台是主界面」的 IA v2 定位，已否决。
- **会话删除的确认分级**：inactive（dormant/done/failed）直接执行；active（starting/queued/working）弹确认。依据：误删 dormant 会话的代价低（子进程已不在），误杀 working 子进程不可逆。这与 `sessions.ts` 列表页现有「close 需确认」的语义不完全一致，但 rail 是高频操作区，分级确认更贴操作员心智。

### D2：focus 同步靠「前端补 switch + 后端语义显式化」双侧落实，而非改 create 语义

- **选择**：
  1. 前端 rail `+` 创建占位会话后，**先调 `switchSession(key)` 再 `navigate`**——让 focus 指针与 URL 同时到位，工作台 composer 由 summary 的 `active_session_key` 驱动自然进入跟随模式。
  2. 后端 `switch` 端点的「设置 focus」语义在 spec 中显式化（webui delta 的 MODIFIED 段），并把「深链页访问即聚焦」这条既有行为补写成 scenario。
  3. `session-detail.ts` 深链页加载时补一次 `switch` 调用（幂等），覆盖「从书签/外部链接直达深链页」的路径。
- **备选 A**：让 `create_placeholder` 之后由后端主动推送 focus 变更——通道层没有 focus 推送帧，扩展协议成本高，且 focus 本来就是 webui 侧指针，不该泄漏到 core。
- **备选 B**：composer 自己猜（URL 里有 key 就跟随）——把 focus 真相源复制到前端路由，两处真相会漂移，否决。

### D3：ACP 创建时模型下发放在 spawn_with 内、首条 prompt 之前

- **选择**：`src/agent_backend.rs` 的 ACP `spawn_with` 路径在拿到 session 建立应答后、注入首条 prompt 前，若 `model` 非空则先走一次 `session/set_config_option`（与 `set_session_model` 同一通道命令）；agent 回 typed rejection 时把该错误作为 spawn 的拒绝原因上抛，会话不静默回退默认模型。无模型面的 agent（configOptions 无 model 项）按既有约定直接跳过（silent no-op，与 native 侧「记住但不强制」的语义对齐）。
- **备选**：让前端在创建后立刻补一次 `setSessionModel`——race：首条 prompt 可能先于模型下发到达 agent，第一回合用错模型；且「创建即带模型」是后端字段语义，不该让前端编排两步。
- **0-turn 占位路径**（`create_placeholder`）：子进程尚未 spawn，`model` 记入 mapping，首条消息触发真实 spawn 时随 spawn_with 同路径下发——两条路径在「prompt 前下发」这一点收敛。

### D4：模型被拒即 typed rejection，不静默回退

- **选择**：创建时带的模型被 agent 拒绝（non-terminal Error 或 terminal），后端以与 `set_session_model` 相同的 `SessionRejection` 形状上抛，前端在 composer 内联错误中呈现，会话保持存活但 `current_model` 不变。
- **依据**：与既有「set_session_model rejects unknown model」scenario（webui spec 补充段）同构；操作员显式选了模型却被静默换掉，是「不诚实」的失格行为。

## Risks / Trade-offs

- [rail 行内按钮变多（archive + close + new-session + remove），误点率上升] → 确认分级（D1）+ hover-reveal 降低视觉噪声；active 会话删除必须过弹窗。
- [深链页补 switch 引入「访问即改 focus」的副作用] → focus 本就是 display-only 指针（spec 明示），改它不产生路由/投递影响；且这正是 spec 已声明的既有行为，本次只是补全实现缺口。
- [ACP spawn 前多一次 set_config_option 往返，spawn 延迟增加一个 RTT] → 仅在 `model` 非空时发生；本地通道 RTT 极小，可接受。
- [「无模型面 agent 跳过模型下发」依赖 configOptions 探测，探测失败时可能漏下发] → 探测失败按「无模型面」处理（保守跳过），操作员仍可在会话建立后用跟随模式切换——与现状等价，不退化。
- [关闭聚焦会话后 composer 回到创建模式，操作员可能误以为消息已发出] → composer 创建模式的 placeholder 与 follow 模式文案本就不同，且舞台空态有「No session focused」提示；不额外加 toast。

## Migration Plan

纯增量：无 wire 形状变更、无配置变更、无状态文件格式变更。上线顺序无所谓前后端先后——

- 后端先上：旧前端不调新语义，行为不变。
- 前端先上：rail 删除按钮调用的是既有端点；focus 补 switch 调的是既有端点；ACP 模型字段后端本就接受（no-op），只是不生效，不报错。

回滚 = revert 对应 commit，无数据迁移。
