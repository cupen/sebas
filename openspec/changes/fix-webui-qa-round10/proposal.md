## Why

第十轮 GUI 全量验收（A/B/C 三簇、37 个测试点、证据在 `C:\Users\cupen\AppData\Local\Temp\sebas-qa-r10\reports\`）确认 9 个缺陷：2 个 P1——GUI 创建的 agent 因可达性探测误报被完全禁用（无法建会话，连带 error 场景无法验证）、转录直播态整面空白绘制；2 个 P2——历史消息的模型徽章随当前模型回溯改写（历史失真）、member 角色可变更 provider（凭据面越权，服务端放行）；5 个 P3 文案/徽章/布局缺陷。主链路（登录、项目、会话、审批、权限四档、thinking、流式、异常、归档、RBAC 隔离、主题）全部验证可用。

## What Changes

- Agent 可达性探测修复：GUI 创建（store 行）的 agent 填真实存在的 Windows 绝对路径（正/反斜杠）被误报 `command not found`，新建会话下拉同步禁用——探测的绝对路径判定被 store 行的 command 组装绕过（`agent_kinds.rs` 的 `resolved_binary` 语义本身正确，嫌疑在 server 组装 store 行 source 处）；修复后免重启建 agent 即可用
- 不可用指引分流：agent 不可用文案按 cause 分流（二进制缺失 → 「设置 → Agent」；模型凭据缺失 → 「设置 → 模型」），不再一律指向模型页
- 新建 agent 表单 dup-id 提示一致：删除「保存将覆盖」黄条与「已存在」红条的语义矛盾，只保留与实际行为（拒绝保存）一致的提示
- 转录直播态渲染修复：回合进行中转录面板整面空白（DOM/aria 完好、console 零错误、reload 恢复，超宽表格回合后连续回合必现）——渲染层修复并附复现测试；同时 transcript 容器不再被超宽内容撑破（实测撑到 5350px 隐藏溢出），表格/代码块滚动区补可见滚动条
- 模型徽章保真：历史消息的模型徽章按回合当时的观察模型渲染，不从会话当前模型回填
- bad-model 类型化拒绝链路：调查 composer 选模型是否真的下发 `set_config_option`，修复断点侧（前端接线或驱动投影），fakeacp 桩（`--reject-model bad-model`）保持测试 oracle
- Provider 变更角色门禁：provider/模型别名的增删改与默认选择限 root/admin（`settings.manage`，rbac.rs 矩阵不变只接线），member/viewer 只读；服务端路由守卫强制 + 前端 role-visibility 隐藏
- 侧栏「历史」徽章接线：接会话页同源统计（当前恒 0），随会话创建/归档联动

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-settings`: store 行 agent 的可达性探测语义（绝对路径直接判定）、不可用文案按 cause 分流、dup-id 表单提示一致
- `agent-workbench`: 侧栏历史徽章接真源、转录直播态不出现整面空白、超宽内容不撑破容器且滚动区可见、模型徽章按回合观察值保真
- `acp-model-selection`: composer 模型选择必须真实下发 `set_config_option` 并呈现桩的类型化拒绝（bad-model）
- `provider-management`: provider/别名的变更面角色门禁（root/admin 可写、member/viewer 只读，服务端强制）

## Impact

- `sebas-webui/src/agent_kinds.rs` 及 store 行 → `AgentKindSource` 组装处（server.rs）、`sebas-webui/src/routes.rs`（provider/别名变更路由挂守卫）
- `sebas-webui/frontend/src/views/`（transcript-view.ts、dashboard.ts 侧栏、new-session-dialog.ts 文案分流、settings-modal.ts agent 表单、role-visibility.ts）与 composer 模型菜单接线
- `tests/`（探测单测、provider 守卫 API 测试、转录渲染回归）；`AGENTS.md` 如 O-B-01 核实为桩预期则补注
- 无 wire 形状破坏性变更；全部为行为修复与门禁收紧

## Non-goals

- ZCode 内嵌浏览器（iab）登录无响应：三簇在正常 Chromium 均登录成功，判定环境特定兼容性，记录观察项不修
- error 场景（上游 5xx 呈现）代码改动：现实现未验证到是被探测误报阻断，修复后补验；若暴露新缺陷另立 change
- rbac.rs 四档权限矩阵本身的重排（只把既有 `settings.manage` 接到 provider 变更面）
- 触屏适配与 i18n（`webui-i18n-sweep` 承载）
- Auto 档 perm 回合缺收尾正文（O-B-01）：核实 fake-claude 桩语义，桩预期则仅文档注明
