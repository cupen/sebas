## REMOVED Requirements

### Requirement: Project as the organizing unit
**Reason**: 项目作为顶层组织单元的语义原样并入新 capability `workbench`，本条随 agent-workbench 一起归位。
**Migration**: 见 workbench「Project as the organizing unit」。

### Requirement: Project registry persistence is WebUI-owned
**Reason**: 项目注册表独立持久化、WebUI 不写 router state file 的约束随注册表归属语义迁入 `workbench`。
**Migration**: 见 workbench「Project registry persistence is WebUI-owned」。

### Requirement: Session attribution
**Reason**: 会话按项目目录归组、无目录的 Feishu 会话归入 origin 分组语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Session attribution」。

### Requirement: Concurrent projects
**Reason**: 多项目会话并行、切换项目不改路由的并发语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Concurrent projects」。

### Requirement: Unseen-turn seam
**Reason**: 未读转折缝与每浏览器 seen 边界语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Unseen-turn seam」。

### Requirement: Composer promises only what the process can do
**Reason**: composer 交付保证与 core 不可达时禁用/自恢复语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Composer promises only what the process can do」。

### Requirement: Session origin is visible
**Reason**: 会话展示 Feishu/workbench 来源的语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Session origin is visible」。

### Requirement: Project view states real working-copy context
**Reason**: 项目头展示路径与 git 分支、读不到则省略的语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Project view states real working-copy context」。

### Requirement: Session execution over the native agent kernel
**Reason**: 会话经原生内核（`native` 执行体）执行的语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Session execution over the native agent kernel」。

### Requirement: Gated call approval on the native kernel
**Reason**: 原生内核门控工具调用的 WebUI 审批卡审批、fail-closed 语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Gated call approval on the native kernel」。

### Requirement: Add project via directory browser
**Reason**: 本项目注册语义被 project-session-actions 的新版「Add project via directory picker」取代（lazy-load 目录树 `browse-dirs`、root 限定、Windows 路径往返修复），非操作独有内容已无保留价值。
**Migration**: 见 workbench「Add project via directory picker」。

### Requirement: New session without prompt
**Reason**: 0-turn 占位会话语义在 project-session-actions 中是更新更全的版本（多 API 创建场景），以该文本并入 `workbench`。
**Migration**: 见 workbench「New session without prompt」。

### Requirement: Session archive
**Reason**: 会话归档/只读/恢复语义在 project-session-actions 中是更全版本（含只读拒绝 400 场景），以该文本并入 `workbench`。
**Migration**: 见 workbench「Session archive」。

### Requirement: History group is the archive
**Reason**: History/Inbox 分组语义在 project-session-actions 中逐字相同，以该文本并入 `workbench`。
**Migration**: 见 workbench「History group is the archive」。

### Requirement: Archive expiry
**Reason**: 归档保留期删除语义在 project-session-actions 中是更全版本（含「无操作员通知」与 in-retention 场景），以该文本并入 `workbench`。
**Migration**: 见 workbench「Archive expiry」。

### Requirement: Execution-body availability is stated, not discovered
**Reason**: 执行体可用性由会话后端上报、不可用则明示原因并在 composer 阻断提交的语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Execution-body availability is stated, not discovered」。

### Requirement: Model selection covers the native kernel
**Reason**: 原生内核会话模型选择语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Model selection covers the native kernel」。

### Requirement: Model selector offers the backend catalog before any session
**Reason**: 会话前即提供后端目录模型选择的语义迁入 `workbench`，文本未变。
**Migration**: 见 workbench「Model selector offers the backend catalog before any session」。
