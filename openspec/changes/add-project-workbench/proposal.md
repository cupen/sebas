## Why

WebUI 的组织单位是飞书会话（`chat_id, thread_id`），但实际工作单位是项目。要同时推进多个项目的 agent 对话时，一张扁平会话表无法表达归属。地基其实已经铺了一半却没接上：`Mapping.project_dir` 字段存在（`router/src/state.rs:37`），`RouterHandle::web_spawn(prompt, project_dir)` 存在，但 `webui/src/routes.rs:201` 恒传 `None`；`agent.html` 那套 project-chat 界面既无路由也未注册模板。

## What Changes

- **项目成为一级实体**：注册一个目录（通常是 git 仓库根）为项目，持久化在 webui 自有的 `~/.sebas/projects.json`。**不写 `state.json`**——那是 core 每次 mutation 整文件原子重写的文件，detached webui 去写会互相覆盖。
- **`/` 改为 workbench**：左项目栏 / 中 turn 流 / 下方带内联开关的输入区，右侧会话信息面板。扁平会话看板降级为项目下属的跨项目视图，从主导航移除。
- **会话归属**：`project_dir` 匹配的归入对应项目；飞书来的（`project_dir = None`）进「来自飞书」桶，按 chat 分组。
- **签名元素「你不在的时候」接缝**：turn 流里一道横向标记，划出你上次离开后到达的 turn；有未读时打开项目**默认滚到接缝而非底部**。last-seen 存在浏览器本地，不占服务端状态。
- **输入区就是能用的**：会话创建与发消息经由 `add-core-session-channel` 的会话通道打到 core，detached 与 in-process 两条路径行为一致，同时驱动多个项目的对话只是对同一个 core 开多个会话。唯一的不可用态是「core 未连接」——瞬时、有成因、自动恢复。

**依赖**：样式与实时更新地基来自 `redesign-webui-console`；会话数据与驱动能力来自 `add-core-session-channel`。归档顺序为 console → channel → 本 change。

## Non-goals

- 不改会话通道协议本身——那属于 `add-core-session-channel`。
- 不记录 per-turn 出身（F/W）——需要 router 新增状态；本期只在会话级标出身。
- 不做 preset mode / model 切换的后端：输入区旁只读显示当前 provider/model 并链到 settings。
- 不改飞书侧任何行为，不引入构建步骤、npm 或前端框架。
- 不做项目内的文件浏览、diff 审查、终端。

## Capabilities

### New Capabilities
- `agent-workbench`: 以项目为单位的 agent 工作台——项目注册与持久化、会话归属规则、turn 流与未读接缝、输入区可用性契约、多项目并发的呈现。

### Modified Capabilities
- `webui`: 路由面变更——`/` 由会话概览改为 workbench；新增项目路由；`/sessions` 保留但降级为跨项目视图。

## Impact

- `webui/src/`：新增 projects 模块（注册表读写）+ 路由；会话创建改为经 backend 传入 `project_dir`；`server.rs` 增模板注册。
- `webui/templates/`：新增 workbench 模板；`agent.html` / `agent_timeline.html` 复活重写。
- `webui/static/style.css`：workbench 分区样式。
- 新文件 `~/.sebas/projects.json`（webui 独占）。
- 无新增 Rust 依赖，router 与 feishu 侧不改。
