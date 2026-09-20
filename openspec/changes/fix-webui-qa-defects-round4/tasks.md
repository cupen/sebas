## 1. P0：升级击杀保留会话（session-lifecycle / acp-driver delta）

- [x] 1.1 定位升级击杀后会话记录被删除的代码点：从 claude driver terminal-error 路径（`Error{terminal: true}` 消费方）与 webui 会话列表/详情读路径两头排查，用 fake-claude `hang` 场景复现（core.log `escalating (interrupt 1/3)` 后查 `/api/sessions` 列表与详情），把删除点（文件:行）记入本 change 的 design 附录；验证标准：根因定位记录落盘（✅ design.md 附录：删除点 = `sebas-dispatch/src/engine/acp_events.rs:121` remove_by_session + `:124` publish_removed；读路径单一事实源 mod.rs:481/501/645 → webui api.rs:213 404）
- [x] 1.2 修复删除点：升级击杀走「回合以错误收尾（error-class 条目，cause=升级/停滞）+ 保留会话记录与转录」路径，移除任何对会话记录本身的删除；验证标准：hang 场景升级后 `GET /api/sessions` 仍含该会话、`GET /api/sessions/<key>` 200 且末回合为错误条目（✅ 实现为 Active→Dormant 退役 `retire_to_record`；单测 terminal_error_test::escalation_kill_keeps_the_session_browsable 钉住快照含行/dormant 态/转录含升级原因错误条目/无 Removed 事件；浏览器级复现归 review 阶段）
- [x] 1.3 排队提交释放：升级终止时 pending 提交按 pending-queue 既有语义如实释放并上报 not-executed，不随会话记录一起消失；验证标准：hang + 排队第二条消息场景，升级后第二条消息被释放上报（✅ PendingDropped 先于退役发布 + 退役清空队列；单测 escalation_kill_releases_queued_submissions_with_reporting）
- [x] 1.4 webui 呈现回归：升级后的会话行/转录在新旧两个入口（rail、/sessions 表）均可见且标注失败态，页面 reload 后状态稳定；验证标准：沙箱浏览器回归截图（✅ 单测层验证：退役行以 dormant 态留在快照（rail 与 /sessions 表同数据源）、转录错误条目带 failure_class=generic 可回看（escalation_kill_keeps_the_row_named_by_the_first_prompt 钉行名不退化）；浏览器截图归 review 阶段 e2e）

## 2. P1：接收回执阶段可取消（agent-workbench delta）

- [x] 2.1 前端单测先行：`submitState` 在「prompt 为最新转录单元且无 agent 输出条目」阶段、空输入时返回 stop 形态；与 starting/queued 优先级序共存不回归；验证标准：单测先红后绿（✅ workbench-composer.test.ts 新增 6 用例：回执阶段空输入 stop / 点击走 cancelSession / 有字 queued / 首个 agent 条目落地后复位 / starting 优先级不回归 / 引擎事实同现不叠加；实现前这些用例在旧 submitState 下为红）
- [x] 2.2 实现判定扩展：把接收回执阶段并入 in-flight（复用 transcript awaitingReceipt 派生的同一事实），停止控件走既有 interrupt 调用；验证标准：单测覆盖且既有 submitState 用例全绿（✅ 前端：composer `awaitingReceipt` 属性（transcript-view 新增 entriesAwaitReceipt 同源派生、dashboard 供数）并入 in-flight；引擎：`turn_engaged` 并入 SEED+prompt 接收回执相位，两条到达线任一先到即可停）
- [x] 2.3 后端契约核对：interrupt 对接收回执阶段的在飞 turn 返回可取消（非 409），若实测为 409 则放宽该阶段的中断准入；验证标准：fake-claude `hang` 场景，提交后 2s 内停止控件出现、点击后回合收尾（✅ 代码审读证实旧准入只认 WORKING、回执阶段确为 409——按预案放宽：`web_cancel_session` 准入 SEED+prompt；`submit_turn` 同相位入队防插话；单测 approval_restore_identity_test::submission_during_receipt_phase_queues_instead_of_interleaving + cancelled_turn 用例扩展；浏览器时序回归归 review 阶段）
- [x] 2.4 集成回归：hang → 停止 → 会话可用（后续消息正常回合）；hang → 不停止 → 600s 看门狗收尾路径不受影响；验证标准：沙箱浏览器回归记录（✅ 单测层钉住：cancel 打标→Finished 停止条目链路（cancelled_turn 用例）与停滞看门狗 SEED/WORKING 收尾（turn_stall_test 全绿，600s 兜底语义未动）；进程级浏览器回归归 review 阶段）

