## 1. 审批升级可见化（permission-flow）

- [x] 1.1 `sebas-webui/src/session_backend.rs`：escalate 应答路径感知降级事实，向转录追加系统条目（含操作者原因与「已按仅放行一次降级」语义），对齐 `permission_mode_result` 条目形态；单测钉住「escalate → 条目存在且工具执行有痕」。验证：`cargo test -p sebas-webui session_backend`
- [x] 1.2 前端 `transcript-view` 渲染升级降级系统条目（中性样式，含原因文本）。验证：既有条目渲染单测 + 浏览器复测 B12 场景

## 2. native 转录补全（agent-workbench）

- [x] 2.1 `sebas-webui/src/agent_backend.rs`：`NativeAgentBackend::message` 投递内核前 push `TurnEntry::prompt`（与 ACP seed_card 等价）；单测断言 native 会话详情含 prompt 条目。验证：`cargo test -p sebas-webui agent_backend`
- [x] 2.2 webui 影子队列：native 会话提交即记入会话级 pending 读模型，`session.updated` 相位帧对账推进，会话终结清空；`SessionInfo.pending` 对 native 非空使 `pending-stack` 可渲染（含移除/顺序操作语义与 ACP 对齐的可用子集）。验证：单测覆盖 记入→推进→清空 三相
- [x] 2.3 浏览器级复测：native 会话发消息见用户气泡；回合中追加提交见待执行栈。验证：Playwright 旅程（TESTSUITE_NATIVE=1 装配）

## 3. 滚动跟随与浮标（agent-workbench）

- [x] 3.1 `transcript-view.ts`：未读缝定位完成后恢复 sticky；仅操作者主动上滚脱离跟随；新增「跳到最新」浮标（sticky=false 且有新条目时出现，点击回底并恢复跟随）。验证：组件单测覆盖三态（跟随/脱离/浮标点击）
- [x] 3.2 浏览器级复测：开卷带未读缝的会话后新条目到达 → 浮标出现；点浮标 → 底部 + 后续自动跟随。验证：Playwright 旅程

## 4. 布局与入口（agent-workbench）

- [x] 4.1 连接徽标让位：`app-shell.ts` 工作台头部右端预留徽标宽度，徽标不再绝对定位拦截点击；归档只读视图同策略。验证：组件几何断言 + 浏览器点击聚焦链接/恢复按钮走通
- [x] 4.2 项目行菜单上移/下移后关闭 dropdown，不重锚。验证：`project-rail` 组件单测
- [x] 4.3 `/sessions` 入口：History 组头链接（或等价常驻入口）导航到总览页。验证：Playwright 点击导航断言
- [x] 4.4 key 友好化：纯前端解码工具（渠道 · 本地段标签）接入审批面板与聚焦链接展示位；wire 与路由不变。验证：工具函数单测（`web%00…`、`feishu%00…` 样例）

## 5. 未读与留痕

- [x] 5.1 未读锚收敛：定稿写锚与切会话统一为「已见驱动」单一路径，rail 徽标以服务端 `msg_count` 对账。验证：单测模拟「后台完成→切换→打开」序列，徽标计数不被竞速清零
- [x] 5.2 模型切换留痕：`sebas-dispatch` `apply_model_changed` 与 native override 路径各落系统条目（含新旧模型名）。验证：两执行体各一条单测 + 浏览器复测中程切换
- [x] 5.3 `sebas-agent` 事件时序：终态（Error）落地后再发 summary，零输出判定随之后移；判据表不动。验证：`sebas-agent` 单测（Failed 回合先 error 后 summary）+ test/error 浏览器复测不再出现空回合 notice

## 6. 别名管理 UI（router-model-aliases）

- [x] 6.1 设置弹窗新增「模型别名」分区模块（列表/新建/编辑/删除+确认），接既有 `/api/model-aliases`。验证：Playwright 旅程 CRUD 一轮，刷新后一致

## 7. P3 打磨（agent-workbench / agent-skills）

- [x] 7.1 thinking 增量聚合：同回合相邻思考增量合并为连续段落渲染。验证：test/thinking 浏览器复测展开后为少量连续段
- [x] 7.2 markdown 未闭合围栏容错：围栏作用域限单条目。验证：含未闭合 ``` 的条目渲染单测
- [x] 7.3 操作者取消中性化：停止回合收尾条目改中性「已取消」形态，不用错误色。验证：test/long 中途停止浏览器复测
- [x] 7.4 关于页复制按钮操作反馈（toast/文案变化）。验证：组件单测
- [x] 7.5 技能预览剥离 frontmatter：预览只渲染正文。验证：`agent-skills` 预览单测 + 沙箱 demo-skill 复测
- [x] 7.6 权限模式下拉重开失效修复（wa-dropdown 重建后选项可达）。验证：连续开合下拉的组件单测

## 8. 工具链与清理（无 spec delta）

- [x] 8.1 `tasks.py`：`testsuite-webui-sandbox` 透传 native 姿态（`--native` 或 `TESTSUITE_NATIVE`），端口沿用派生规则。验证：`TESTSUITE_NATIVE=1 invoke testsuite-webui-sandbox` 拉起的 core 含 `SEBAS_AGENT_ROUTER_URL`
- [x] 8.2 删除前端 `client.ts` 中无调用方的 `settings()` 方法（`GET /api/settings`）。验证：`pnpm build` 通过且全仓无引用
- [x] 8.3 回填验收账本 `tests/acceptance/COVERAGE.md` 本轮缺陷→修复证据映射

## 9. 整体回归

- [x] 9.1 `invoke testsuite-webui`（六链 Playwright）全绿；`cargo test --workspace` 无回归
- [ ] 9.2 按第八轮 QA 同款沙箱（TESTSUITE_NATIVE=1）对本 change 涉及的缺陷场景逐条浏览器复测通过
