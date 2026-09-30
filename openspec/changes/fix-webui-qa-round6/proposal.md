## Why

2026-10-01 第四轮 GUI 验收（fake-claude 全链路 + 浏览器黑盒）发现 2 个 P1 功能缺陷、1 个 P1 疑似与 1 个 P2 渲染缺陷：同会话第二轮工具审批请求整体丢失（不出卡、读模型为空、回合永久挂起）；crash 重生后会话行名漂移（既非首条也非最新）；新建 agent 表单在值已同步的情况下保存仍报「agent id 必填」；thinking 条目内容不渲染（只显示占位词）。另有交互稳定性簇与 4 项 P3 打磨点。缺陷集中在核心链路（审批与会话呈现），需在本轮收口。

## What Changes

- 修复工具审批请求丢失：同会话任意轮次的 hook_callback 都必须登记为待批请求并出现在读模型与审批卡（permission-flow）
- 修复会话行名漂移：行名恒取首条 prompt；`first_prompt_preview` 死代码复活或删除回退路径（project-session-actions）
- 修复新建 agent 表单：保存读取到表单实况值；表单可重复打开（agent-settings）
- 修复 thinking 条目渲染：展开的过程组显示实际 thinking 内容而非占位词（agent-workbench）
- 交互稳定性：对话框入场动画期间的点击不再落入遮罩被吞；刷新后交互失效窗口定位并修复或如实豁免（agent-workbench）
- P3 打磨：allow/auto 等价标注；会话创建对话框 agent 预选标注「上次使用」；未发消息会话 rail 行名显「未命名会话」；粘滞 toast 自动消失

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `permission-flow`: 每一轮工具环的审批请求都必须到达审批面（新增场景：多轮审批不丢失）
- `project-session-actions`: 会话行名在 crash 重生后仍取首条 prompt（新增场景）
- `agent-settings`: agent 表单保存读取实况值、表单可重复打开（新增场景）
- `agent-workbench`: thinking 过程组展开显示实际内容；对话框动画期点击不被吞；未命名会话行名；模式等价标注（新增场景）

## Impact

- `sebas-acp`（claude driver 审批注册路径）、`sebas-webui`（行名投影/回退、agent 表单状态绑定、thinking 条目渲染、对话框/交互时序）
- 无 wire 形状变更；permission-flow 既有语义（fail-closed、Cancel 释放、三态决策）不变
- 验收账本：`tests/acceptance/COVERAGE.md` 需回填本轮证据

## Non-goals

- FOUC 首帧空白帧（DOM 完整、自愈，记录不改）
- 非法 model_id 服务端校验（设计如此：校验在真实 agent 侧）
- flood 场景中途取消不可达（1200 段 <50ms 完成，测试器具观察）
- native 内核沙箱不可用（缺 SEBAS_AGENT_PROVIDER_API_KEY，部署事实）
