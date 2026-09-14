# Tasks: status-driven-service-rows

## 1. 后端：ServiceStatus 去合成行

- [ ] 1.1 `src/watchdog/executor.rs` `service_status()`：删除列表头部注入的 `watchdog` 行与尾部注入的 `updater` 行，列表只由 `all_snapshots()` 组成；验证 `cargo check` 通过
- [ ] 1.2 更新钉死合成行的既有测试（`executor.rs` 中 `assert!(names.contains(&"watchdog"))` 等）：改为断言条目名集合 ⊆/== 受管集合（core/webui/router/im，按启用子集）；验证 `rtk cargo test --lib watchdog` 全绿
- [ ] 1.3 检查 CLI（`main.rs` `ctl services` 渲染）与 IM（`im_cmd.rs`）消费面无对 `watchdog`/`updater` 行名的特判；如无则仅记录结论，不改代码；验证：grep 零命中

## 2. 前端：status 驱动的动作区

- [ ] 2.1 `settings-modal.ts` `renderServiceRow()`：按 status 映射实现动作互斥（running→■；stopped/disabled→▶；starting/restarting→禁用过渡占位；degraded/failed-startup→■+⟳；非过渡态保留 ⟳；busy 全禁用）；core 行删去全部按钮（含 ⟳）与 `alwaysOn` 特判残留；验证 `rtk vitest` settings-modal.test.ts 全绿
- [ ] 2.2 更新/新增前端测试：core 行零按钮、running 只显 ■、stopped 只显 ▶、过渡态占位不可点、degraded 显 ■+⟳、无 watchdog/updater 行渲染；验证 vitest 用例齐且绿
- [ ] 2.3 `api/client.ts`：删除无调用方的 `restartCore()` 方法及其类型引用；验证 grep 无 `restartCore` 残留、`rtk vitest` client.test.ts 全绿

## 3. 前端：定宽对齐

- [ ] 3.1 `settings-modal.ts` 内联样式：`.service-actions` 改定宽（CSS 自定义属性声明一次，按 2 钮宽度）+ `justify-content: flex-end`，core/过渡态行以等宽占位填充；验证 vitest 快照/断言通过
- [ ] 3.2 验证状态列对齐：三行并置（core 只读、running 两钮、过渡占位）时状态圆点与文字同一 x；验证：组件测试断言或沙箱目验截图

## 4. 端到端验收

- [ ] 4.1 `cargo build` 后按 AGENTS.md 沙箱配方起 core（`--webui`，端口 ≠ 9797）+ 独立 router + fake-claude stub；`GET /api/admin/services` 返回条目仅受管名、无 watchdog/updater；验证：curl 断言
- [ ] 4.2 沙箱目验 Services 分区（`invoke testsuite-webui-sandbox`，127.0.0.1:9879）：四行真实服务、按钮互斥、core 只读、状态列对齐、router force 停止保护流程不受影响；验证：操作员或浏览器冒烟通过
- [ ] 4.3 全量回归：`rtk cargo test`（后端）+ `rtk vitest`（前端）全绿；验证：CI 或本地输出
