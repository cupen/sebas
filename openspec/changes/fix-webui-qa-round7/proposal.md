## Why

第七轮 GUI 全量验收（fake-claude 全链路、21 测试点、93 张截图，证据目录 `C:/Users/cupen/AppData/Local/Temp/sebas-qa-shots/`）发现 2 个 P1 缺陷、1 个 P3 缺陷与 3 项 P3 打磨点：超长无空格文本把会话主面板撑到 1.3 万 px 宽、用户消息被推出视口且该会话持续受损；ACP 模型切换被类型化拒绝后回合停滞 600 秒直至 watchdog 强收，解除后又出现虚假「操作者中断」条目；未认证期 WebSocket 反复撞升级端点刷 console 错误。两个 P1 都在核心链路的可用性面上，需本轮收口。

## What Changes

- 修复转录长文本布局破坏：无空格长 token、GFM 表格、meta 行作者名在任意输入下不得撑破主面板、不得把内容推出视口（agent-workbench）
- 修复 ACP `set_config_option` 拒绝路径的回合状态：拒绝即如实终态化收尾，不再等 600 秒 watchdog 强收；`cancelled_turns` 标记按回合标识消费，杜绝虚假「操作者中断」条目（acp-model-selection）
- 修复未认证期 WS 重连噪音：升级被拒且从未打开过的客户端不进入重连急连梯（webui-ws-rpc）
- P3 打磨：权限模式选项标签补中文（Ask/Edit/Allow/Auto → 带中文副标注）、浅色主题 composer 常态边框不得近似错误语义色、转录时间戳在用户/agent 两侧位置统一

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`: 新增「转录任意内容不破坏布局」需求（长 token 换行、表格滚动、meta 行收缩）
- `acp-model-selection`: 修改「Model change via session/set_config_option」——拒绝路径必须终态化回合并如实呈现，不得遗留挂起回合
- `webui-ws-rpc`: 新增「未认证客户端不反复撞升级端点」需求

## Impact

- `sebas-webui/frontend`（transcript-view 样式约束链、ws 重连闸联动鉴权态、mode-vocabulary 文案、composer 边框样式、时间戳渲染位置）
- `sebas-acp` / `sebas-dispatch`（set_config 拒绝的终态回执与回合收尾语义；`cancelled_turns` 消费按回合身份）
- 无 wire 形状变更；`usage.db` 归属与口径不动
- 验收账本 `tests/acceptance/COVERAGE.md` 回填本轮缺陷→修复证据

## Non-goals

- `/goal` 等 agent 自广告命令的英文描述：属 agent 侧内容，UI 不翻译、不改写
- watchdog 600 秒时长本身：本 change 只保证拒绝路径不等 watchdog，不改超时值
- 通用 ACP 驱动 usage 采集与会话级用量展示：归 `add-webui-round7-gaps`
- 触屏/移动端断点适配（验收环境为桌面视口）
