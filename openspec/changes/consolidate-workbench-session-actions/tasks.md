# Tasks — consolidate-workbench-session-actions

## 1. 归档 agent-workbench

- [ ] 1.1 创建 `changes/2026-09-09-archive-agent-workbench`：specs delta 声明移除（REMOVED agent-workbench 全部 18 条 requirement，附 Reason/Migration 指向新 `workbench`），proposal 说明归档理由；verify: `openspec validate --all` 通过且无活跃 change 残留
- [ ] 1.2 `openspec archive` 后目录移入 `changes/archive/`，主 spec `agent-workbench` 删除；verify: `openspec list --specs` 不再含 agent-workbench

## 2. 归档 project-session-actions

- [ ] 2.1 创建 `changes/2026-09-09-archive-project-session-actions`：specs delta 声明移除（REMOVED project-session-actions 全部 5 条 requirement，Reason/Migration 指向新 `workbench`）；verify: `openspec validate --all` 通过
- [ ] 2.2 `openspec archive` 移入 archive、主 spec 删除；verify: `openspec list --specs` 不再含 project-session-actions

## 3. 归档 webui/projects

- [ ] 3.1 创建 `changes/2026-09-09-archive-webui-projects`：specs delta 声明移除（REMOVED webui/projects 全部 requirement，Reason/Migration 指向 `webui` SPA 工作台语义）；verify: `openspec validate --all` 通过
- [ ] 3.2 `openspec archive` 移入 archive、嵌套目录删除；verify: `openspec/specs/` 全平铺、无 webui/projects

## 4. 创建并落地新 capability `workbench`

- [ ] 4.1 本 change specs delta 提供 `specs/workbench/spec.md`（已建，ADDED 18 条 requirement = 合并基座）；verify: `openspec validate --change consolidate-workbench-session-actions` 通过
- [ ] 4.2 归档本 change，delta 合入 `openspec/specs/workbench/spec.md`；verify: `openspec validate --specs` 通过、新 capability 出现

## 5. 引用同步与收口

- [ ] 5.1 全文搜索 `agent-workbench` / `project-session-actions` 在 `openspec/specs/`（现指 testsuite-acceptance:44 与 raise-core-coverage archive delta）与 `glossary.md` 的引用，改指 `workbench`；verify: grep 结果仅剩 archive 目录中的历史提及
- [ ] 5.2 归档动作全部完成后运行 `openspec validate --all` 无 ERROR；verify: invalid count = 0
