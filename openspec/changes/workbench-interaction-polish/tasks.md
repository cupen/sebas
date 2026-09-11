# 任务：工作台交互与布局优化

## 1. 后端 cancel 链路

- [ ] 1.1 core channel 服务端与客户端新增 `session.cancel` 方法：core 侧路由到会话既有取消机制（interrupt），未知 key / idle 会话返回 typed 拒绝；带单元测试验证两条 typed 拒绝路径（`cargo test -p sebas-webui core_channel`）
- [ ] 1.2 webui BFF 新增 `POST /api/sessions/{key}/cancel`（POST-only、鉴权与 message 同姿态），core 不可达时 503；`cargo test` 覆盖 503 与转发成功两例
- [ ] 1.3 前端 `api/client.ts` 新增 `cancelSession(key)`，带 client 单测（`pnpm test`）

## 2. 前端模型目录复用

- [ ] 2.1 把 composer 里的目录加载逻辑抽为 `model-catalog.ts` 的 `loadModelCatalog()`（providers+defaults 并取、toModelCatalog、default 预选规则），composer 改为调用它；现有 composer 单测保持绿（`pnpm test`）

## 3. rail 创建对话框

- [ ] 3.1 新建 `sebas-new-session-dialog`（wa-dialog：agent 必选下拉 + 两级模型选择 + 取消/确认），agent 预选项目 default_agent、模型预选目录 default；无 agent 时确认禁用；组件单测覆盖禁用/预选/取消不创建（`pnpm test`）
- [ ] 3.2 project-rail 接入：项目行与 Inbox 组的 `+` 改为打开对话框，确认后调 `POST /api/sessions`（0-turn 占位）并沿用 create 后聚焦刷新链路；rail 单测更新（`pnpm test`）

## 4. composer 纯跟随化 + 状态机

- [ ] 4.1 composer 移除创建模式：`createRequested`、agent select、provider select、`+ new session` chip、`settings-link`、无聚焦时的创建渲染全部删除；无聚焦时由 dashboard 空态承接提示；composer 单测更新并新增「工具条无创建/设置入口」断言（`pnpm test`）
- [ ] 4.2 模型 chip：底部右侧单 chip + 两级分组菜单（目录交叉引用分组、未匹配归"会话提供"组、目录不可得退化平铺、当前模型打勾、无 available_models 显式提示）；单测覆盖分组与降级（`pnpm test`）
- [ ] 4.3 发送状态机：dashboard 传 `turnInFlight`；composer 按优先级渲染 disabled/send/转圈/停止方块/排队形态，停止点击调 `cancelSession`、错误走 callout、turn 结束自动复位；单测逐态断言（`pnpm test`）

## 5. 布局分割与视觉

- [ ] 5.1 app-shell 侧栏|主区换 `wa-split-panel`（180–480px clamp、localStorage 读写 try/catch、<640px 退化）；单测用假 storage 断言持久化与恢复（`pnpm test`）
- [ ] 5.2 dashboard 会话流|输入框加垂直 `wa-split-panel`（composer 最低 120px、最高主区一半、同款持久化）；单测同上（`pnpm test`）
- [ ] 5.3 浮岛视觉：tokens.css 新增 canvas token，app-shell 背景、三区域圆角浮岛、去通高硬线与 `hr.divider`、splitter rest 态透明 hover 亮把手；遵守 reduced-motion（`pnpm build` 通过）

## 6. 联调与验收

- [ ] 6.1 沙箱 e2e：`invoke testsuite-e2e` 增补 cancel 路由用例（fake-claude 下 typed 拒绝路径 + 503 路径；真实中断路径如实标注受 stub 限制），全部通过
- [ ] 6.2 沙箱截图验收：`invoke testsuite-webui-sandbox` 起沙箱，走「+ 创建会话对话框 → 发送 → 流式停止态 → 拖拽两道分割线 → 刷新恢复尺寸 → 浮岛视觉」全旅程截图确认，浮动细节按截图迭代
- [ ] 6.3 acceptance 套件增补创建对话框旅程用例并通过：`invoke testsuite-acceptance`
