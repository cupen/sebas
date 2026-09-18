## 1. 创建闸门（wire 层）

- [x] 1.1 `CreateSessionRequest.project_id` 语义改为必填：省略 / `null` / 空串 → typed 400「project_id 必填：会话必须从属于项目」；未知 id → 400「未知 project_id: {id}」；越界本机项目仍 400（add-workspace-root 判据不变，随必填路径一并生效）；0-turn 占位走同一闸门
- [x] 1.2 项目默认 agent 的落点随必填简化（不再有 `if let Some(id)` 分支）
- [x] 1.3 不变量用例：`sebas-webui/tests/api_endpoints_test.rs::create_session_without_a_project_is_rejected_400`——省略 / `null` / 空白 / 未知 id / 无 prompt 占位五种形态一律 400，且文案点名 `project_id`

## 2. 失败不改归属（内存层）

- [x] 2.1 `Map::fail_spawn` 就地改状态（`get_mut` + 只写 state/last_active），保留 `project_dir` / `pending_kind` / `pending_model` / `pending_mode` / `desired_mode`
- [x] 2.2 删除 `Mapping::spawn_failed` 构造器（重建型映射是「造无项目失败会话」的陷阱）与其唯一测试断言
- [x] 2.3 回归用例：`sebas-dispatch/tests/spawn_race_test.rs::fail_spawn_keeps_the_sessions_project_and_agent`——失败后 `project_dir` / `pending_kind` / `pending_mode` / `desired_mode` 原样

## 3. 持久化清退（飞书例外）

- [x] 3.1 判据收敛到 `mapping_may_lack_project(channel, project_dir)`：飞书放行，其余要求非空 `project_dir`
- [x] 3.2 `restore_json` 丢弃不合判据的存量行（warn 留痕，不迁移）——操作者拍板历史数据可整片删
- [x] 3.3 `dump_json` 同样跳过（双向兜底，幽灵行不落盘）
- [x] 3.4 内部归档记录键 `closed-*` 豁免判据；`preserve_closed_mapping` 改为带源键、连身份（project_dir/kind/model/mode）一起抄进归档记录（否则 D4 的「原映射保留在存储」会被清退抹掉）；调用点 `src/session_boot.rs` 同步
- [x] 3.5 既有单测按新不变量补项目归属（`pending_kind_round_trips_through_disk_shape`、`desired_mode_migration_tests`、`dump_uses_structured_channel_key_round_trip`）

## 4. 前端收口

- [x] 4.1 `api.createSession` 的 `projectId` 收为必填 `string`
- [x] 4.2 `/sessions` 视图创建表单补必选项目下拉（空注册表 → 禁用 + 「尚未注册项目」；提交前本地拦下并说明；成功/失败态沿用既有 error 呈现）
- [x] 4.3 创建弹窗：`projectId` 参与确认门禁（无目标项目 → 确认禁用），注释点明「创建只从项目行 + 发起」
- [x] 4.4 前端测试：client 用例改为恒带 `project_id`（删「inbox task」路径断言）；弹窗 mount 缺省带项目 + 新增「无项目 → 确认禁用」用例；vitest 545 全绿

## 5. 套件与旅程对齐

- [x] 5.1 `tests/support::scene_project_id`：沙箱场景项目幂等注册助手（workspace root 已钉沙箱目录，注册必界内）
- [x] 5.2 `testsuite_e2e_test`（23 处创建站点）与 `testsuite_acceptance_test`（3 处 + `create_session` 助手统一注入）带上项目
- [x] 5.3 `testsuite-webui`：`errors` 旅程恢复强断言——spawn 失败的会话仍挂在项目行下、圆点读作 `failed`（raw status `spawn-failed`）；`expectSessionStatus` 助手按行标签定位
- [x] 5.4 类型诚实性回归：撤销误加到 `StatusSlug` 的 `spawn-failed` 成员（派生 slug 只有七词；raw status 才是 `spawn-failed`）

## 6. 门禁

- [x] 6.1 `cargo test --workspace --no-fail-fast` 全绿（1974 passed / 53 ignored）
- [x] 6.2 `pnpm vitest`（frontend）全绿（545）；`tsc --noEmit` 仅剩 `settings-modal.test.ts` 两处**既有**报错（本 change 未触碰该文件；新加的弹窗门禁用例已避开 `toBe` 的二参写法）
- [x] 6.3 `invoke testsuite-e2e` 37/37；`invoke testsuite-acceptance` 9/9
- [x] 6.4 `invoke testsuite-webui` 四配置全绿（exit 0；detached 一处 console 收集器偶发重试即绿）；沙箱实测：无项目创建 400（省略/空白/未知三种形态）、带项目 + 坏 agent 的失败会话仍带 `project_id` 与 `agent_kind`
- [x] 6.5 `openspec validate --all` 55/55 通过

### 门禁期间发现并修掉的两处

- **`scene_project_id` 的路径比对必须规范化**：沙箱目录可能以符号链接形态存在，注册表落的是服务端规范化后的路径——`e2e` 里三个用例（`mode_threads_to_agent_argv` / `two_sessions_spawn_and_turn_concurrently` / `turn_queue_timing_and_dropped_accounting`）因此报「scene project missing from list」。改为两侧都 `canonicalize` 后比对。
- **验收套件的 `native_agent_turn_via_router_journey` 用的是进程内 webui**（`core --webui`，端口 ≠ `sb.webui_url()`），注册项目必须走 `dashboard_url`，否则 transport 直接失败。
- **`errors` 旅程的行标签不能按 prompt 定位**：失败会话没有 `chat_id`/`session_id_short`，`fullSessionLabel` 落到键尾段（= reference）——故按解码后的 reference 定位；圆点读的是七词 slug `failed`（raw status 才是 `spawn-failed`）。
