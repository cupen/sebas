## 1. 创建确认首次激活（P1）

- [x] 1.1 在 fake-claude 沙箱复现创建按钮被吞路径，用 hit-target 探测定位根因（wa-select 浮层 overlay 残留假设证实/证伪），把结论记入本 change 的 design 附录；验证标准：根因记录落盘（✅ 主 agent 浏览器验证：overlay 残留假设证伪——吞点击瞬间无任何 active 浮层；根因改判 popover dismiss 吞 click，见 design 附录三次修订记录）
- [x] 1.2 修复创建确认激活链路：首次点击/键盘激活即触发创建（按 1.1 结论选 overlay 清理或 form-submit 备选路径）；验证标准：前端单测覆盖「选过下拉后首次激活即发创建请求」（✅ 第三轮修复 + 主 agent 浏览器回归通过：选 thinker 后首击创建成功、弹窗关闭、会话落库）
- [x] 1.3 实现 dialog 级 in-flight 忙态与防重复提交（入口 guard + 控件 busy 态）；验证标准：单测断言 in-flight 期间二次激活不再发请求
- [x] 1.4 对 wa-select 三个使用点（新建会话 agent 下拉、composer 模式切换、执行节点选择）各跑一次「选择后立即点击其它控件」冒烟，确认 overlay 修复无回归；验证标准：浏览器冒烟记录（✅ 主 agent 浏览器验证：agent 下拉与 composer 模式下拉选择后紧接点击均即时生效；执行节点下拉为静态展示（沙箱仅 local 一项）未单独驱动，选择后交互无异常）

## 2. 权限卡推送即时渲染（P1）

- [x] 2.1 补「PermissionRequest 推送到达即渲染审查卡」的前端单测，固定期望行为后再动结构；验证标准：单测先红后绿
- [x] 2.2 将审查卡渲染收敛为读模型 store 单一入口：WS 推送只更新 store 并触发同一渲染入口，移除与重建并行的推送渲染分支；验证标准：单测覆盖推送渲染与 request_id 合并去重
- [x] 2.3 集成回归：fake-claude 沙箱走「perm → Allow for session 切 auto → 手动切回 ask → 再 perm」路径，审查卡随推送即时出现、无重复卡、已决策不复活；验证标准：沙箱截图与 core 日志记录（✅ 主 agent 浏览器回归通过：卡随推送即时出现两轮、auto 下 perm 直接放行无卡（spec 一致）、无重复卡；reload 后 parked 卡正确重建）

## 3. 非聚焦会话未读徽章（P2）

- [x] 3.1 rail 行渲染改为从 per-browser 锚点推导未读计数与行强调（聚焦会话与流式底部跟读不产生徽章）；验证标准：前端单测覆盖「非聚焦 + 新回复 → 计数呈现、聚焦后清零」
- [x] 3.2 集成回归：聚焦会话 A，经 API 向会话 B 注入消息，rail 出现未读计数；切聚焦 B 后清零；验证标准：沙箱截图记录（✅ 主 agent 浏览器回归通过：非聚焦 thinker 收回复出现「1」徽章，切走再切回清零。⚠️ 遗留两个低危边界问题记入 §6，不在本 change 阻塞）

## 4. P3 瑕疵批量打磨

- [x] 4.1 wa-select 选项面板最小宽度放宽、中文长文案不换行（或单行省略），模式/agent 下拉目测验收；验证标准：截图对比（⚠️ 浏览器回归：agent 下拉 Native Kernel 选项已单行 ✅；composer 模式下拉选项仍折行 ❌——修复面未覆盖到该形态，遗留记入 §6）
- [ ] 4.2 深链/刷新直达 `/sessions/…` 时主区项目标题从会话归属项目绑定，不再显示「未选择项目」；验证标准：刷新直达后标题正确（❌ 主 agent 浏览器回归未通过：深链直达后会话头正常但主区标题仍「未选择项目」，修复未生效，待重派——见 §6）
- [x] 4.3 About 页 Rust toolchain 空值回退为「未知」；验证标准：About 页无空行值
- [x] 4.4 路径展示统一规范化（注册弹窗填充值与「项目已注册」错误提示）+ Services 页 `sebas run` 内联代码不断行；验证标准：截图对比（✅ 主 agent 浏览器回归：注册弹窗填充值已统一正斜杠）

## 5. 整体验收

- [x] 5.1 `pnpm` 前端单测 + `cargo` 相关测试全绿；验证标准：测试命令输出（634 passed / 28 files；build 成功）
- [x] 5.2 fake-claude 沙箱全链路回归（创建会话、perm 权限三按钮、模式切换、未读徽章、P3 各项），按原复现路径逐项核对；验证标准：验收记录与截图（✅ 主 agent 完成：见 design 附录与 /tmp/sebas-qa-shots/r*.png；4.2 未过项除外）

## 6. 浏览器回归遗留问题（本 change 收口时如实记录，待后续小 change 处理）

- [ ] 6.1 未读徽章边界：聚焦会话收到新回复（流式底部跟读场景）仍被标未读——违反 session-unread-badge「read at the bottom never badges」；聚焦行重复点击（同会话 no-op）不触发清零。复现：聚焦 A，API 注入 A → A 行出现徽章。（round5 验收补充证据：新建即聚焦会话的首个交换即触发，三个不同 agent 会话均复现；徽标+「~1 new」分界线持续不消、需手动 mark all seen；复现时 document.visibilityState=visible 且 hasFocus=true，排除后台标签豁免路径；round4 已把「首聚焦交换永不闪现」写入主 spec 但实现未跟进）
- [ ] 6.2 composer 模式下拉选项中文长文案仍折行（4.1 修复只覆盖 agent 下拉形态）。
- [ ] 6.3 深链/刷新直达 `/sessions/…` 主区标题仍「未选择项目」（4.2 修复未生效，需重查 followFocusedProject 归属解析路径）。
