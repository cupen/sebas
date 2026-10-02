## Context

会话面（turn_log、泊车审批、会话级 usage）目前只存在于 `sebas-dispatch` 引擎内存（`engine/mod.rs` 的 `turn_log`、`stall.rs` 的 parked 登记、native 侧 `agent_backend.rs` 的 transcript）；落盘通道只有两个：session_map 行的逐变更落库（不含转录）与显式 close 归档快照。`run.rs` 的关停 dump 已退休（run.rs:722 注释），QA round9 实测强杀即全丢。审批应答链路（review-card → `/api/permissions/{id}/answer` → core channel → 引擎 → ACP manager）功能正确（网络级取证 + 既有 `parallel-approval-routing.spec.ts` 在位），但前端 `answering` 态无上限、core 侧无应答留痕，QA 窗口内 4 次点击失效无法归因。项目 rename 无端点（routes.rs 全量核对）；`/router` 不在 `RETIRED_REDIRECTS`；其余为文案/a11y/键盘细节。

状态库布局：settings.db（providers/model_aliases/settings/agents）+ projects.db（projects/session_map），DDL 只允许在根 crate `sebas_state` 的表注册表；写入一律经 `StateWriter` 单写 actor（persistence-runtime 准入）。

## Goals / Non-Goals

- Goals：会话面在进程任意方式终止后恢复到最近 checkpoint；审批应答全程可观测；项目可重命名；工作台细节与规格文案对齐。
- Non-Goals：逐条写穿；退出 dump 复活；看门狗探针协议与「聚焦即拉起」行为变更（见 proposal Non-goals）。

## Decisions

1. **checkpoint 采用「一会话一行快照 blob」而非条目行式表**：projects.db 新表（注册表手写 DDL），行 = (session_id 主键, updated_at, transcript_json, parked_json, usage_in, usage_out)。单行单事务 `INSERT OR REPLACE` 天然满足「完整旧快照或完整新快照」；回放 = 全表扫一遍反序列化。被否的行式方案（entry 一行 + 增量写）写放大更小但对账/裁剪/去重逻辑重，且 flood 场景整块重写 ≈ 百 KB/30s，代价可接受。归档 close 时删除对应行（不复活、不重复计数）。
2. **checkpoint 触发 = 30s 周期 + 审批事件驱动**：引擎侧一个周期任务扫「脏会话」写快照；泊车审批进入/解除时对该会话即时 checkpoint（审批丢失代价高、频度低）。内容未变的会话跳过写入（spec 场景「不重复写」）。写入经 `StateHandle` 单写 actor，与回合事件流异步，不阻塞转发。
3. **回放 = 引擎初始化时一次性回填**：session_map 载入后读 checkpoint 表，逐会话回填 turn_log / parked 登记 / usage；无 checkpoint 的会话保持空转录（现状）。回放后回合状态为已完成（不复活 ACP 子进程）；checkpoint 截断点不伪造内容。泊车审批回放进既有待批读模型，审查卡经既有 restore 链路（fix-webui-approval-restore）自然恢复。
4. **审批应答可观测**：`InProcessBackend::answer_permission` 成功投递处 `info!(request_id, decision, session_id)`（此处三者齐备）；前端 `answer()` 用 10s 超时包裹 POST，超时 → `patch(state:'pending', error:'应答超时，请重试')`，语义 fail-closed 不变（后端是否已送达由 core 日志对账）。
5. **项目 rename 走既有 registry 更新路径**：`POST /api/projects/{id}/rename`，校验非空后更新 projects 表 name 列（注册时 name=basename 的同列），经既有 per-mutation 落库；rail 菜单「重命名」复用会话重命名的弹窗形态。
6. **工作台细节**：`RETIRED_REDIRECTS` 加 `/router`（testsuite-webui-browser 规格本就要求退役路径重定向，此为合规修复）；停滞通知文案改在合成 notice 生成处；回合帧模型名取帧 model 观察值（与会话头同源，缺省不伪造）；设置弹窗容器加 `role="dialog"`/`aria-modal`/可访问名；重命名弹窗输入框 `Enter` 触发确认；工作台形态（项目分组展开 + 聚焦 key）仿 `split-persist` 用 localStorage 持久、启动恢复。

## Risks / Trade-offs

- 快照 blob 随转录增长（flood 1200 条 ≈ 百 KB）：30s 一次整块重写可接受；若未来成为瓶颈，行式方案是预留的迁移路径（表结构变更走既有 schema 版本机制）。
- 回放使引擎启动时内存占用一次性升高（与会话数线性）——与运行期持有量同阶，无新增渐近负担。
- 审批事件驱动 checkpoint 与周期 checkpoint 竞争：同一单写 actor 串行化，无并发风险。
- answering 超时后操作者重试可能与已送达的决定撞 404：既有 expired 语义如实呈现（「已不在待决状态」），不新增状态。
