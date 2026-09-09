# Tasks — retire-session-persistence

## 1. 语义拆归（已完成）

- [x] 1.1 provider-management ADDED「Default selection wire compatibility and atomic delete」；verify: delta 覆盖 object/bare-string、原子删除、state store 持久化
- [x] 1.2 state-store ADDED「Runtime state boundaries for persisted session state」；verify: delta 覆盖不持久化清单 + 会话 map 每变更持久三场景

## 2. 退役与收口

- [ ] 2.1 归档本 change：ADDED 合入 provider-management / state-store 主 spec；verify: `openspec archive retire-session-persistence -y` 成功、validate --specs 通过
- [ ] 2.2 物理删除 `openspec/specs/session-persistence/`，建 retire 记录（REMOVED delta + proposal 说明退役）；verify: 记录 change validate 通过
- [ ] 2.3 testsuite-acceptance 核心集移除 session-persistence（会话管理簇只剩 session-lifecycle、acp-session-mapping）；verify: grep 无残留
