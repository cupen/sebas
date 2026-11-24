# Design: Agent Task Center

## Context

sebas 已有 project（git 目录）+ session（agent 会话）两层概念，缺中间的「任务」协作单元。beads（v1.3.4）是一个仓库级 issue tracker，DB 存于 `<repo>/.beads/dolt/`，提供原子 claim / lease / heartbeat / events journal / comments / dependencies，且天然为 agent 工作流设计（`bd prime` 输出 LLM 友好的工作流指南）。

技术约束：
- 项目规则「一个文件一个写入者」——beads DB 由 `bd` CLI 独占，sebas 不直接读写 Dolt。
- sebas 持久层走 `sebas_db::conn`——但 beads DB 不是 sebas 管的库，不受此约束；sebas 侧的 `session_map` 表加列才走 `sebas_db::schema`。
- 实时性已实测：后端轮询 `bd events tail --since <seq>` 端到端 p95 = 330ms，亚秒达标（spike 报告：`openspec/changes/agent-task-center/spike-report.md`）。

## Goals / Non-Goals

**Goals:**
- 把 beads 作为任务存储与协作引擎，sebas 只做薄封装层（HTTP API + WebUI 渲染 + 权限校验 + 实时推送）。
- 任务 CRUD / 认领 / 状态推进全部走 `bd` CLI（`bd --json`），保留 beads 的原子语义。
- 后端轮询 events journal + WS 推送，前端局部更新看板，端到端亚秒反馈。

**Non-Goals:**
- 不绕过 `bd` CLI 直接写 Dolt（会破坏 claim 原子性、lease、journal 一致性）。
- 不在首版做 agent 自主轮询队列捡活（仅「人派给 agent」）。
- 不做跨机同步的 sebas 侧集成（`bd dolt push` 由用户自己跑；journal per-replica 边界已实测确认）。
- 不把 beads 的优先级 / 依赖图 / Epic / 子任务等高级特性暴露到 UI。

## Decisions

### D1：引擎选 beads，sebas 不重复造轮子

beads 已解决 agent 工作流的硬问题（原子 claim、lease/heartbeat、死工人恢复、claim.pools 池化、agent 身份、comments、dependencies、跨机同步走 git refs），且 `bd prime` 就是为 LLM 设计的工作流指南。sebas 自建 tasks.db 是重复劳动。

**备选**：自建 `tasks.db`（早期草稿方案 B 的 v1）——被否，因为重复造 beads 已有的轮子。

### D2：sebas 通过 spawn `bd --json` 调用，不链库

beads 是 Go 实现，sebas 是 Rust。跨语言链接得走 cgo/FFI，复杂且脆。spawn CLI 的代价是每次操作起进程（~50ms），任务操作 QPS 低（人点击 + agent 轮询），可接受。`bd serve` 提供 HTTP API 但当前版本无 WS/SSE，且增加一个常驻进程的部署负担——首版走 CLI spawn。

**备选**：链 Go 库（否：跨语言复杂）；走 `bd serve` HTTP API（否：无常驻推送能力，且部署复杂）。

### D3：session ↔ 任务关联存于 `session_map.task_id`

任务 1:N session。物理存储在 session_map 表加 `task_id TEXT` 列（存 beads issue id 如 `bd-42`），任务侧不维护 sessions 数组（SQLite 数组要 JSON 序列化，查询/索引都麻烦）。任务的「当前活跃 session」通过查询 `WHERE task_id = ? ORDER BY created_at DESC LIMIT 1` 反查最新一条。

**core 域无关**：core 创建 session 时仅透传 `task_id` 到 session_map，不校验 task 存在性——校验由 WebUI API 层做。失效 task_id 在 WebUI 查询时过滤掉。

### D4：状态机用 beads 原生 + `in-review` label

beads 原生 `open → in_progress → closed`。`in_review` 用 `--add-label in-review` 表达，`done` = `bd close`，`cancelled` = `bd close --reason cancelled`。

**闸门**：`in_review → done` 只能由人或创建时指定的 reviewer agent 推进（reviewer 存于 issue 的 `--metadata '{"reviewer":"..."}'` 字段），由 WebUI API 层强制——beads 本身不管 RBAC。

### D5：实时性 = 后端轮询 events journal + WS 推送

spike 实测：
- `bd events tail --since <seq>` 严格增量、按 seq 升序、载荷含完整 issue 快照。
- 单次 tail p50 = 65ms / p95 = 122ms。
- 端到端 p95 = 330ms（操作发起 → 前端收到 WS 事件）。

**配置**：轮询间隔 300ms，单次 tail 超时 200ms（防止偶发慢查询卡轮询循环）。事件载荷是完整 issue 快照——前端收到直接替换本地状态，无需回查 `bd show`。

