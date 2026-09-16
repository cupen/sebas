## 1. 后端通道

- [x] 1.1 `session_backend.rs` trait 新增 `reachability_updates() -> broadcast::Receiver<Reachability>`；in-process/fake 后端返回立即关闭的接收端（`subscribe_turn_events` 先例）；`cargo test` 编译面与 fake 行为单测通过
- [x] 1.2 `src/core_channel/client.rs` 的 `set_status()` 发布翻转：与旧值比较、真翻转才广播；单测覆盖 Connected↔Failed 各 kind 翻转、重复 set 不重复发布、startup summary 富化随帧携带
- [x] 1.3 `api.rs` 注册 `core.reachability.get` handler（读 `reachability()` 当前态，payload 对齐 `reachability_payload`）与翻转→`core.reachability` Notification 广播支路；ws 集成测试覆盖「get 返回当前态」「set_status 翻转推帧」「payload 含 kind/cause」

## 2. 前端订阅与状态收敛

- [x] 2.1 `app-shell.ts`：删除 `CORE_REACHABILITY_POLL_MS` 定时器与 `pollCoreReachability`；`/ws` open/重连时 `request("core.reachability.get")` 初始化，订阅 `core.reachability` 更新结构化可达性状态（ok/kind/cause）；vitest 单测覆盖 get 初始化、翻转更新、重连收敛
- [x] 2.2 `workbench-composer.ts`：删除 `WORKBENCH_REACHABILITY_POLL_MS` 定时器、挂载 fetch 与内部 `unreachable` 态，提交门改为消费 shell 下传的可达性状态（下传通道按 design D4 落）；单测覆盖「不可达禁用、恢复通知解除、无 fetch 残留」

## 3. 清理与回归

- [x] 3.1 全仓 grep 确认无 `/api/summary` 轮询残留（`setInterval` + `api.summary` 组合为零），dashboard/settings/初始加载的按需调用保持；`testsuite-webui` 沙箱回归：浏览器套件 69-70/72，1-2 例失败为操作者文档记载的串行多播遗留（隔离跑全绿、失败点跨 run 漂移），「停 core→横幅/禁用翻转」由 Rust FlipBackend 集成测试承载（浏览器级无停 core 旅程，如实记录）
- [x] 3.2 `openspec validate add-core-reachability-ws-push` 通过；`tests/acceptance/COVERAGE.md` 与 `tests/testsuite-webui/README.md` 增补推送语义覆盖说明（两文件在操作者在途脏集中，增补仅追加、不随本 change 过渡提交）；落地顺序（协议层 → 本 change → 呈现层 change）已在双方 proposal/design 记录，呈现层 change 的轮询表述已随本规划修订
