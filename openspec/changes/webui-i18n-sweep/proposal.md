# Proposal — webui-i18n-sweep

## Why

第三轮 GUI QA（证据 `qa-evidence/{w1,w3,w4}/`）全面巡检发现：中文界面下存在成片英文文案混排——Close 会话确认框整段英文、Settings 分区名/标题英文、`PROJECTS` 侧栏标题、Env Vars 表头、composer 占位符（首回合后变 "Ask for follow-up changes…"）、新建会话对话框标题 "New session in work"、provider/skill 删除确认框正文、sync 统计行（"1 written · 0 overwritten · …" / "no placement:"）、按钮与 title 提示等 12+ 处；setup 表单还弹浏览器英文原生气泡（"Please fill out this field."）。工作台主语言为中文（模式菜单、权限卡、通知均已中文化），混排破坏一致性观感，属打磨级缺陷群。

## What Changes

- 全站用户可见文案中文化清扫（zh 为主基准，与现状一致）：确认框、对话框标题、分区名/标题、表头、占位符、按钮、title/tooltips、统计行、空态文案。
- 浏览器原生校验气泡中文化（表单 novalidate + 自定义校验消息，或元素 lang 方案，实现择优）。
- 前端拼装的错误前缀（如 "Error: 权限不足…" 的英文前缀）中文化；后端错误消息正文保持原样原样透传（后端多为中文）。
- 文案快照测试落地：对清扫范围建立快照/断言，防回归。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`：新增「界面文案语言一致性」要求——zh 基准下用户可见文案 SHALL 一致使用中文，列举校验气泡与错误前缀的口径。

## Impact

- 纯前端 `sebas-webui/frontend/src`（views 与组件文案常量）；无后端、无 wire 变更。
- 验证：文案快照单测 + GUI 手测对照第三轮 QA 的 i18n 样本清单逐项转中文。

## Non-goals

- 不引入 i18n 框架或多语言切换机制（Settings→Generic 的语言切换缺省态不在本期；单语言直改文案）。
- 不改后端错误消息语言（服务端文案多为中文，透传保持）。
- 不改品牌名、agent/模型 id、命令名（`/compact`、`toolloop` 等）与技术术语的英文形态（代码级标识符不属文案）。
- 不动分空格/标点等排版规范（维持现状）。
