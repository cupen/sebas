## 1. 会话面持久化（session-transcript-durability）

- [x] 1.1 状态库注册 checkpoint 表：根 crate `sebas_state` 表注册表新增 projects.db 域表（session_id 主键、updated_at、transcript_json、parked_json、usage_in、usage_out），struct（`SchemaColumns`/`ActiveRecord` derive）落 `sebas-models`，验证：注册表单测 + `cargo test -p sebas-db` 机械断言（叶子无 sebas-*）仍绿
- [x] 1.2 引擎快照写入口：`sebas-dispatch` 引擎为脏会话序列化（turn_log、泊车登记、usage → JSON）并经 `StateHandle` 写 checkpoint 行；泊车进入/解除事件驱动即时 checkpoint，30s 周期任务兜底，内容未变跳过。验证：dispatch 单测（写出行内容与内存态一致；未变不重写）
- [x] 1.3 启动回放：引擎初始化在 session_map 载入后读 checkpoint 表回填三面状态；无 checkpoint 会话保持现状；close 归档时删除对应 checkpoint 行。验证：dispatch 单测（回放后 turns() 输出与 checkpoint 前一致；归档行已删）
- [x] 1.4 进程级 e2e：`tests/testsuite_e2e_test.rs` 新增「强杀 core → 重启 → 转录/usage/待批审批恢复、决定可送达」用例（fake-claude `perm` 泊车中强杀，重启后 Allow 走完回合）。验证：`cargo test --test testsuite_e2e_test -- --ignored` 过
  - 状态（review 阶段补写）：`scenario_projection::parked_approval_transcript_and_usage_survive_a_hard_kill_restart`——hello 完整回合（usage {100,10}）→ 第二轮 `perm` 泊车 → 杀前屏障直读 projects.db 钉「事件驱动 checkpoint 已提交」→ taskkill/进程组强杀（fake-claude 随树同死）→ 按原参数重启 → 断言回放留痕行（restored=1）、转录与 usage 恢复（= 杀前快照行，未上报投影 null）、待批审批同 request_id 恢复呈现且 deny 可决定（200 + core.log `approval answer accepted (acp)` 留痕 + 泊车登记解除）、无伪造 tool_result，末段补「决定后再发消息可继续对话」（新子进程完成新回合）。已知语义如实钉住：泊车中的快照 usage 为「未上报」（每轮开轮 emit_turn_card 重置卡态，首轮已上报的累计值不进快照）——缺口与最小修复建议见 review 报告；spec 场景「照常落 tool_result」在强杀后不可达（子进程已死），按实际语义断言。
- [x] 1.5 沙箱手测复核：QA round9 的 L 场景重跑（taskkill //F 重启），对照 `gui-test-screenshots/qa-round9/l3b_hello_after_restart.png` 的失败形态确认复绿

## 2. 审批应答加固（permission-flow）

- [x] 2.1 core 应答留痕：`InProcessBackend::answer_permission` 成功投递处 `info!(request_id, decision, session_id)`。验证：dispatch/webui 单测或沙箱日志目视（deny 一次，core.log 出现对应行）
- [x] 2.2 前端 answering 超时：review-card `answer()` 以 10s 超时包裹 POST，超时回 `pending` + 可见错误文案「应答超时，请重试」，按钮恢复可点。验证：前端单测（模拟悬挂 fetch → 卡片回 pending 带错误）
- [x] 2.3 沙箱 GUI 复核：并行双卡场景先拒后允各一遍，确认卡片移除、core.log 两条留痕、无 answering 残留

## 3. 项目重命名（project-session-actions）

- [x] 3.1 端点：`POST /api/projects/{id}/rename`（body `{name}`，trim 非空否则 400；更新 projects 表 name 列，per-mutation 落库）；路由注册 + typed rejection。验证：api 单测（成功/空名 400/未知 id 404）
- [x] 3.2 rail 入口：项目操作菜单加「重命名」（弹窗形态对齐会话重命名，Enter 提交 + 空名内联报错），成功后行内即时更新。验证：前端单测 + 沙箱手测截图
- [x] 3.3 重启持久：rename 后重启 core 名字保留、会话归属不变。验证：并入 1.4 e2e 断言或单独 curl + 重启 + `GET /api/projects` 断言

## 4. 工作台打磨（agent-workbench）

- [x] 4.1 `/router` 加入 `RETIRED_REDIRECTS`（router.ts）。验证：router.test.ts 单测 + 浏览器手测地址栏归一
- [x] 4.2 停滞通知文案改「静默超过 {N} 秒即判定停滞，最迟约 {2N} 秒内强制收尾」口径（合成 notice 生成处）。验证：既有 stall 通知断言更新 + 沙箱触发目视
- [x] 4.3 回合帧呈现模型名（帧 model 观察值，缺省不伪造）。验证：前端单测（转录渲染含 model 标识）
- [x] 4.4 设置弹窗 `role="dialog"` + `aria-modal` + 可访问名。验证：a11y 断言（app-shell/设置弹窗测试更新）
- [x] 4.5 会话重命名弹窗 Enter 提交。验证：前端单测
- [x] 4.6 工作台形态持久：项目分组展开态 + 聚焦会话 key 存 localStorage，启动恢复。验证：前端单测（focus 恢复路径）+ 沙箱重启手测

## 5. 文档对齐（无 spec delta）

- [x] 5.1 `AGENTS.md`：修正「graceful exit … dumps state」表述为「session_map 逐变更落库；转录/审批/usage 由 session-transcript-durability checkpoint 承载」，并注明看门狗 1Hz `set_permission_mode` 探针语义（每活会话 1 条/秒，属存活探测）
- [x] 5.2 `sebas-webui/src/api.rs` `CreateSessionRequest.prompt` 注释：补「GUI 创建后自动聚焦会触发激活路径拉起子进程（聚焦即拉起），端点级 0-turn 语义不变」

## 6. 收口验证

- [x] 6.1 全量 `cargo test` 与前端单测过；`invoke testsuite-webui` 既有套件不回归（含 unread-badge、parallel-approval-routing）
- [x] 6.2 `openspec validate fix-webui-qa-round9` 过；验收账本 `tests/acceptance/COVERAGE.md` 回填新增旅程
