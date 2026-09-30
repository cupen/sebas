# Tasks — webui-i18n-sweep

## 1. 文案清扫

- [x] 1.1 确认框/对话框类：Close 会话、删除 provider/skill、归档、移除项目等全部确认框标题/正文/按钮中文化（口径统一，消除 skill/provider 框之间现状不一致）。
- [x] 1.2 常驻界面类：Settings 分区名/标题/空态、侧栏 PROJECTS、Env Vars 表头、usage 视图、新建会话对话框标题、composer 占位符（两态统一中文）、Appearance 提示。
- [x] 1.3 按钮与提示类：Refresh/Sync/Save/Cancel/Delete/New agent 等按钮、title/tooltips、skills sync 统计行（"已写入 1 · 覆盖 0 · 删除 0 · 私有 0" 风格）与 no placement 文案。
- [x] 1.4 错误呈现类：前端拼装错误去英文前缀；后端消息正文透传不改。

## 2. 原生校验气泡

- [x] 2.1 表单容器 novalidate + 提交时中文自定义校验消息（setup / Settings 各表单 / 新建会话等全部原生校验位）。验证：GUI 手测缺填提交出中文提示。

## 3. 防回归与验收

- [x] 3.1 文案快照单测：关键组件快照 + 裸英文段落抽查断言集合；`pnpm test`（或仓库既有前端测试入口）过。
- [x] 3.2 GUI 对照验收（主 agent 浏览器实测）：侧栏「项目/历史/设置」、新会话对话框「在 <项目> 中新建会话」+ 模式描述、关闭确认框整段中文、/sessions 卡片「聚焦/关闭」、Native Kernel 未配置提示——逐项中文；模型 id/角色值等技术术语保持英文未误翻。套件断言同步 30+ 处以实际渲染核实（含 settings/skills/models/users-admin 定位器）。
- [ ] 3.3 全量回归：前端单测 + 既有 Playwright 套件（断言若含英文文案选择器，随文案同步更新——逐条核对非放松断言）。
