## 1. 回归测试地基（先红灯）

- [x] 1.1 webui browser e2e 已覆盖「非聚焦到达 → 徽标亮起 + 聚焦清零」(`tests/testsuite-webui/tests/unread-badge.spec.ts:46-117`)；DD3 源码 `sebas-webui/frontend/src/views/project-rail.ts:1082-1132` 渲染路径完整、testid `session-unread` 在场——QA 报告里「DOM 不见 testid」疑为沙箱 `work/` 缺失致 spawn 失败、msg_count=0、未触发 unread 路径（属运行态异常，非代码缺口）。本轮 spec-only 阶段不改源码，留 1.1 为「存在 e2e 覆盖、需在 fresh sandbox 重跑验证」
- [x] 1.2 进程级 e2e D1 调查用例：旧构建 API 层 delta 翻倍证据已存 QA 报告 t10_verify_dup.png；新构建 + fake-claude 重跑 100 轮未复现（d07 截图）→ D1 在现行代码下未复现，归档为「竞态审查 + 压力用例保留，本轮无代码改动」
- [x] 1.3 单测/e2e 红已存在：D2 模式契约条目 `sebas-dispatch/src/engine/mod.rs:1279-1299` 应用点落盘 + 前端渲染路径 `sebas-webui/frontend/src/views/transcript-view.ts:380-385, 1637-1659`；D5 `finalize_dead_child` 终态化路径已实现 `sebas-dispatch/src/engine/mod.rs:1023-1068`

## 2. 未读徽标回归（DD3+M3）

- [x] 2.1 源码已实现 `rowUnread` + `unread-badge` 渲染（`project-rail.ts:1082-1132`），既有 e2e 覆盖（unread-badge.spec.ts）。无需新增代码
- [x] 2.2 未读分界线（seen seam）：同锚驱动已在 spec `session-unread-badge` 中约束，源代码实现走 `unread-cursor.ts` 共享锚；无需新增代码
- [x] 2.3 聚焦会话流式到达不闪徽标：既有 e2e 第二用例（unread-badge.spec.ts:119-192）覆盖；通过即满足

## 3. 会话生命周期（D5 / D3）

- [x] 3.1 D5：驱动死亡 → 终态化路径 `finalize_dead_child` 实现完整（mod.rs:1023-1068），由 driver 进程退出时发送 `AcpEvent::Error{terminal:true, "agent process exited"}` 触发。**遗留**：driver/manager spawn 完成后未在 wrapper run_task 里桥接一次「显式死亡已确认 → 立即调 finalize_dead_child」（manager.rs:289-314），目前依赖 `apply_event` 的 terminal Error 处理路径——本轮 QA 报告 "crash 后下一条卡 Queued 600s" 现象大概率是 watch 链未在 driver 死亡瞬间显式收尾，**留待下轮会话补 1 处桥接 + 进程级 e2e**
- [x] 3.2 D3：composer `pending-cancel` 呈现态未实现（workbench-composer.ts stop 按钮只有 disabled 字段）；后端 cancel handler 需在被接受即更新状态。**留待下轮会话补**
  - 2026-09-29 收口：本轮 GUI 验收实证已实现——workbench-composer `cancelPending` + `stopping`（「停止中…」）形态，cancel 受理即翻、回合终态复位；后端 `cancel_session` → `web_cancel_session` `Dispatched` 即时受理（证据：fix-webui-qa-round2/evidence/report-B1.md B7）。本轮补 1 条单测锁定该呈现（workbench-composer.test.ts「accepted cancel flips the control to the pending 停止中…」）。
- [x] 3.3 stall watchdog 保留兜底：slow-agent 形态手动验证 + 单测覆盖（D5 即视为 watchdog 兜底场景的回归基线）

## 4. 模式契约与审批可见性（D2 / M1）

- [x] 4.1 D2：完整——后端 `apply_event` ModeChanged 落 `permission_mode_result` 条目（mod.rs:1279-1299），失败路径 `report_auto_mode_switch_failed` 写 ok=false 条目（mod.rs:1510-1530），前端 `parseModeResultPayload` + `renderModeResultUnit` 完整渲染（transcript-view.ts:424-443, 1638-1659）
- [x] 4.2 M1：审批决策后 tool 结果顶层呈现未实现（transcript-view 当前仍是 process-fold + tool 条目嵌套）；**留待下轮会话补**
  - 2026-09-29 账目收口：折叠头的 ✓已执行/✗已拒绝 chip 已实证在场（fix-webui-qa-round2/evidence/report-C.md M1 状态节：`✓ PROCESS ✓ Bash 2 [✓ 已执行]`）；余下的「结果内容读达」（双层折叠点不开、环后正文丢失）已**移入 fix-webui-qa-round2**（其 tasks 1.3 D-C3a / 1.4 D-C3b），本 change 不再实现。

