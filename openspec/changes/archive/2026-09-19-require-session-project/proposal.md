## Why

会话与项目的关系此前是「可选」的：`POST /api/sessions` 的 `project_id` 可省略（省略即落一个「inbox」会话），`/sessions` 视图的创建表单压根不带项目，而 spawn 失败还会把会话的项目归属整个抹掉（`Mapping::spawn_failed` 重建映射，`project_dir`/`pending_kind` 一并丢失）。三条路径合起来制造出一批**没有归属的会话**：它们不在任何项目下、不在 rail 的任何分组里（Inbox 分组早已下线）、状态只挂在 rail 圆点而它们根本没有行——操作者既看不到也管不了，等于幽灵。

操作者拍板：**会话必须从属于项目**，不存在无项目会话；历史无项目数据可以整片删除；飞书可以例外（聊天发起的会话本就没有项目目录，只在飞书面呈现）。

## What Changes

- **创建面唯一入口 = 项目**：`POST /api/sessions` 的 `project_id` 改为必填——省略 / `null` / 空串 / 未知 id 一律 typed 400（文案点名 `project_id`），绝不静默落一个无项目会话。0-turn 占位走同一道闸门。
- **前端不再有任何无项目创建路径**：`/sessions` 视图的创建表单补必选项目下拉（无项目可选时表单禁用并说明原因）；创建弹窗的 `projectId` 参与确认门禁（无目标项目 = 确认禁用）；`api.createSession` 的 `projectId` 收为必填 `string`。
- **失败不改归属**：`Map::fail_spawn` 就地翻状态，**不重建映射**——`project_dir` / `pending_kind` / `pending_model` / `pending_mode` / `desired_mode` 全部保留。spawn 失败的会话照常出现在它的项目行下，圆点如实读作 `failed`（raw status 仍是 `spawn-failed`），不再从项目里消失。
- **持久化清退（历史数据可删）**：`SessionMap` 的 dump/restore 双向丢弃**非飞书**且无 `project_dir` 的映射（带 warn 留痕）；不迁移、不猜归属。飞书通道例外。内部归档记录键（`closed-*`，acp-session-mapping D4 的「原映射保留在存储」）不受该约束，且归档时连身份一起保留。
- **spec 文本更正**：删掉「无项目会话不进 rail」这类以「无项目会话存在」为前提的表述，改为「会话必须从属于项目，飞书是唯一例外」。

## Capabilities

### Modified Capabilities

- `agent-workbench`：（新增）「Sessions belong to a project」——webui 建立/呈现的每个会话都必须归属于一个已注册项目；spawn 失败不改变归属；（修改）「History group is the archive」——去掉 inbox 例外，只留飞书这一条。
- `feishu-option`：「双通道共享会话状态」——明确飞书会话是**唯一**允许无项目目录的通道（它们只在飞书面呈现），不是「无项目会话」的通例。

## Impact

- 后端：`sebas-webui/src/api.rs`（创建闸门）、`sebas-dispatch/src/state.rs`（`fail_spawn` 保留身份、dump/restore 清退判据、`preserve_closed_mapping` 带身份）、`src/session_boot.rs`（归档调用点）。
- 前端：`sebas-webui/frontend/src/views/sessions.ts`（必选项目）、`views/new-session-dialog.ts`（确认门禁）、`api/client.ts`（`projectId` 必填）。
- 测试：`sebas-webui/tests/*`（创建用例先注册项目；新增「无项目 400」不变量用例）、`sebas-dispatch`（`fail_spawn` 保留身份回归）、`tests/support`（`scene_project_id` 助手）、`testsuite_e2e_test` / `testsuite_acceptance_test`（创建统一注入项目）、`testsuite-webui`（旅程回到 rail 圆点强断言）。

## Non-goals

- **不做存量无项目会话的迁移**：操作者明确「历史数据可以全部删除」，故直接丢弃，不做归属猜测，也不写一次性搬迁脚本。
- **不改飞书侧**：飞书会话继续无项目目录、继续不进 rail（它们本就只在飞书面呈现）。
- **不碰远端节点语义**：节点会话的 `RemoteSessionView.project_dir` 仍是 `Option`（冻结 wire 契约），本 change 只约束主控侧经 WebUI 建立的会话。
- **不新增 rail 分组**：不再引入 Inbox 分组——无项目会话这个概念本身被取消。
