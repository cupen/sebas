# Review 记录 — workbench-turn-queue

## 1.2 全仓 `queued` 用法盘点（glossary pending submission 口径落地清单）

任务 1.1 后，`queued turn` 已是 glossary 定义的 pending submission 处置之一；
`SessionStatus::Queued`（webui 会话行状态词，子进程尚未产出）已显式消歧。
主规格（openspec/specs/*）现有 `queued` 用法逐段核对：

| 位置 | 现文 | 判定 |
|---|---|---|
| `session-lifecycle` "Lazy spawn on first message" / "Double-spawn race protection"（queued messages / is queued） | 指 spawn 窗口的暂存提交 | 本 change delta 改写为 staged / pending submission 口径（已落地） |
| `session-lifecycle` "Turn queue back-pressure while streaming"（enqueued / queued turn） | 指流式期间排队的待执行回合 | 本 change delta 改写并明确覆盖 web 通道；"queued turn" 与 glossary 处置名一致（已落地） |
| `session-lifecycle` "Post-completion turn drain"（Queued turn runs after completion） | 指按序执行的待执行回合 | 与 glossary `queued turn` 处置一致，**无需改** |
| `session-lifecycle` "Terminal error teardown"（queued turns dropped） | 指未执行的待生效提交 | 本 change delta 改写为 pending submissions + 未执行上报（已落地） |
| `dispatch-commands` `/btw`（enqueued ahead of ordinary queued turns） | 指插队到待执行回合之前 | 与 glossary 一致，**无需改** |
| `feishu-cards`（message is enqueued, no new card） | 泛指消息排队，非本 change 引入的二义 | **无需改** |
| `agent-workbench` / `project-session-actions` Rail close（starting/queued/working） | 这是 `SessionStatus` 的 slug 词表（会话行状态），glossary 已显式消歧「与 pending submission 无关」 | **无需改**（保留 SessionStatus 口径） |
| `router-auth-rate-limit`（records queued while writer stalled） | 限流写入队列，无关 | **无需改** |

结论：需要改口径的段落全部由本 change 的 session-lifecycle delta 承载；
其余用法要么与 glossary `queued turn` 处置名天然一致，要么属无关域/已消歧的
`SessionStatus` 词表。

## 任务 0 记录

webui delta 的 `HTTP route surface` MODIFIED 块已按归档后的主规格现文重生成：
provider 管理簇改回「core 状态库经 core 通道履行、不代理 router 进程、
core 不可达 503」，路由枚举织入两个 pending 端点，新增 pending payload 段落
与 4 个队列场景；主规格全部 22 个场景原样保留。`openspec validate
workbench-turn-queue --strict` 通过。

`openspec list`（实施开始时）：`workbench-conversation-view` 仍活跃
（0/24 tasks）——归档时序上它在本 change 之后，detail payload 形状
（user_prompt/body → 条目序列）由它 BREAKING。
