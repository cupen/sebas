# add-agent-skills Proposal

## Why

sebas 目前没有任何「skill 管理」的面：用户想让 agent backend 用上 skills，只能自己往
`~/.agents/skills` / `~/.claude/skills` 等目录取舍材质，sebas 不参与、不可见、不可
验证。execution-node spec 的 D9 分工（操作者级材料由主控提供、节点拉取）一直没落实
体源——主控侧那份材料放在哪、怎么编，spec 一直空着。现在给它一个最小实体：支持通
用 agentskills 格式的本机仓，带 CLI + webui 可视窗 + 手动投影到各 backend 落点。

## What Changes

- 新增 `sebas skills` CLI 子命令组：`list` / `add <path|url|pkg>` / `remove <name>` /
  `sync`。`add` 是社区方案（git clone / `npx skills add`）的薄封装，把 skill 写进
  本机仓 `~/.agents/skills/`。
- 新增 webui **Settings / Skills** 页：列表、预览（SKILL.md 渲染 + attachments
  浏览）、删除、刷新（重扫仓）、同步（手动触发投影）。**不提供编辑**：增改走社区
  方案（git / npx），webui 是可视窗。
- 新增投影内核 `reconcile`：把仓内容按「方言表」镜像到各 backend 落点目录（首个
  落点是 claude code 的 `~/.claude/skills/`，已验证与 agentskills 同构）。镜像
  语义=名下即镜像（同名覆盖/删除）、名外即私有（backend 里用户手放的不看不导不
  动）；无 manifest、无状态文件。
- 新增 **Settings/Skills 下的 `skills` API 面**（`/api/skills*`，沿用 provider
  管理面先例），webui 页面与 CLI 共用同一 reconcile 内核。
- design 产出一份「backend 方言矩阵」调研结果：claude（同构已验证）、gemini /
  codex / opencode 等的落点与消费格式，标出哪些本期接住、哪些如实 NoPlacement。

## Capabilities

### New Capabilities

- `agent-skills`: 操作者级 skill 仓 + 多 backend 方言投影。覆盖仓的 CRUD（CLI/社区
  方案/删除）、webui 预览面、投影 reconcile 语义（名下镜像/名外私有）、纯手动触发、
  本机边界（远程节点交付不在内）、项目级 skills 不进本面。

### Modified Capabilities

- （无。`agent-driver` 的驱动抽象不变；`execution-node` 的 D9 措辞保持原样——本
  change 填补 D9 的主控侧实体，不改其 requirement 文本。远程节点接入留作后续
  change，届时再动 `execution-node`。）

## Impact

- **代码面**：src/（新 `skills` 模块 + `sebas skills` CLI 接线）、sebas-webui/
  （Settings 新区域 + `/api/skills*` 路由）、sebas-node 不摸。
- **API 面**：新增 `/api/skills*` 系列端点（webui-only，不碰 router provider 面）。
- **依赖**：无新外部 crate 需求预期（reconcile 是目录比对；渲染 SKILL.md 用现有
  markdown 能力）。
- **配置**：config 新增可选 `[skills]` 段（仓路径覆盖，默认 `~/.agents/skills`）。
- **风险**：投影会覆盖 backend 目录下与仓同名的 skill 目录——这是设计内的「仓
  wins」语义，需在 webui/CLI 同步结果里如实报告覆盖了几条。

## Non-goals

- 远程执行节点的 skills 交付（node-link 运输接入）——后续 change。
- webui 上的 skill 编辑器 / 在线新建——增改只走 CLI 与社区方案。
- 监听文件系统自动同步（watch）、后台定时同步——触发永远手动。
- 项目级（某仓库内）skills 的管理——随项目树，不进本面。
- 除 claude 之外 backend 的格式转换器——本期按同构假设埋接口，真差异由方言矩阵
  调研结论决定后续是否立项。
