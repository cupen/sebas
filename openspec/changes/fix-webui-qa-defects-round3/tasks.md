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
- [x] 4.2 深链/刷新直达 `/sessions/…` 时主区项目标题从会话归属项目绑定，不再显示「未选择项目」；验证标准：刷新直达后标题正确（✅ 根因重查定案：`followFocusedProject` 在「detail 先于 projects.list 落地」的深链竞态下以 `path=null` 空转一次即写 `lastFollowedFocusKey` 记账，列表到达后的核对一拍全部早退——标题停在「未选择项目」；上一轮的 detail 侧对账与 `project_id` 字段本身有效，失效路径在记账时序。修复：项目 id 有值而路径未解析不记账，列表到达的 refetch 补投影。单测覆盖「项目列表晚于 detail 到达仍投影」。✅ 主 agent 浏览器回归通过（2026-09-21 沙箱 GUI）：硬刷新直达 `/sessions/web%00…` 后主区标题=归属项目名，快照全树 0 处「未选择项目」；证据 `.openspec/round5-evidence/deeplink-session-title.png`）
- [x] 4.3 About 页 Rust toolchain 空值回退为「未知」；验证标准：About 页无空行值
- [x] 4.4 路径展示统一规范化（注册弹窗填充值与「项目已注册」错误提示）+ Services 页 `sebas run` 内联代码不断行；验证标准：截图对比（✅ 主 agent 浏览器回归：注册弹窗填充值已统一正斜杠）

## 5. 整体验收

- [x] 5.1 `pnpm` 前端单测 + `cargo` 相关测试全绿；验证标准：测试命令输出（634 passed / 28 files；build 成功）
- [x] 5.2 fake-claude 沙箱全链路回归（创建会话、perm 权限三按钮、模式切换、未读徽章、P3 各项），按原复现路径逐项核对；验证标准：验收记录与截图（✅ 主 agent 完成：见 design 附录与 /tmp/sebas-qa-shots/r*.png；4.2 未过项除外）

## 6. 浏览器回归遗留问题（本 change 收口时如实记录，待后续小 change 处理）

- [x] 6.1 未读徽章边界：聚焦会话收到新回复（流式底部跟读场景）仍被标未读——违反 session-unread-badge「read at the bottom never badges」；聚焦行重复点击（同会话 no-op）不触发清零。（✅ 修复落地，四层收敛：① rail 行未读按「聚焦 + 文档可见不呈现」推导（后台 tab 照常计未读），锚的推进权仍归 transcript；② 读锚写入广播 `sebas:anchor-advanced`，rail 监听就地失效重渲染（localStorage 非响应式——此前锚推进后徽标驻留的根因）；③ transcript 快照增长路径与 turn.append 同一贴底语义推进锚（首挂载/换会话装载除外——「开门不是看着」）；④ 空流登记补 dashboard 侧入口（`registerEmptyStreamSession`，0 回合占位渲染的是 dashboard 空态而非 transcript 组件），首聚焦交换的锚建立不再依赖巧合。spec 增量「Focused session arrivals never badge」写入本 change specs/agent-workbench/spec.md，兑现 round4「首聚焦交换永不闪现」。单测覆盖：聚焦+流式到达不出徽章、后台 tab 仍出、重复聚焦清零、锚外推清零不刷新、非聚焦到达仍出徽章。✅ 主 agent 浏览器回归通过（2026-09-21 沙箱 GUI）：聚焦+贴底到达不出徽章（badge-focused-arrival-no-badge.png）；上滚后到达→切走徽章「1」如实出现→点回清零（badge-clears-on-refocus.png）；首访无锚=已读语义复核一致。观察：hidden-tab 期间到达切回无徽章（聚焦+可见即抑制，与 delta 字面一致；「隐藏期到达无任何提示」的 UX 缺口另记 bd 跟进）。复现背景：新建即聚焦会话的首个交换即触发，三个不同 agent 会话均复现；visibilityState=visible 且 hasFocus=true）
- [x] 6.2 composer 模式下拉选项中文长文案仍折行（4.1 修复只覆盖 agent 下拉形态）。（✅ 根因：4.1 的放宽写在 document 级 `wa-overrides.css`，选择器匹配不到 shadow 树里的 wa-select——composer 下拉从未被覆盖，agent 下拉当时目测单行是短文案的假阳性。修复：同款规则（listbox min-width:max-content / 320px 封顶 + option 单行省略）补进 `workbench-composer.ts` 组件样式表——shadow 内样式表才能命中子组件 part。单测以 4.1 同款源码读回钉死。✅ 主 agent 浏览器回归通过（2026-09-21 沙箱 GUI）：模式下拉展开后四选项全部单行无折行，含最长「allow（放行并留审计）」；证据 `.openspec/round5-evidence/composer-mode-dropdown-singleline.png`）
- [x] 6.3 深链/刷新直达 `/sessions/…` 主区标题仍「未选择项目」（4.2 修复未生效，需重查 followFocusedProject 归属解析路径）。（✅ 与 4.2 同一修复：记账时序竞态，见 4.2 行内记录。主 agent 浏览器回归通过，证据同 deeplink-session-title.png）

## 7. review 缺陷收尾（2026-09-20 提交 review 发现，bd 工单同源）

- [x] 7.1 review-card phase reconcile 关死「waiting 无卡」窗口：`sessionPhase` 为 waiting 且合并结果为空时退避重试 reconcile（或在 session.resync 到达时补一次），不再依赖相位值翻转这一单次触发（review-card.ts:245-250，Lit 按值判变导致读模型拉取先于审批落库时 reconcile 不再发生）。验证标准：单测复现「拉取先于落库」时序——waiting 期间后续重试/resync 触发 reconcile 且卡片出现（✅ 双路都做：扑空后退避重试（250ms 翻倍至 4s 封顶，waiting 不结束不放弃，换会话/卸载/出卡即撤）；`session.resync` 到达补一次对账。单测：fake timers 复现「拉取先于落库」——249ms 不动、到点重取出卡、卡后退避停摆；resync 旁路同验）
- [x] 7.2 收敛挂载期三路并发 sessionApprovals GET（connectedCallback:191 + willUpdate:206 + 初次 reconcile:218）为单一入口去重，保留 pullSeq 防陈旧语义。验证标准：单测断言挂载期仅发一次拉取（或等效去重）（✅ 挂载期拉取只走 willUpdate 的 sessionKey 变更一条路（connectedCallback 直拉的响应本就被 pullSeq 代际核对丢弃，纯浪费）；`pullApprovals` 按 sessionKey 共享在途 GET（in-flight map），sessionKey+waiting 同帧预置时三路收敛为一次请求；pullSeq 只在真实 GET 上自增，防陈旧语义不变。单测：waiting 预置挂载恰好一次 GET 且卡片照常出现；普通挂载一次 GET）
