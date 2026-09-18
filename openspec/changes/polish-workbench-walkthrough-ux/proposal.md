# Proposal: polish-workbench-walkthrough-ux

## Why

一次针对 webui 工作台的沙箱 GUI 全功能走查（项目注册 → 建会话 → 对话往返 → 模型/模式切换 → 归档/移除 → Settings）暴露出一批体验与安全问题：点击 History 条目会**无确认地立即恢复**归档会话且零反馈（走查中因此误写真实 `archive.json`）；归档文件默认路径不受沙箱 env 钉定约束，已两次污染真实 `~/.sebas`；聚焦中的会话仍累积「~2 new」未读标记；模式/模型在创建弹窗与会话面板两处口径互相矛盾。这些问题在同一条使用路径上高频可复现，应一次性收口。

## What Changes

- **History 点击语义重塑（需求修订）**：点击归档条目改为打开**只读归档视图**（现状 spec 规定点击即恢复+激活，属误触陷阱）；「恢复」改为归档视图内的显式按钮并带确认弹窗；恢复/失败必须有 toast 反馈；恢复到未注册项目的会话不得静默隐身。
- **归档文件路径收敛**：`archive.json` 默认路径从 `$HOME/.sebas/` 改为跟随 state DB 所在目录（`SEBAS_ARCHIVE_PATH` 仍最高优先，旧路径存在时一次性迁移）；AGENTS.md 沙箱配方把 `SEBAS_ARCHIVE_PATH` 列入必钉 env 清单。
- **聚焦会话不再累积未读**：会话聚焦且页面可见时到达的回合直接推进 seen boundary，「~N new since you last viewed」只对真正离场期间到达的回合出现。
- **模式/模型 UI 一致性**：composer 权限模式下拉补 label 与默认态文案（不再空白）、选项措辞与创建弹窗统一（带中文解释）；`UNGATED`/`UNKNOWN` 徽章改为中性中文措辞并与 `mode auto` 章去重；Agent 下拉不外露 `SEBAS_AGENT_PROVIDER_API_KEY` 等内部 env 名；「尚未配置任何模型」警告与 agent 内置模型目录的真实可用性对齐。
- **文案与弹窗卫生**：用户消息 `you`/`你` 双语混排统一；0-turn composer 占位符语境化；关闭的弹窗移出可访问性树；Settings 弹窗消除横向滚动条；rail 占位会话行按既有 spec 复测 short-id 回退显示。
- **创建会话失败不再静默**（第二轮走查）：项目已注册但未选中时，「创建会话」点击后无请求、无错误、无反馈——失败必须以 inline 提示或 toast 呈现。
- **slash 命令面板可见性**（第二轮走查）：面板 DOM 与数据均就绪，但被会话面板遮挡至仅剩约 2px 缝——实现对既有 `session-slash-commands` spec「SHALL render a command palette above the input」的合规修复（属实现缺口，不改 spec）。
- **崩溃会话三面一致**（第二轮走查）：子进程崩溃后后端直接删除会话，而聚焦视图继续显示「Working」幽灵并保留停止按钮——会话终止（含崩溃）必须同步 rail、聚焦视图与后端三处，并给出崩溃通知。

## Capabilities

### Modified Capabilities

- `project-session-actions` — 「Session archive」需求修订：History 点击行为从「恢复+激活」改为「只读查看」，恢复走显式确认 + toast 反馈；补「原项目未注册时恢复的落点反馈」场景。
- `webui` — 「Archive persistence」需求修订：归档文件默认路径跟随 state DB 目录，env 覆盖与旧路径迁移语义；新增「聚焦会话终止一致性」需求：会话被后端移除（崩溃等）时聚焦视图同步退出并通知。
- `agent-workbench` — 「Unseen-turn seam」「Execution-body availability is stated, not discovered」「Model selector offers the backend catalog before any session」需求修订：聚焦可见即推进已读边界；执行体不可用态的用户措辞不含内部 env 名；模型警告只反映 agent 目录真实为空。新增「创建失败必须可见」需求。
- `session-unread-badge` — 「Unread badge on session rows」需求修订：聚焦可见期间到达的消息不计入未读计数。

## Impact

- **代码**：`sebas-webui/frontend/src/views/`（rail、workbench 会话视图、composer、settings 弹窗）、`sebas-webui/src/archive.rs`（默认路径与迁移）、`sebas-webui/src/api.rs`（restore 反馈语义不变，行为不变）。
- **文档**：`AGENTS.md` 沙箱必钉 env 清单补 `SEBAS_ARCHIVE_PATH`（防御纵深）。
- **测试**：`testsuite-webui-browser`（tests/）补归档视图/恢复确认/未读聚焦三组用例；既有 archive 相关测试随路径迁移更新。
- **无 breaking API 变更**：wire 协议与 HTTP 路由面不动；归档文件路径变化对单机用户由自动迁移吸收。
