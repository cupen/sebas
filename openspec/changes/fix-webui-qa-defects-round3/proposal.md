## Why

第三轮 WebUI 全链路黑盒 GUI 验收（browser-use 驱动真实浏览器 + fake-claude 沙箱，覆盖项目注册 /
会话创建 / 消息收发 / thinking / tool use / 权限卡三按钮 / 模式切换 / slash 命令 / 流式 / 崩溃 /
Settings，截图证据在 `/tmp/sebas-qa-shots/`）发现 3 个确认缺陷：2 个 P1 交互失效（新建会话
首次点击被吞、权限等待态下权限卡不渲染）与 1 个 P2 功能缺失（非聚焦会话新回复无未读徽章）。
其中权限卡不渲染会让操作员面对「等待」状态却无处决策，会话永久卡死，只能靠刷新自救。

## What Changes

- **新建会话弹窗：创建确认对首次激活立即生效**——操作 Agent 下拉（wa-select）后，
  第一次点击「创建会话」（或键盘激活）必须触发创建；不再需要点第二次。创建期间给出
  忙态指示，防止重复提交。
- **权限卡推送即时渲染（实现修复，无 spec 变化）**——权限请求到达时审查卡必须随推送
  立即出现；本轮实测出现「rail 显示等待 + Stop 亮红但无卡可决策」的空悬态（reload
  重建路径正常）。该行为已由 `agent-workbench`「Parked remote approvals surface in
  the workbench」明确要求，本次是实现回归修复。
- **非聚焦会话未读徽章（实现修复，无 spec 变化）**——会话不在焦点时收到新回复，rail
  行显示高对比未读计数并强调行样式。该行为已由 `session-unread-badge`「Unread badge
  on session rows」明确要求（WS 推送已到达——行名实时翻转正常，缺的是徽章呈现链路）。
- **P3 瑕疵批量打磨**：wa-select 选项中文长文案换行（模式/agent 下拉宽度）；深链或刷新
  直达 `/sessions/…` 时主区项目标题误显「未选择项目」；About 页 Rust toolchain 行空值；
  路径显示 `\` 与 `/` 混用（注册弹窗填充值与「项目已注册」错误提示）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`: 新增「Creation confirmation activates immediately」需求——创建
  确认控件（点击或键盘激活）必须在首次激活时生效，与此前是否操作过下拉无关，创建
  进行中呈现忙态且不重复提交。权限卡与未读徽章两缺陷为既有需求的实现回归，不产生
  spec 增量。

## Impact

- `sebas-webui/frontend/src/views/new-session-dialog.ts`（创建按钮激活链路 + 忙态）
- `sebas-webui/frontend/src/views/workbench-composer.ts` / `transcript-view.ts`
  （权限卡推送渲染与等待原因说明）
- `sebas-webui/frontend/src/views/unread-cursor.ts` / `project-rail.ts`（未读徽章呈现）
- wa-select 宽度样式、app-shell 项目标题绑定、About 构建信息、路径规范化显示（P3）
- 不涉及后端/Rust 代码；不改变 wire 协议。

## Non-goals

- 不修复 Stop/interrupt 无法在 fake-claude 流场景中验证的问题（桩在场景运行中不消费
  stdin interrupt，属测试基建限制，非产品缺陷）。
- 不覆盖本轮未测区域（login/RBAC、归档恢复、pending 队列、移动端断点、WS 断连重连、
  provider 编辑器深流程）——留待下一轮 QA。
- 不调整权限模式语义（ask/edit/allow/auto 的门控行为本轮验证符合既有 spec）。