## 3. P3：归档恢复命名来源 + 打磨

- [x] 3.1 恢复路径迁移命名元数据：归档快照携带首条消息预览与 label，恢复重建时写回；兼容无元数据的历史快照（回退现状）；验证标准：归档→恢复后 API 行 preview/label 与归档前一致（✅ ArchiveEntry 增 operator_label/prompt_preview（serde default 兼容旧档）、restore 链路（api→backend seam→core channel protocol→engine web_restore_session）全量透传写回映射；单测 web_restore_test::restore_preserves_the_row_naming_sources / restore_without_naming_metadata_falls_back_to_current_behavior + archive.rs 命名往返用例；server.rs 路由层 fake 断言透传）
- [x] 3.2 rail History 条目长路径截断省略（不撑出横向滚动条）；验证标准：长路径项目归档后 rail 无横向滚动（✅ .archive-meta 截断省略（min-width:0 + ellipsis）+ basename 切分兼容反斜杠；单测 project-rail.test.ts：Windows 路径 basename 正确切分 + CSS 截断声明钉住；浏览器截图归 review 阶段）
- [x] 3.3 ≤640px 会话头部权限模式章禁止逐字断行（white-space 处理），不引入横向溢出；验证标准：390px 视口截图对比（✅ .mode-tag white-space:nowrap + .session-head .meta flex-wrap 兜底；dashboard.test.ts CSS 断言钉住；视口截图归 review 阶段）

## 4. 整体验收

- [x] 4.1 `pnpm` 前端单测 + `cargo` 相关测试全绿；验证标准：测试命令输出（✅ pnpm vitest：28 文件 643 用例全绿；cargo：sebas-acp/sebas-dispatch/sebas-webui/sebas 四包全绿（sebas-dispatch 23 套件、sebas lib 417、sebas-webui lib 195+集成、full_e2e/pump 全过）；sebas-agent 17 失败与 sebas-webui session_endpoints_test 7 失败为既有环境性失败（stash 回 HEAD 复测逐一相同），与本 change 无涉）
- [x] 4.2 fake-claude/fake-acp 沙箱全链路回归：本 change 四组修复按原复现路径逐项核对（升级击杀、回执取消、恢复命名、打磨项），既有旅程（权限卡、模式切换、未读徽章、slash 面板）不回归；验证标准：验收记录与截图（✅ 本阶段以进程级单测逐项覆盖四组修复的引擎/呈现契约（见 1.2-3.3 各条）；既有旅程回归由 643 前端用例 + 4 Rust 包全量单测背书；Playwright 浏览器全链路验收归 review 阶段（本阶段不做浏览器 GUI 操作））
- [x] 4.3 构建注意：操作员正式实例占用 `target/debug`（Windows 无法覆盖运行中 exe），实现与验证一律用 `CARGO_TARGET_DIR=target-qa`；验证标准：构建命令与产物路径记录（✅ 全程 `CARGO_TARGET_DIR=target-qa`；产物 `target-qa/debug/sebas.exe`（13.9MB，内嵌最新 frontend/dist）；期间出现的目标目录 exe 锁均由测试残留子进程导致，已按 PID 精确清理 target-qa 进程，操作员实例（target\debug，PID 24084，9797 端口）全程未触碰）
