# Design — webui-i18n-sweep

## 清扫清单（第三轮 QA 实测样本，实现时全站再扫一遍兜底）

| 面 | 现状英文样本 |
|---|---|
| Close 会话确认框 | "Closing will terminate the agent child process…" 整段 |
| Settings 分区 | "No general preferences yet. Language switching…"；分区名/标题英文；"No providers configured." / "no default set" |
| 侧栏 | "PROJECTS" 标题 |
| Env Vars 表 | USED FOR 等表头 |
| composer 占位符 | 首回合 "开始对话…"，之后 "Ask for follow-up changes…" |
| 新建会话对话框 | 标题 "New session in work"（按钮已中文） |
| 删除确认框 | "Delete provider …? This removes it from the router configuration."（skill 删除框正文反而已中文——口径统一为中文） |
| skills sync 统计行 | "1 written · 0 overwritten · 0 deleted · 0 private"、"no placement: …"、"2 skills in store" |
| 按钮/提示 | Refresh/Sync/Save/Cancel/Delete/New agent/New (preset)/(custom)；title "Agent is immutable…"、"Fetch model list…"、"native backend needs…" |
| 原生校验气泡 | "Please fill out this field." |
| 错误前缀 | "Error: 权限不足…" 的英文前缀 |
| Appearance | "Your OS currently asks for light…" |

## 决策

- **单语言直改，不上 i18n 框架**：现状基准就是中文，模式菜单/权限卡/通知已中文化；框架是更大的独立投资，本期不做（Non-goal）。被否备选：引入 zh/en 资源表——范围爆炸且无切换需求支撑。
- **原生气泡方案**：表单容器 `novalidate` + 提交时自定义校验消息（中文），或对表单元素设 `lang="zh-CN"` 依赖浏览器本地化——前者可控性强（各浏览器本地化不一），择前者为主、后者兜底。
- **技术术语白名单**：品牌（sebas）、id（agent id、模型 id、`/compact` 等命令）、协议字段名、_drive letter/路径_ 保持英文；「模型 chip 的 default/opus/sonnet/haiku」是模型 id，不改。
- **快照防回归**：对清扫涉及组件建文案快照单测（现仓库已有 `*.test.ts` 快照惯例），断言不出现裸英文段落模式（抽查关键字符串集合）。

## 假设

- 「Language switching…」提示暗示未来多语言——本期不实现切换，仅保证 zh 基准一致；该提示文案本身一并中文化。
- 后端错误消息可能偶有英文（router/上游）——透传不翻译，不属前端文案；如遇高频英文后端消息另行立项。
