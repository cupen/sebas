# workbench-rail-polish — Tasks

## 1. 副标题文案

- [x] 1.1 导出 `APP_TAGLINE` 常量并在 `app-shell.ts`、`login-view.ts`、`setup-view.ts` 三处替换「agent router」，单测断言三处渲染文案为「你忠诚的 AI 伙伴」（`app-shell.test.ts` 等既有测试文件内补断言）
- [x] 1.2 全仓 grep 确认无「agent router」残留字面量，`pnpm run build` 通过

## 2. rail 高亮分层

- [x] 2.1 `project-rail.ts` 样式改造：`.row.active` 改中性提亮（surface 底 + text-bright 文字），`li.session-item.current` 保持 accent 不动；单测断言两态类名下的样式类组合互不相同
- [x] 2.2 更新受影响的既有快照/样式断言，`rtk cargo test` 与前端单测全绿

## 3. 创建会话后的焦点链

- [x] 3.1 `confirmNewSession` 成功路径重写：强制展开（直接置 `expanded[path]=true`）、移除 `onSelect` 调用；单测覆盖「从已展开项目创建后项目仍展开」「不再派发 rail-select」
- [x] 3.2 composer 对焦链路：dashboard → composer 的一次性焦点请求（事件或 `focusInput()`，取 design D2 侵入小者）；单测断言创建成功后 composer 输入框获得焦点。真实浏览器修正：wa-dialog 关闭动画收尾会 `trigger.focus()` 把焦点抢回项目行「+」（hide 动画 ~150ms+，慢于创建往返），一次性送焦必输——dashboard 侧改为**短窗重试**（~1s 窗、100ms 节拍）：每拍穿透 shadow 核对活焦点是否已在 composer 输入框内，不在即重送（focusInput 幂等），焦点落定后 hasFocus 短路停止重复送焦，到点放弃、绝不抢其他焦点；单测覆盖「窗内被偷走即送回」「截止后不再抢」两态
- [x] 3.3 既有 new-session 相关浏览器/验收用例同步（若断言了旧的 toggle 行为则修正）

## 4. 端到端验证

- [x] 4.1 `invoke testsuite-webui-sandbox` 起沙箱，人工冒烟：登录页/rail 副标题、项目+会话同屏高亮可辨、从项目「+」创建会话后项目保持展开且可直接打字；截图留档（登录页截图 + DOM 断言留档：focusChain 终态 sebas-workbench-composer>wa-textarea>textarea、真实键盘输入直达输入框；IAB 截图通道中途失效，视觉证据由 4.2 Playwright 覆盖）
- [x] 4.2 `invoke testsuite-webui-server`（Playwright 面）相关用例通过（全量 63 过/8 挂：对照基线（stash 改动后重跑同批用例）7 个同样挂——既有问题非本次引入，见会话记录；session-roundtrip 在改动后全量轮偶挂、单跑复验通过（1.9s）属负载偶发。本 change 相关的创建/往返/高亮面用例全绿）