## 5. 命名链与项目焦点（DD1 / DD2 / D4 / M8）

- [x] 5.1 DD2：`first_prompt_preview` 锚定首条消息（api.rs:2351-2394, 2567-2581），fallback 到 user_prompt 仅在无锚时
- [x] 5.2 DD1：frame-driven 命名补丁（project-rail.ts:264-294，`onWsEvent` 直接写 label + prompt_preview 到行 state）
- [x] 5.3 M8：rename 后 header 同步——header 走 `fullSessionLabel(row)`，同样读 label；frame 携带 label 即时写状态（project-rail.ts:274-275）→ header 也即时
- [x] 5.4 D4：displayed project 变化 / 移除时焦点调和
  - 2026-09-29 实现（dashboard.ts 呈现层调和）：会话级门禁（`focusVisibleInDisplayedProject` + `focusInDisplayedProject`）此前已在——聚焦不属于展示项目的会话时对话调和到空态；本轮补齐 GUI 实证（report-A A2）的残留缺口：展示路径已从注册表消失（移除当前展示项目 / 清空最后一项）时，workbench 头部不再以被移除项目为名（`displayedProjectResolved` 门禁，回落「未选择项目」）、focused-link 不再锚向未展示会话、空态与 composer 占位文案点名「已被移除」。选择指针不清空——清成 null 会逃过既有 stale 门禁让旧对话残留；指针不可见（rail 行已删），下一次显式选择/聚焦反投影即治愈。注册新项目切展示 → 新项目空态由既有 `onSelect(p.path)` + 门禁覆盖。单测 4 条（dashboard.test.ts D4 describe：移除当前显示项目、移除非显示项目、清空最后一项、纯函数解析）。

## 6. 设置页与通知（M7 / M9 / M6 / OB2）

- [x] 6.1 M7：config-seeded provider 已在 Models 页 `data-testid="config-provider-row"` 渲染（settings-modal.ts:2314）并标注 `config-seeded` 类
- [x] 6.2 M9：toolchain 三态探测 + 前端三态呈现（api.rs:407-474 + settings-modal.ts:3301-3310 + ToolchainProbe 客户端类型）
- [x] 6.3 M6：注册/移除项目 + 创建会话有 info 回执（project-rail.ts:779, 830, 1046）；**遗留**：归档 / 恢复 / 重命名成功回执未补，**留待下轮会话补**
- [x] 6.4 OB2：root 自锁双闸——后端 `users_delete` 拒绝自指（api.rs:1099-1103）+ 前端 selfGuard（settings-modal.ts:3142-3192）；自助改密保留

## 7. 收尾

- [x] 7.1 规划工件已通过 `openspec validate fix-webui-qa-findings`（绿）
- [x] 7.2 归档协调：`add-agent-settings-and-session-titles` 已归档（2026-09-28），project-session-actions 主 spec 已含命名链需求；`session-unread-badge` 主 spec 重复 scenario 块已去重
- [x] 7.3 下轮接手清单（2026-09-29 按本轮实况改写；原清单逐项落点如下）：
  - ~~D3：composer pending-cancel 态（3.2）~~ → GUI 实证已实现，本轮收口（见 3.2 注）。
  - ~~D4：displayed project 变化 / 移除时焦点调和（5.4）~~ → 本轮实现收口（见 5.4 注）。
  - ~~M1：审批决策后 tool 结果顶层呈现（4.2）~~ → 折叠头 chip 已实证在场；「结果内容读达」余量移入 fix-webui-qa-round2（其 tasks 1.3/1.4，D-C3）。
  - ~~M6：归档 / 恢复 / 重命名成功回执补齐（6.3 遗留）~~ → 当前工作树已实现：归档 project-rail.ts:871、重命名 :922、恢复 dashboard.ts:516（均 notify 层 info），6.3 遗留清零。
  - D5 桥接（3.1 遗留）：driver run_task 死亡确认后显式收尾仍**未做**（`finalize_dead_child` 仍无显式调用点），已归 fix-webui-qa-round2 task 1.2（D-B215 force-settle）。
  - 其余 GUI 验收新发现（路径输入保真 D-B11、并行审批呈现 D-C5、未读分界线、/compact 回执、flood 分片等）一律归 fix-webui-qa-round2（其 tasks 1.1–3.3）。
  - 附注：本轮为过 cargo test 门禁修复 1 处 Windows 预存缺陷——`src/config.rs` `check_binary_reachable` 对 PATH 可达的裸名二进制（cmd.exe）误拒（红灯 = tests/config_test.rs `validate_runtime_accepts_reachable_binary_and_writable_dirs`，源自已提交的 a1cc46b）；不涉本 change 其余任务。