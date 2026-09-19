## 1. 审批读模型与恢复（permission-flow / agent-workbench）

- [x] 1.1 engine 侧泊车枚举：`SessionBackend` seam 新增 `pending_permission_requests(key)`，返回 `[{request_id, tool, args}]`（数据取自既有 parked 登记）；单测——泊车后枚举可见、批复后消失。验证：`cargo test`（dispatch/webui 单测）
- [x] 1.2 webui 路由 `GET /api/sessions/{key}/approvals` 暴露读模型（鉴权语义随既有路由）；单测——泊车中返回请求体、空时返回 `[]`、未知会话 404。验证：`cargo test`（api.rs 路由级单测）
- [x] 1.3 前端 `review-card` 读模型重建：sessionKey 就绪时拉取一次，与 WS `permission.requested` 按 `request_id` 幂等合并，批复后按 `request_id` 摘除；前端单测——重建渲染、同 id 不重复、推送已决 id 不复活卡片。验证：`pnpm test`（frontend 单测）
- [x] 1.4 批复路由对未命中 `request_id` 返回 typed rejection（404/409），不复活状态；单测覆盖。验证：`cargo test`

## 2. interrupt 全程收尾（permission-flow / agent-workbench）

- [x] 2.1 cancel 释放泊车：`web_cancel_session`（与会话终结路径）清除该会话 parked 集合（fail-closed 释放、幂等）；`turn_engaged` 随 `parked_count=0` 回落。单测——审批挂起时 cancel：pending 清空、`turn_engaged=false`、重复 cancel 无害。验证：`cargo test`（dispatch 引擎级）
- [x] 2.2 停止条目：cancel 命令在 engine 侧打标，`apply_event` 的 `Finished` 分支为被打标回合 append「回合被停止」错误类条目；单测——停止后 transcript 含停止条目、正常完成回合不含。验证：`cargo test`
- [x] 2.3 释放请求的迟到批复被拒：对已释放 `request_id` 的批复返回 typed rejection 且不解除任何状态；单测覆盖。验证：`cargo test`
- [x] 2.4 前端停止控件复位：`turn_engaged=false` 后停止按钮消失、刷新稳定；前端单测——detail `turn_engaged:false` 时不渲染 stop 态。验证：`pnpm test`

## 3. 归档/恢复会话身份（project-session-actions）

- [x] 3.1 `ArchiveEntry` 增可选身份字段（`agent_kind` / `desired_mode` / `current_model` / `available_models`，serde default），归档路由从 `SessionInfo` 填充；单测——归档条目含身份、旧格式条目可反序列化。验证：`cargo test`（archive.rs）
- [x] 3.2 恢复链路带身份：restore 路由 → `web_restore_session` → `Mapping::dormant()` 增身份参数（None 维持现默认）；单测——恢复后 `agent_kind`/model/mode 与归档前一致，旧档字段缺失时回退默认。验证：`cargo test`
- [x] 3.3 前端恢复后头部如实显示身份（含 "default agent" 回退）；前端单测——恢复响应后 agent 显示来源。验证：`pnpm test`

## 4. 聚焦联动与 rail 稳定（agent-workbench）

- [x] 4.1 焦点反投影项目上下文：dashboard 在 rail-focus / 新建落地 / 恢复聚焦路径上把 `active_session.project_id` 投影为 shell `selectedPath`（查项目列表，未命中维持现状）；前端单测——聚焦会话后标题更新、项目行点击语义不变。验证：`pnpm test`
- [x] 4.2 rail 展开态持久化：`sebas.rail-expanded` localStorage（项目路径数组，恢复时忽略已删路径），聚焦会话所在项目无记录时缺省展开；前端单测——刷新保持、聚焦缺省展开、无操作不自行收起。验证：`pnpm test`

## 5. 命名、注册提示与打磨

