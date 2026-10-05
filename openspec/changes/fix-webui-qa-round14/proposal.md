## Why

第十四轮 GUI 全链路验收（五簇 subagent 黑盒：入门/项目/会话链路/权限审批/设置管理/RBAC，报告在沙箱 `sebas-qa/reports/QA-1..5.md`）发现 3 簇 P2、7 项 P3 与 3 项 UX 缺口。核心问题：后端已有的类型化拒绝被 GUI 吞掉（容量满静默失败）、模型别名有 CRUD 无消费入口、`/sessions` 总览页绕过角色可见性、viewer「只读」被 switch 写操作耦合挡死，外加一批状态一致性缺陷。沿 round10–13 惯例随验收轮次收口。

## What Changes

**规格收口（spec delta）**
- 会话创建失败 SHALL 呈现类型化拒绝文案（如「会话数已达上限 32」），不再静默跳转幻影会话 URL（D-3-1/D-4-3）
- 模型别名进入 GUI 消费面：composer 模型菜单与新建会话模型下拉合并别名短名；别名目标 provider 下拉列出 config.toml 种子行——种子行列出但禁选并说明（外键约束：别名只能绑定 store provider），零 provider 空态说明原因（D-4-1/D-4-2）
- `/sessions` 总览页消费 role-visibility：viewer 隐藏新建表单与卡片写入口；被拒操作明确呈现「无权限」；错误横幅区分「无权限」与「加载失败」（D-5-1/2/3）
- viewer 只读语义落地：打开会话 = 纯 GET 只读视图（不触发 switch 写），转录可读（D-5-2）
- 历史分组展开态持久、会话行生命周期状态指示、新建会话可预命名（QA-1/UX 缺口）

**实现修复（已有 spec 已覆盖，纯 bug，不立 delta）**
- 未读徽标按会话键归属（违反 session-unread-badge 行级归属）
- 历史分组计数对齐「归档总数」（spec 明文）
- agent 编辑切「启动定义」后 path 预填硬编码覆盖存量（违反「编辑表单以存储值预填全部字段」）
- 新会话项目标签瞬时滞后（D-2-1）；停滞收尾后「等待」标记清除（违反 session.updated 逐翻转语义）
- 文案两处：Settings→设置、History→历史（D-5-4）

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`：会话创建错误呈现类型化拒绝；模型选择面合并别名短名（含目标 provider 下拉种子行）；/sessions 页角色可见性与错误语义
- `webui-user-management`：viewer 只读口径=工作台读面含转录（GET 只读视图，不耦合 switch 写）；被拒操作 SHALL 有明确呈现；修去与 provider-management 角色门相悖的陈旧段落
- `agent-workbench`：历史分组展开态持久；会话行生命周期状态指示（status_slug 驱动）；创建弹窗可选预命名

## Impact

- `sebas-webui/frontend`：workbench、sessions、settings-aliases、new-session-dialog、unread-cursor、split-persist、role-visibility 消费面、文案
- `sebas-webui/src`：错误形状透传（若需）；RBAC 中央表预计不动（GET 本就不挂权限）
- 不动：dispatch 容量语义与数值、router、跨进程 wire 形状、协议 fixture

## Non-goals

- 不改 dispatch Capacity(32) 数值，不做容量可配置化（如需另立项）
- 不做会话硬删除（归档即删除形态，既有设计）
- 不动 admin 无用户分区的 root-only 设计（canManageUsers）与「项目默认 agent 记住最近使用」语义（agent-workbench 既有 spec，QA-4 观察项即此行为，非缺陷）
- 不处理 native backend 真实凭据链路与 watchdog 服务操作（沙箱无 watchdog 进程）
- D-2-3 usage 序号口径为 fake-claude 桩进程重启伪影，不修核心（记录存档）
