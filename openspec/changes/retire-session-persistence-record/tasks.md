# Tasks — retire-session-persistence-record

## 1. 退役记录

- [x] 1.1 REMOVED delta（2 条）就位，附 Reason/Migration；verify: 文件结构合规
- [x] 1.2 proposal 成文；verify: `openspec validate --changes retire-session-persistence-record` 通过

## 2. 收口

- [ ] 2.1 本 change 仅记录用途（归档器会因目标 spec 已删而失败），作为文档 change 随批次 F 统一收口；verify: 决定并执行
- [ ] 2.2 testsuite-acceptance 核心集移除 session-persistence 引用；verify: grep 无残留、validate --all 通过
