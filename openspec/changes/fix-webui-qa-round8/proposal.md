## Why

第八轮 GUI 全量验收（三阶段浏览器黑盒，86 个测试点，native+ACP 双执行体，证据在 `D:/workbench/tmp/sebas-qa/`）经源码逐条核查确认 10 项 P2 缺陷与一批 P3 打磨点：审批「升级」在 ACP 面静默降级为放行且原因只进日志；native 会话用户气泡、待执行栈、未读徽标三面缺失；转录 sticky 滚动门控失效；全局连接胶囊遮挡工作台右上角功能；模型切换无留痕；错误回合误报「零输出」。两项直击审批可信度与 native 面可用性，需本轮收口。

## What Changes

- 审批升级可见化：ACP 面 escalate 降级为 allow_once 时必须留痕——转录系统条目说明降级与原因去向，不允许静默放行（permission-flow）
- native 会话转录补全：内核侧补用户消息 prompt 条目；排队提交以 webui 影子队列呈现待执行栈（agent-workbench）
- 转录滚动：sticky 门控修复（开卷定位不再无条件翻 false），新增「跳到最新」浮标（agent-workbench）
- 布局与入口：连接徽标不再遮挡工作台头部/归档恢复按钮；项目菜单移动后关闭；/sessions 总览页补 History 组头链接入口；编码会话 key（`%00`）展示友好化（agent-workbench）
- 未读与留痕：native 后台回合未读锚竞速修复（锚只反映已见内容）；会话内切模型两条执行体统一落转录条目（session-unread-badge / agent-workbench）
- 回合收尾时序：零输出判定移到终态时点，错误回合不再误报空回合（live-turn-stream）
- 模型别名管理 UI：补齐 Settings 分区（CRUD，走既有 `/api/model-aliases`）（router-model-aliases）
- P3 打磨：thinking 流式碎片合并、markdown 未闭合围栏容错、操作者取消中性呈现、复制按钮反馈、技能预览剥离 frontmatter（agent-workbench / agent-skills）
- 工具链：`invoke testsuite-webui-sandbox` 透传 native 姿态；清理无调用方的 `GET /api/settings` client 方法（无 spec delta）

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`: 新增 native 用户气泡、待执行栈呈现、滚动跟随浮标、连接徽标不遮挡、菜单移动关闭、/sessions 入口、模型切换留痕、key 展示友好化、围栏容错、取消中性化各需求
- `permission-flow`: 新增「升级降级必须可见」需求
- `session-unread-badge`: 新增「锚只反映已见内容，切会话不竞速推进」需求
- `live-turn-stream`: 新增「零输出判定在终态时点求值」需求
- `router-model-aliases`: 新增「别名管理必须有 WebUI 入口」需求
- `agent-skills`: 新增「技能预览只渲染正文（剥离 frontmatter）」需求

## Impact

- `sebas-webui/frontend`（transcript-view、app-shell、project-rail、settings-modal、pending-stack、dashboard、sessions 页、client.ts）
- `sebas-webui/src`（agent_backend 的 native prompt 条目与影子队列、session_backend 的 escalate 应答留痕）
- `sebas-dispatch`（apply_model_changed 落转录条目）、`sebas-agent`（summary/error 发射时序）
- 无 wire 形状变更；验收账本 `tests/acceptance/COVERAGE.md` 回填

## Non-goals

- claude 驱动协议级 escalate 支持（agent 侧无等价物，另立项）
- 会话 key 编码格式变更（`\0` 分隔符是 wire 兼容面，本轮只做展示层友好化）
- provider 名称改名（名称即主键、API 无改名语义，disabled 属正确设计）
- admin update/rollback/restart 的 UI 入口恢复（revamp 后移除属有意决策）
- WS 未认证 console 噪音（round7 已收口，本轮实测不复现）
- 触屏断点与全面 i18n（另有 webui-i18n-sweep 承载）
