# Tasks — retire-workbench-legacy-specs

## 1. 记录退役（已完成）

- [x] 1.1 三个 REMOVED delta 归位：`agent-workbench`（18）、`project-session-actions`（5）、`webui/projects`（7），附 Reason/Migration；verify: 每 delta 无游离 H1、30 条 requirement 全覆盖
- [x] 1.2 proposal/design 成文；verify: `openspec validate --changes retire-workbench-legacy-specs` 通过

## 2. 收口

- [ ] 2.1 本 change 仅记录用途（归档器会因目标 spec 已删而失败），作为文档 change 随批次收口归档，或保持 active 待批次 F 统一处理；verify: 决定并执行其一
- [ ] 2.2 `openspec validate --all` 确认无 ERROR；verify: invalid = 0
