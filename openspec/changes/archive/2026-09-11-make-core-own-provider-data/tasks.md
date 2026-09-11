## 1. core 管理面

- [x] 1.1 把默认 provider / 默认 model 并入 core 状态库 `settings` 域，与 provider 数据同事务落盘；验证：`cargo test -p sebas-webui` 或 `tests/state_persistence_test.rs` 中断言默认值重启后仍在、且不产生 defaults 文件
- [x] 1.2 core 的 provider / alias mutation 校验补齐（未知字段、非法 payload 走 typed rejection）；验证：`tests/state_mutation_rejected_does_not_silently_swallow` 扩展用例通过
- [x] 1.3 core 提供静态 preset 表的只读读取（复用既有代码表，不新增数据）；验证：单测断言 core 读到的 preset 与 `sebas-router::config::presets()` 一致
- [x] 1.4 `defaults.json` 一次性导入 `settings` 域并留日志，之后不再读该文件；验证：单测覆盖「有 defaults.json 时导入一次」「再次启动不重复导入」

## 2. router 去掉写路径

- [x] 2.1 删除 `admin.rs` 的 provider / alias / defaults / probe mutation 处理器与路由注册；验证：`sebas-router/tests/admin_test.rs` 中断言这些路由返回 404，`cargo test -p sebas-router` 通过
- [x] 2.2 删除 `channel_write` 的 providers.json 回退写与 `write_overlay_rmw` 写路径；验证：单测断言无 core 通道时任何管理操作都不写文件
- [x] 2.3 `put_defaults` / `clear_defaults_for` 删除，改由 core 承担；验证：全仓 grep 无 `defaults.json` 写入点，`cargo test -p sebas-router` 通过
- [x] 2.4 保留并确认只读面：stats / metrics / preset 表读取 / 通道订阅；验证：`External change hot reload` 相关用例与 `/admin/stats` 用例通过

## 3. WebUI 改由 core 承载

- [x] 3.1 为 `sebas-webui` 接入 core 通道客户端（复用 root binary 既有 `CoreChannelBackend`），并注入到 `WebUiState`；验证：`cargo build` 通过，`sebas-webui/tests/gateway_bff_test.rs` 以 core 后端替换 RouterClient 后编译通过
- [x] 3.2 `/router/api/*` 的 provider / alias / defaults / preset 处理器改为读写 core 状态库；验证：BFF 测试断言创建 provider 后 GET 立即读到新值（无重启）
- [x] 3.3 无 core 可达时如实 503，不回退陈快照；验证：BFF 测试断言 core 不可达时 mutation 返回 503 且 GET 不返回过期数据
- [x] 3.4 飞书 `/provider` 卡片链路复核仍走 `state_mutate`，无需改动；验证：`sebas-dispatch/tests/provider_test.rs` 与 im 前端用例通过

## 4. spawn 权威修正

- [x] 4.1 `read_overlay_item` 去掉 legacy `providers.json` 优先，改读状态库；验证：单测覆盖「文件与库不一致时以库为准」
- [x] 4.2 保留无状态库时的文件降级读取，并如实上报来源；验证：单测覆盖降级路径仍可用

## 5. 测试与门禁

- [x] 5.1 更新 `sebas-router/tests/admin_test.rs`：删除或改写 provider/alias CRUD 往返、probe apply 用例为「路由不存在」；验证：`cargo test -p sebas-router` 全绿
- [x] 5.2 更新 `tests/state_*` 与 `sebas-webui/tests/gateway_bff_test.rs` 承载变更；验证：`cargo test --all-targets` 全绿
- [x] 5.3 Playwright `settings.spec.ts` / `models.spec.ts` 中 provider 管理旅程改走新承载；验证：`invoke testsuite-webui-server` 后 `pnpm playwright test` 全绿
- [x] 5.4 降级验证：旧二进制 + 新库可读、无数据格式变更；验证：`invoke testsuite-webui-sandbox` 中用新库启动旧构建，确认 provider 列表可读
- [x] 5.5 冲突复核：并行的 `workbench-turn-queue`（及 `workbench-conversation-view`）也改 `webui`，其中 `HTTP route surface` 与本 change 同名增量重叠；若它先归档，须按归档后的正文重生成本 change 的该 MODIFIED 块再应用；验证：重生成后 `openspec validate make-core-own-provider-data --strict` 通过
