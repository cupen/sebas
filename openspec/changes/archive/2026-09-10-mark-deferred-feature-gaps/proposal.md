## Why

审计收尾发现最后一批「spec SHALL 承诺了尚未构建的特性」的漂移（代码侧 bug 与文档错误已全部修复，见 beads 已关闭项与 reconcile-audit-spec-drift）。这些特性缺口是真路线图项（媒体内容链路 = sebas-03s、ack reaction、后台工作池、policy 配置面等），不该在无产品拍板的情况下突击实现，也不该假装 spec 与现实一致。项目既有先例（align-session-map-persistence-spec）是**在 spec 文本里如实标注 deferred**——本 change 把该模式套用到全部剩余缺口。

## What Changes（直接修主 spec，全部为「标注现状 + 指向跟踪项」）

- **feishu-bridge**：媒体内容（Image block / ACP content block / mime / file_key 解析）标注 deferred，现状 = 文本标记 + 路径附件，指向 beads sebas-03s。
- **feishu-reactions**：per-message `Get` ack 标注 deferred（`record_ack`/`take_ack` 就绪无调用方）。
- **im-service**：`[card]` 截断/折叠/thinking 旋钮标注 deferred（仅 theme_color 生效）。
- **channels**：core「SHALL NOT hold presentation state」补充现实注——引擎内部卡片机是死路径（出站泵丢弃），待移除，非呈现面。
- **acp-session-mapping**：解决 spec-vs-spec 矛盾（kwf-H2）——无映射时按实现文档批准「以 routing id 尝试 load，被拒才 fresh」；归档映射「仍可寻址」的过度承诺改为「保留备查」。
- **agent-core**：long-running pool / policy 配置面标注 deferred（内联异步执行、Rust-API-only）；`permission_decision` 命名对齐实现（`ToolPolicy` 事件 + outcome 词表）。
- **agent-workbench**：session origin 标签标注 deferred（`channel` 字段在、UI 未消费）。

## Capabilities

（无新增/修改 requirement 语义——每处都是现状标注。skip_specs: true，直接修主 spec。）

## Impact

纯 spec 文本标注；无代码变化。所有 spec 与现实一致后，「spec 审阅无漂移」达成——剩余 deferred 项各有 beads/issue 跟踪。

## Non-goals

- 不实现任何 deferred 特性（各有跟踪项）。
- 不动 operator 在途的 capability 拆分所涉 spec。
