# 创建会话模型预选改为「上次选择」

## Why

运营者裁决：不存在"default provider/model"预选之说——新开会话的模型应默认用**上次的选择**，没有则用目录第一个，一个都没有就引导用户去建。现状预选跟随配置的 default provider/model（`/router/api/defaults`），语义不符；且 review 发现 About 分区的 default 行是假勾选死 UI（数据源 `/api/agent-defaults` 已退役为 404，界面上永远显示未设置）。

## What Changes

- **预选三级规则**（创建对话框）：① 上次选择（全局记忆于浏览器 localStorage 的 `(provider, model)` 对，且该对仍在目录中）→ ② 目录第一对 → ③ 目录为空/不可得时显示显式引导，指路 Settings → Models 去配置。
- 记忆只由创建对话框的确认动作写入；会话内模型 chip 的切换（会话域词汇表）不写该记忆。
- **About 分区删除 default provider/model 行**及其跳转 Models 的链接；"default agent kind" 行保留但改读真实数据（现为写死的 `acp` 字面量）。
- Settings → Models 的「设为默认」管理入口与 router 的路由兜底默认**均不受影响**——退役的只是"把该配置当创建预选来源"这一层。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`：「Model selector offers the backend catalog before any session」的预选语义从 configured defaults 改为 last-used → first pair → 引导创建。
- `webui`：「设置弹窗分区与缺省首项」的 About INSTANCE 段删去 default provider/model 行；default agent kind 行改读真实值。

## Impact

- frontend：`new-session-dialog.ts`（预选 + localStorage + 空目录引导）、`model-catalog.ts`（预选 helper 从 defaults 语义改为 last-used 语义）、`settings-modal.ts`（About 行删改）；单测同步。
- 后端零改动（router defaults 载荷、管理入口、路由兜底均不动）。

## Non-goals

- 不动 router 的路由兜底默认与 Models 分区的设默认管理入口。
- 不做每项目/每会话的模型记忆（全局一份）。
- 不动 composer 模型 chip（会话域）。
- 不动 native kernel 的默认模型语义。
