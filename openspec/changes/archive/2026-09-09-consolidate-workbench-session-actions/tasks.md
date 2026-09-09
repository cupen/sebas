# Tasks — consolidate-workbench-session-actions

## 1. 收敛准备（已完成）

- [x] 1.1 建立新 capability `workbench` 的 ADDED delta（18 条 requirement = project-session-actions 细化文本 + agent-workbench 独有语义合并）；verify: `openspec validate --changes consolidate-workbench-session-actions` 通过
- [x] 1.2 三个 REMOVED delta：`agent-workbench`（18 条）、`project-session-actions`（5 条）、`webui/projects`（7 条，Reason 指向新 SPA 工作台取代），均附 Reason/Migration；verify: 30 条 requirement 全覆盖、无游离 H1 头

## 2. 归档执行

- [ ] 2.1 运行 `openspec archive consolidate-workbench-session-actions -y`：ADDED 合入主 spec 生成 `openspec/specs/workbench/spec.md`，REMOVED 从主 spec 删除 `agent-workbench`、`project-session-actions` 与嵌套 `webui/projects`；verify: 命令成功、change 移入 `changes/archive/2026-09-09-*`、`openspec list --specs` 出现 workbench 且不再含三个旧 capability
- [ ] 2.2 检查归档后的 `workbench/spec.md`：Purpose 非 TBD 占位、requirement 数 = 18、文本为合并基座；verify: `openspec show workbench` 输出正常、`openspec validate --specs` 通过

## 3. 引用同步与收口

- [ ] 3.1 全文搜索 `agent-workbench` / `project-session-actions` 在 `openspec/specs/` 与 `glossary.md` 的引用（现指 testsuite-acceptance、嵌套 webui/projects 内链），改指 `workbench` 或按语义落位；verify: grep 结果仅剩 archive 目录中的历史提及
- [ ] 3.2 glossary「遗留偏离」节移除 `project-session-actions` 与 `webui/projects` 两条（已归档）并保留 `workbench` 归属说明；verify: glossary 与目标树一致
- [ ] 3.3 归档后运行 `openspec validate --all` 无 ERROR；verify: invalid count = 0
