## Context

QA round14（五簇黑盒验收）的缺陷面集中在 `sebas-webui` 单体之内：前端不呈现后端已有的类型化拒绝、别名/角色可见性等既有能力未被对应消费面接线、以及一批状态一致性 bug。关键既有事实（已核实）：

- `sebas-webui/src/server.rs` 的 `required_permission`：GET 一律不挂权限词，写面挂中央表——viewer 读转录的后端门**已经开着**，卡点在前端「打开会话 = 调 `switch`（写）」的耦合。
- `sebas-domain/src/session.rs` 的 `SessionRejection` 已带中文文案（「会话数已达上限 {limit}」），webui 只是没有把它呈现在创建动作上。
- `role-visibility.ts` 已有 `canCreateSessions`（viewer 除外）；`sessions.ts`（/sessions 总览页）没有消费它。
- `session.updated` 帧已携带 `status_slug`/`pending` 并承诺「每个 FSM 翻转都发帧」（session-unread-badge spec）——行级状态与「等待」标记滞留都是消费侧 bug。
- 编辑表单「以存储值预填全部字段」是 `agent-settings` 既有明文，D-4-4 直接违反它，无需新 spec。

## Goals / Non-Goals

**Goals:**

- 被拒操作可见：创建失败（含容量满）呈现类型化文案，不产生幻影会话 URL。
- 能力接线：别名短名进入两个模型选择面；/sessions 页消费 role-visibility。
- viewer 只读语义自洽：GET 只读视图可读转录，写被明确呈现地拒绝。
- 状态一致性：历史展开态持久、行级生命周期状态、未读徽标归属、等待标记生命周期、计数口径、文案两处、预命名。

**Non-Goals:**

- 不改 dispatch 容量语义/数值、不改 RBAC 中央表、不改 wire 形状（见 proposal Non-goals）。

## Decisions

1. **viewer 打开会话 = 纯 GET 只读视图**（不改后端）。前端对 viewer 角色走「GET 转录渲染、不调 `POST /switch`」的打开路径；聚焦指针本就是写面产物，viewer 无 composer 故无副作用。备选：为 viewer 放开 switch（否——switch 是全局活跃焦点写，spec 明文挂 sessions.write）；viewer 保持列表级不可读（否——使「只读=工作台读面」落空，QA-5 实证语义不自洽）。
2. **类型化拒绝呈现复用 notice 层**：webui 前端在创建提交的非 2xx 响应上取错误 message（`SessionRejection` 的 Display 文案）呈现 notice，且失败路径不 `pushState`。若核实发现错误响应体缺 message，则只在 `sebas-webui/src` 的错误映射处补透传，不动协议。
3. **别名合并策略**：`GET /api/model-aliases` 结果并入 composer 模型菜单与创建弹窗下拉；别名条目带来源徽标区分目录模型；同名冲突时别名条目优先并标注。选中即以别名串为模型值（ACP 经既有 `set_model` 控制帧送达，无协议改动）。别名编辑器的目标 provider 下拉列出「store 行 ∪ config 种子行」——**GUI 复验实测修正**：state 库 aliases 表对 providers 表有外键，「别名只绑定 store provider」是持久层域规则，种子行在 UI 列出但禁选并说明原因（后端 store-only 校验维持原样）；零 provider 时禁用并给原因文案。
4. **/sessions 页按 sessions.write 收口**：新建表单与卡片写操作按钮按 `canCreateSessions` 隐藏；操作被 403 时呈现「无权限」notice；横幅语义拆分——403 类不替换整个列表（区别于读失败的「加载失败」）。
5. **历史展开态并入既有本地持久机制**（`workbench-persist.ts`/split-persist 同一 localStorage 域），新增一个键，缺省收起（向后兼容）。
6. **行级状态单一来源**：rail 行的状态呈现统一改为 `status_slug` 驱动（working/dormant/failed/waiting/…），替换现「活跃绿点 + 最近活跃」里的歧义部分，避免双指示器打架；「等待/1」滞留先定位（force-settle 帧未发 or rail 未消费），按 session-unread-badge 既有语义修消费侧。
7. **预命名**：优先核实 `POST /api/sessions` 是否已接受 title 形参——已支持则创建弹窗直传；不支持则「创建后立即重命名」兜底（零协议改动）。两路径均不改 spec。
8. **陈旧 spec 语料顺带清理**：MODIFIED「RBAC 角色与权限执法」时改写「provider 管理面 SHALL 不纳入角色执法」段——它与 round10 落地的 `provider-management`「Provider and alias mutations are role-gated」直接矛盾（round10 只加了新需求未回改旧文）。校验器不允许场景级删除，故「router BFF 仅登录门」场景**保留名、改写内容**为现行为（仅覆盖读面；写面归 provider-management 角色门），归档语料不再自相矛盾。

## Risks / Trade-offs

- [别名并入改变模型菜单项集合] → 既有浏览器套件中断言菜单内容的用例需随翻新（tasks 列明），GUI 黑盒用例以「别名可选且生效」为准。
- [viewer GET-only 视图与活跃指针语义分叉] → viewer 无 composer、不提交，指针分叉无观察面；spec 场景已钉「后续写仍 403 且有呈现」。
- [行级状态替换绿点可能动 CSS/布局回归] → 状态点沿用现有 badge 样式族，视觉验收走浏览器套件截图对照。
- [等待标记的根因可能在后端 force-settle 未发帧] → tasks 先写定位步骤：journal/日志确认帧序，若确属后端漏帧则属 webui/src 单体修复，不越协议边界。

## Migration Plan

纯单体二进制内改动，无数据迁移；localStorage 新键对旧值缺省安全。回滚 = 还原前端提交即可。

## Open Questions

（无——预命名 API 形状两路径都不改 spec，实现时核实即可。）