**踩坑**：`bd events tail <N>` 位置参数被静默忽略，必须用 `--since <N>` flag。spike 实测发现。

### D6：beads 初始化 = 首次进入任务面板时惰性 init

项目目录下 `.beads/` 不存在时，WebUI 后端跑：
```
bd init --prefix <项目目录名前 4 字符>
bd config set events-journal true
```
已存在则只确保 `events-journal = true`（用户可能已手动 init 过）。不主动建 `.beads/` 的项目零侵入。

### D7：session 终态联动 → 任务加 `in-review`

sebas 的 `SessionPhase::is_terminal()` 终态有 4 个：`Terminated | Closed | Exited | Failed`。精确联动规则：

| session 终态 | 最后一轮 turn 状态 | 任务联动 |
|---|---|---|
| `Closed` / `Exited` | 正常完成（`stop_reason = end_turn`，非 `cancelled`/`error`） | 加 `in-review` label |
| 其他所有情况 | — | 不动 |

**hook 位置**：`on_session_terminal(session)` 在 session 生命周期管理器里，检查条件满足则调 `bd update <task_id> --add-label in-review`。

**边界**：只对「当前活跃 session」生效——历史 session 终态不影响任务状态（避免老 session 关闭触发任务状态回退）。

### D8：WebUI 信息架构 = stage 区加「任务」标签

现有 workbench 是 rail（项目+session 列表）+ stage（会话区）。stage 顶部加 `[会话] [任务]` 切换；切到任务后整个 stage 显示三栏看板。rail 不变。

**备选**：rail 内嵌任务分组（否：信息密度爆炸 + 抽象层级混乱）；独立顶层路由（否：与「任务挂在 project 之下」语义冲突）。

### D9：任务详情页 = GitHub issue 式双栏

左主区：标题 + 描述（markdown）+ 评论流。
右侧栏：元信息（状态/assignee/reviewer/创建时间）+ 操作按钮 + 历史 session 列表。

按钮按状态条件渲染：
- 待认领 → 「认领」
- 自己 claim → 「释放」「进入评审」
- 评审中 → 「完成」（仅人/reviewer 可见）「取消」
- 完成/取消 → 只读

「开一轮会话推进此任务」按钮始终显示（只要有权限），点击复用现有创建对话框（标题改为「推进任务 bd-XX：<标题>」），确认时带 `task_id`。

### D10：权限校验在 WebUI API 层

beads 的 assignee 是自由字符串，不校验用户存在。WebUI API 层在调 `bd` 之前校验：
- `assignee` 必须 ∈ 已注册用户名 ∪ 已配置 agent kind，否则 400。
- `close` 接口仅人或 reviewer agent 可调，其他 agent 调返回 403。
- `claim` 幂等：被自己 claim 返回 200；被他人 claim 返回 409。

## Risks / Trade-offs

- **每次任务操作 spawn `bd` 进程（~50ms）** → 接受：任务 QPS 低，可接受；如未来成瓶颈再考虑链 `bd serve` HTTP API。
- **`bd` 偶发失败（SQLite 写冲突）** → 调用方重试；journal 只记已提交事务，不会留脏事件。
- **per-replica journal 边界**（跨机同步场景下本机 journal 不含远端 mutation） → 首版显式「单机多 agent」边界；二期做 re-baseline（同步后全量重新拉取任务列表）。
- **session 终态联动的「正常完成」判定依赖 turn `stop_reason`** → 已有字段，无需新增；但需在 session 生命周期管理器里加 hook，注意不要让 hook 阻塞主流程（异步 fire-and-forget + 错误日志）。
- **beads DB 损坏/缺失时 WebUI 报错** → WebUI 捕获 `bd` 命令的非零退出 + stderr，向用户显示清晰错误而非 panic。

## Migration Plan

- `session_map` 表加 `task_id TEXT` 列：`sebas_db::schema` 日期戳版本迁移，`ALTER TABLE session_map ADD COLUMN task_id TEXT`（可空，旧数据无需回填）。
- 前端部署：新组件随 `cargo build` 一起打包（前端 dist 嵌入二进制）。
- beads CLI 依赖：文档说明「任务中心需要 `bd` ≥ 1.3.4」；首次进入任务面板时检测 `bd --version`，不可用时显示安装指引。
- 回滚：删 `session_map.task_id` 列不影响现有功能（旧代码忽略未知列）；`.beads/` 目录用户自己决定保留或删除。

## Open Questions

无。所有影响 spec/approach/task 拆解的决策已在 grill-me 阶段敲定。
