# Tasks: conversation-incremental-sync

## 1. 后端：entries_after 参数

- [ ] 1.1 `sebas-webui/src/api.rs`：`session_detail` 增 `Query` 解析
  `entries_after: Option<u64>`，透传 `turns(key, entries_after.unwrap_or(0))`
  切片 entries，其余字段照常；非数字 → 400；`session_endpoints_test.rs`
  补用例：无参全量（回归）、`entries_after=N` 只回 >N 且其余字段完整、
  `entries_after=abc` 400、`entries_after` 超过最大 position 返回空序列
- [ ] 1.2 `cargo test -p sebas-webui` 全绿

## 2. 前端：内存游标与增量 merge

- [ ] 2.1 `client.ts`：`sessionDetail(key, entriesAfter?)` 增可选参数；
  `dashboard.ts`：per-session 游标表（Map），`loadFocused` 按游标走
  全量/增量；merge 按 position 升序 + `<= 游标` 过滤去重，游标仅在
  merge 成功后推进，失败保持原序列与游标；会话切换保留各自游标
- [ ] 2.2 `dashboard.test.ts` 补用例：首拉全量、WS 触发的 refetch 增量
  append、切回会话走游标、失败后游标不推进且下次恢复、重载后重新全量
- [ ] 2.3 `pnpm --dir sebas-webui/frontend test` 全绿

## 3. 收尾

- [ ] 3.1 浏览器套件既有「重载恢复」旅程回归通过（增量不改其语义）；
  `invoke testsuite-webui` 冒烟
- [ ] 3.2 `openspec validate conversation-incremental-sync --strict` 通过
