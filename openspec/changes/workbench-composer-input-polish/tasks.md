## 1. 前置

- [x] 1.1 归档 `session-slash-commands`（`openspec archive`），其规范同步进 `openspec/specs/`，确认 `session-slash-commands/spec.md` 出现且内容与本 change delta 的锚点一致

## 2. claude 模型选择（acp-model-selection delta）

- [ ] 2.1 `[acp.claude] models` 配置键（缺省无 = 内置别名表 `default/opus/sonnet/haiku`）；config 解析单测（覆盖键缺省/覆盖/空表回退内置）
- [ ] 2.2 驱动 current 观察：session_start / assistant 帧的 model 字段提为共享 `observed_model`；握手成功时拼装 `AcpModelInfo { current: 观察值或 "default", options: 别名表 }` 上报；单测用 fake-claude 帧序断言拼装与覆盖次序
- [ ] 2.3 `AcpCommand::SetModel` 替换拒绝分支为 `client.set_model()`（`"default"` → `None`），成功乐观写 current、后续帧覆盖；单测断言拒绝分支已移除、切换调用发生、观察值纠偏
- [ ] 2.4 前端：模型芯片无选项但 `current_model` 非空时只读展示（替代「无可用模型」占位），`workbench-composer.test.ts` 补只读态断言；浏览器 `models.spec.ts` 补 claude 会话切换旅程

## 3. slash 面板气泡（session-slash-commands delta）

- [ ] 3.1 面板行收敛：移除行内 description 渲染，行高单行化（name + hint，hint 维持 ellipsis）；`workbench-composer.test.ts` 断言行结构变化后两段式补全/过滤/高亮行为不回归
- [ ] 3.2 气泡渲染：`.cmd-palette` 容器级绝对定位浮层（右缘对齐、360×240 上限、内部滚动），内容走 `renderMarkdown()` sanitize 管线，`:hover` 与 `.highlighted` 两态同源触发；单测覆盖「hover 出泡、高亮出泡、Esc/移开收泡、超界滚动」
- [ ] 3.3 a11y 核验：键盘 ↑/↓ 导航时气泡随高亮行移动可达（无指针可用），a11y 门禁套件通过

## 4. 收尾验证

- [ ] 4.1 `cargo test`（sebas-acp + config 相关）全绿；`invoke testsuite-e2e` 全绿（fake-claude 会话模型表随快照可达、切换链路 200）
- [ ] 4.2 `invoke testsuite-acceptance` 全绿；`tests/acceptance/COVERAGE.md` 补两行（claude 模型面、面板气泡）
- [ ] 4.3 沙箱联调：`invoke testsuite-webui-sandbox` 起真实 UI，人工核验面板密度（单行 + 气泡）、claude 会话芯片切换、只读 current 三点；`pnpm vitest`（frontend）全绿
