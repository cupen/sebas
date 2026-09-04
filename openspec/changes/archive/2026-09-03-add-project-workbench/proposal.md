## Why

WebUI 的组织单位是飞书会话（`chat_id, thread_id`），但实际工作单位是项目。要同时推进多个项目的 agent 对话时，一张扁平会话表无法表达归属。地基其实已经铺了一半却没接上：`Mapping.project_dir` 字段存在（`sebas-router/src/state.rs:37`），`RouterHandle::web_spawn(prompt, project_dir)` 存在，但 webui 后端恒传 `None`。

## What Changes（按 SPA 落地形态记录）

- **项目成为一级实体**：注册一个目录（通常是 git 仓库根）为项目，持久化在 webui 自有的 `~/.sebas/projects.json`。**不写 `state.json`**——那是 core 每次 mutation 整文件原子重写的文件，detached webui 去写会互相覆盖。
- **`/` 改为 workbench**：左项目树（会话树 + 项目分组）/ 中 turn 流 / 底部 composer，会话详情 Deep-Link 与扁平列表降级为次级页。扁平会话看板从主导航移除。
- **会话归属**：`project_dir` 匹配的归入对应项目；飞书来的（`project_dir = None`）进 History/inbox 桶。
- **签名元素「你不在的时候」接缝**：turn 流里一道横向标记，划出你上次离开后到达的 turn；有未读时打开会话**默认滚到接缝而非底部**。last-seen 存在浏览器本地（localStorage），不占服务端状态。
- **输入区就是能用的**：会话创建与发消息经 `SessionBackend` 缝打到 core，detached 与 in-process 两条路径行为一致，同时驱动多个项目的对话只是对同一个 core 开多个会话。唯一的不可用态是「core 未连接」——瞬时、有成因、自动恢复。

**依赖**：会话数据与驱动能力来自 `add-core-session-channel` 的 SessionBackend 缝（已在 main 落地）；样式与交互地线继承 SPA 的暗色设计系统。

## Non-goals

- 不改会话通道协议本身——那属于 `add-core-session-channel`。
- 不记录 per-turn 出身（F/W）——需要 router 新增状态；本期只在会话级标出身。
- 不做 preset mode / model 切换的后端：输入区旁只读显示当前 provider/model 并链到 Settings 弹窗。
- 不改飞书侧任何行为。
- 不做项目内的文件浏览、diff 审查、终端。
- **不复活旧 SSR 的模板载体**——本 change 的落地形态是 SPA（Lit 组件 + JSON API），`templates/` / `static/style.css` 已在 SPA 迁移中整体拆除。

## Capabilities

### New Capabilities
- `agent-workbench`: 以项目为单位的 agent 工作台——项目注册与持久化、会话归属规则、turn 流与未读接缝、输入区可用性契约、多项目并发的呈现。

### Modified Capabilities
- `webui`: 路由面变更——`/` 由会话概览改为 workbench；`/sessions` 保留但降级为跨项目视图；设置/网关/About 退役为 Settings 弹窗分区（见 SPA 重设计）。

## Impact

- `sebas-webui/src/`：新增 `projects` 模块（注册表读写、分支 TTL、可达性）；`api.rs` 增项目 JSON API 端点；`server.rs` 的 `WebUiState` 持有 `Arc<dyn SessionBackend>`。
- `sebas-webui/frontend/src/`：新增 `views/project-rail.ts`（项目树会话栏）、`views/workbench-composer.ts`（composer）、`views/transcript-view.ts`（turn 流 + seam）、`views/settings-modal.ts`（Settings 弹窗）；`views/dashboard.ts` 重构为 workbench 满幅布局。
- 新文件 `~/.sebas/projects.json`（webui 独占）。
- 无新增 Rust 依赖，router 与 feishu 侧不改。