- [x] 5.1 会话 label：`SessionInfo` 带 `label`，设置/清空路由（复用既有 mutation 风格），rail 行与对话框命名顺序 label → 首条 prompt → 短 id，label 稳定不受首条消息影响；前后端单测。验证：`cargo test` + `pnpm test`
- [x] 5.2 越界/不存在路径的注册禁用原因：Add project 路径框下方显示具体原因（越界 / 不存在），按钮保持禁用；前端单测。验证：`pnpm test`
- [x] 5.3 模型菜单当前项标识 + 权限模式下拉弹层不溢出卡片边界；前端单测（渲染断言）。验证：`pnpm test`
- [x] 5.4 图标本地化：移除 ka-f.fontawesome.com 外链，dashboard 实际用到的图标子集打包进 dist；验证——阻断 CDN 域名后刷新无 403、无破图（Playwright 路由拦截断言）。

## 6. 测试桩保真

- [x] 6.1 fake-claude 最终 thinking assistant 帧补 `signature` 字段（对齐真实 CLI）；wire/单测断言帧含签名。验证：`cargo test`（含桩的进程级用例）
- [x] 6.2 driver 对 `MessageParse` warn 降噪（带类型上下文、同一会话连续失败只告警一次）；单测或日志断言。验证：`cargo test`

## 7. 回归与验收（含上轮遗留债）

- [x] 7.1 Playwright 回归：审批刷新恢复（重建 + 幂等）、停止复位与条目、归档恢复身份、rail 联动与展开持久、label 重命名、越界原因——落 `tests/testsuite-webui/tests/`。验证：`invoke testsuite-webui-server` + Playwright 通过
  - 状态（2026-09-19，review/e2e 阶段）：新增 7 个 spec——`approval-restore.spec.ts`（刷新重建同 request_id + 批复后不复活 + waiting 非 working）、`stop-settle.spec.ts`（stream 停止→「回合被停止」条目 + 控件复位跨刷新；泊车中停止→读模型清空 + 迟到批复 404/卡面 expired）、`rail-expand.spec.ts`（rail 切换/新建落地标题跟随 + 项目行独立 + 展开持久/聚焦缺省展开/无操作不收起）、`session-label.spec.ts`（label 优先/对话框同源/清空回退）、`archive-identity.spec.ts`（fakeacp+ask 身份四项归档→恢复原样带回 + 旧档 default agent 如实回退）、`add-scope-reason.spec.ts`（越界/不存在禁用原因）、`icons-local.spec.ts`（阻断 CDN 零请求 + 本地 icons）。
  - 收口（同日，主 agent）：review 报出的两个阻塞缺陷已修复——① Add project 手填失灵根因 = wa-input 标签行尾游离引号被并进未引号属性值，@input EventPart 降级、listener 永不挂接（历轮 10.3-1 同根因；引号已除，@input 恢复）；② `/icons/{*path}` 路由改挂 `assets::icons_file`（补 `icons/` 前缀查嵌入根），图标资源 200。unread-badge × rail-focus 互斥（上轮 4.2⚠/10.3-2）同轮修复：RAIL_FOCUS_EVENT 携带的目标 key 经 @state `focusOverride` 立即生效，stale focus 窗口内旧 transcript 卸载、其 WS 订阅与「亲眼看着到达」写锚随之失效。主套件全绿（1 例 archive-restore 计时 flaky 由重试吸收，空行名窗口已随投影空串归 None 修复）。
- [x] 7.2 上轮 change 遗留 e2e：`fix-webui-qa-defects` 2.4（归档→恢复→rail 可见、转写完整、History 清空）与 3.3（0-turn 占位闲置 transcript 干净，短阈值）进程级用例。验证：`cargo test --test testsuite_e2e_test -- --ignored`
  - 状态（2026-09-19）：2.4 新增 `archive_restore_rebuilds_row_transcript_and_clears_history`（归档→列表退场→条目带全量快照+身份→恢复→行回原项目+转写完整+History 清空+可写，含 agent_kind 带回）；3.3 的 `idle_placeholder_never_stall_settles_and_stays_writable` 上轮已落地，本阶段复跑通过。
- [ ] 7.3 全套验证：`cargo build`、`cargo test`（workspace）、`pnpm test`、`invoke testsuite-e2e`、`invoke testsuite-acceptance`；沙箱 GUI 复检本清单全部缺陷（上轮 10.2/10.3 一并销账）。验证：命令输出全绿 + 沙箱截图/日志归档到报告
