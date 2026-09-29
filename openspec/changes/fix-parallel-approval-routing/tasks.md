# Tasks — fix-parallel-approval-routing

## 1. 失败测试钉根因

- [x] 1.1 前端单测：审批面合并——构造「读模型 [A,B] + WS 推送 [B,A]」等 4 种到达顺序（GET先/WS先/交错/重复帧），断言每张卡的呈现内容（tool_name+args）与其提交时携带的 request_id 配对一致。当前实现应红（若不红，根因不在合并层，转 1.2 优先）。
  - 状态：配对 4 测全绿——合并层按键正确（根因不在合并层，转 1.2）。真正红的是**显示顺序确定性**测（同集合推送倒置序 vs 读模型序呈现不一致），归因见 2.1。
- [x] 1.2 后端单测：审批读模型枚举——同一待批集合两次枚举顺序稳定（排序键钉死）；泊车登记→枚举序与登记序一致。（`approval_restore_identity_test.rs::pending_permission_requests_enumerate_in_a_stable_sorted_order`，键 = request_id 字典序，绿——stall 投影已排序，此测钉死。）
- [x] 1.3 dispatch 单测：两个挂起 hook_callback，乱序决策（先 Deny 后 Allow），断言各自结果路由到正确的请求；vendor 路径（cc-agent-sdk 补丁）在测试覆盖内。（`driver.rs::out_of_order_decisions_route_to_their_own_parked_request`，绿——按键摘除 oneshot，无队列序偏差；沙箱 API 乱序批复实测同样各归其位。）

## 2. 修复

- [x] 2.1 审批面状态改 `request_id` 主键 map 合并；渲染 key 禁用 index（依 1.1 归因落点修复，可能同时涉及前端合并与后端枚举序两处）。
  - 状态：合并本就以 request_id 为主键（round3 2.2），未改。落点是**显示序**：并行推送到达序可倒置（vendor 每条 hook_callback 独立任务竞发），review-card `mergeRows` 改为合并后按 request_id 字典序（与读模型同键）输出——任意到达序/刷新重建收敛到同一显示序。
- [x] 2.2 后端读模型枚举加稳定排序（与 1.2 的测试键一致）。（已在位：`StallRegistry::parked_requests` 按 request_id 字典序；1.2 测试钉死，无代码改动。）
- [x] 2.3 若 1.3 暴露 vendor 补丁路由偏差，修 vendor/cc-agent-sdk 补丁并记录补丁差异。（条件不成立：1.3 与沙箱实测均无偏差——vendor 每条 control_request 独立任务、响应原样回带自己的 request_id；不改 vendor。）

## 3. 回归与验收

- [x] 3.1 全量 `cargo test` 过（dispatch / webui / 前端单测）；既有 parallel-permissions 套件不回归。
  - 状态：`pnpm test` 749 全绿（31 文件）；`cargo build` 0 error；`cargo test` 466 过 + 2 失败——两失败均在根 crate（`core_channel::tests::channel_socket_default_follows_sebas_home_and_env_override` 的 Windows 路径断言 bug，同提交主仓 checkout 同样失败；`watchdog::services::tests::pinned_lifecycle_never_touches_operator_home` 是前者并发改 env 的竞态，单跑即绿）——与本 change 零交集（本 change 未触根 crate）。既有 parallel-permissions 进程级套件为 `--ignored`，按门禁未跑，代码路径（泊车/路由）由本 change 新增单测与 GUI 手测覆盖。
- [x] 3.2 GUI 手测（沙箱，fake-claude `parallel`）：四轮——①顺序决策（Allow→Deny）②乱序决策（Deny→Allow）③刷新后顺序决策 ④刷新后乱序决策；每轮断言两工具结果不互换，截图留档。
  - 状态：自建沙箱（工作区构建、9875 端口、fake-claude 桩、auth 关）全过——四轮挂起顺序均稳定为 [tc-par-1(Bash), tc-par-2(Read)]、内容与 id 配对一致、四轮决策全部各归其位（不互换）。截图与判定日志：`C:/Users/cupen/AppData/Local/Temp/sebas-qa/qa-evidence/fix-parallel-approval-routing/`（R1–R4 cards/results + gui-checks.log）。
- [x] 3.3 单卡回归：ask 模式 `perm` Allow/Deny 两路径 GUI 手测不回归。（同沙箱：Allow → 工具执行、Deny → `denied by fake`，各一卡一路径，截图同目录 r5/r6。）
