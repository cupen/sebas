# 验收矩阵（testsuite-acceptance）

> 账本规则：每个能力一行，requirement 簇逐条标注命中证据（验收旅程 J*、进程级 e2e E*、
> 既有测试引用）或豁免（注明 cause）。未命中且未豁免 = 缺口（⚠️）。
> **核心功能集**（90% 硬指标，raise-core-coverage-to-90 起为五簇）：① 会话管理 =
> session-lifecycle + session-persistence + acp-session-mapping；② models 管理 =
> acp-model-selection + router-model-aliases + provider-management；③ agent workbench
> 相关 = agent-workbench + permission-flow；④ 项目管理 = project-session-actions +
> state-store(projects) + webui(projects 面)；⑤ 通道与监督 = core-session-channel +
> watchdog（本期新增簇，界定见主 spec「核心功能集界定」）。
> 核心集增删必须留变更说明。
>
> 旅程用例：`invoke testsuite-acceptance`（`tests/testsuite_acceptance_test.rs`）
> 冒烟用例：`invoke testsuite-e2e`（`tests/testsuite_e2e_test.rs`）
> 浏览器级旅程：`invoke testsuite-webui`（`tests/testsuite-webui/`，Playwright + chromium，
> 旅程账本见 `tests/testsuite-webui/README.md` 与 `openspec/changes/add-webui-playwright-tests`）

## 矩阵图例

- ✅ = 命中（含证据）　⚠️ = 缺口（未命中且未豁免）　🚫 = 豁免（cause）

## 核心功能集统计（达标复核 2026-09-08，五簇 90% 口径）

复核基数 = 三期前置变更合并后的 main：fail-fast `5eb1d85`、harden `52847f1`、
cover-channel `f14767e`（merge commit，可由 `git log main --grep` 复现，见 tasks 1.1
grep 清单）。复核为 requirement 级全量重数（含三期新增 requirement 与本期补行），
豁免不计分母；命中口径不变：任一测试层（验收旅程 J、进程级 e2e E、集成/单元/前端
单测）完整命中该 requirement 即计入。

| 核心簇 | requirement 数（计分分母） | 命中 | 命中率 | 豁免 | 套件内旅程 |
|---|---|---|---|---|---|
| ① 会话管理 | 13 | 13 | 100% | 0 | `session_lifecycle_journey` |
| ② models 管理 | 20 | 20 | 100% | 1 | `provider_governance_journey`、`native_agent_turn_via_router_journey` |
| ③ agent workbench 相关 | 24 | 24 | 100% | 0 | `workbench_aggregate_journey`、`projects_session_journey` |
| ④ 项目管理 | 13 | 13 | 100% | 0 | `projects_session_journey`；browser `deployment.spec.ts`（降级注册） |
| ⑤ 通道与监督 | 30 | 30 | 100% | 0 | E: `no_secret_assembly_end_to_end`、`secret_rotation_self_heal_across_core_restart`、`watchdog_supervised_core_recovery` |
| **核心合计** | **100** | **100** | **100%** | **1** | 每簇 ≥1 条 ✓ |

上一次复核（2026-09-05，四簇 80% 口径）记 65 条 / 62 命中 / 95%。本期重数差异来源：
三期变更新增 requirement 入账（⑤ 整簇 30 条；④ +1 webui projects 面「项目注册降级
如实提示」）；上期漏行补齐（② +2「Preset data follows the code table」「Set default
provider and model from the page」、③ +3「Execution-body availability…」「Model
selection covers the native kernel」「Model selector offers the backend catalog…」，
均为既有能力 requirement，证据已在库）；豁免不计分母规则回归（② 的 /provider 主卡
布局为飞书端卡片渲染，此前被误计入分母）；缺口收口（③ projects_branch 由 browser
旅程 `projects.spec.ts` 1.2 命中；permission-flow「无应答者 fail-closed」detached
旅程由 cover-channel `approval-detached.spec.ts` + 进程内 fail-closed 单测共同命中）。

全量 34 个能力目录、317 条 requirement（`grep -c '^### Requirement:'` 汇总，可重跑）。
豁免 23 条（飞书真实传输/端上卡片渲染、opencode CLI、真实模型跑分、watchdog
spawn-fail 进程级注入，见豁免清单）；requirement 级「未命中且未豁免」残留 0 条，
唯一保留的旅程级注记为 replay-debug 独立旅程（requirement 已由既有测试命中，
见其行内 ⚠️ 注记，非阻塞）。非核心簇维持「可见不设门槛」。

## 能力矩阵

### ① 会话管理（核心）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| session-lifecycle | 身份按会话/线程 | ✅ | `full_e2e_test`（ChannelKey 语义）|
| | 首条消息懒 spawn | ✅ | `full_e2e_test`；J: `session_lifecycle_journey` |
| | 双 spawn 竞争保护 | ✅ | `spawn_race_test` |
| | Dormant 懒恢复 | ✅ | `restart_recovery_test`；J: lifecycle（unix 段）|
| | 终态错误拆除 | ✅ | `error_test`、sebas-router 内联测试 |
| | 正常回合完成保活 | ✅ | `full_e2e_test`；J: lifecycle |
| | 流式背压排队 | ✅ | sebas-router 内联测试 |
| | 并发会话容量上限 | ✅ | sebas-router 内联测试 |
| | 重启恢复与损坏容忍 | ✅ | `restart_recovery_test`、`state_persistence_test`；J: lifecycle |
| session-persistence | 默认选择语义 | ✅ | `state_persistence_test` |
| | 运行态不入该库 | ✅ | `state_persistence_test` |
| acp-session-mapping | 路由 id ↔ ACP id 映射 | ✅ | `acp_session_mapping_test` |
| | 缺映射诚实回退 | ✅ | `acp_session_mapping_test` |

### ② models 管理（核心）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| acp-model-selection | 会话模型清单暴露 | ✅ | sebas-webui `session_endpoints_test`、src `agent_backend` 内联测试 |
| | set_config_option 换模型 | ✅ | add-acp-model-selection 测试（sebas-acp）|
| | 模型选择存活于会话生命周期 | ✅ | 同上 |
| router-model-aliases | 别名实体与持久化 | ✅ | sebas-router `config` overlay 测试 |
| | 别名解析优先级 | ✅ | 同上 |
| | 上游模型翻译 | ✅ | J: `provider_governance_journey`（my-claude→stub-model）|
| | 别名校验 | ✅ | sebas-router `admin_test` |
| | 别名作用域 | ✅ | sebas-router 内联测试 |
| provider-management | /provider 主卡布局 | 🚫 | 飞书端卡片渲染（豁免，见豁免清单；替代：卡片 JSON 单测）|
| | 模式切换 | ✅ | sebas-webui 内联/admin 测试 |
| | Provider CRUD 表单（API 面）| ✅ | sebas-router `admin_test`；J: provider_governance |
| | 密钥脱敏 | ✅ | sebas-router `admin_test`（api_key_configured）|
| | 模型探测 | ✅ | sebas-router `admin_test`（probe）|
| | Off 模式解析 | ✅ | src 内联测试 |
| | 直连模式 env 翻译 | ✅ | src 内联测试；J: native（SEBAS_AGENT_PROVIDER_* 直连 stub）|
| | 模型旗标优先级 | ✅ | src 内联测试 |
| | Gateway 模式 env 翻译 | ✅ | src `agent_backend` 内联测试 |
| | Provider 错误中止 | ✅ | src 内联测试 |
| | Provider 卡片反映 store 可用性 | ✅ | sebas-router `admin_test` |
| | Preset 数据跟随代码表 | ✅ | sebas-router `config.rs` 内联（`preset_fills_all_slots_and_models_from_code_table`、`preset_explicit_url_or_models_override_errors`、`preset_alias_reuses_table_defaults`、`preset_explicit_api_key_skips_default_env`）|
| | 页面设置默认 provider/model | ✅ | sebas-router `admin_test::agent_defaults_round_trip`（设置/校验/回读/清除）+ `agent_defaults_cleared_on_provider_delete`（API 面）；webui 代理 `/api/agent-defaults`（`routes.rs`）+ 前端 settings-modal「Default provider / model」单测 |

### ③ agent workbench 相关（核心）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| agent-workbench | 项目为组织单元 | ✅ | J: `projects_session_journey`；state-store projects |
| | 项目注册表 webui 持有 | ✅ | state-store 测试；J: workbench |
| | 会话归属 | ✅ | sebas-webui `session_endpoints_test` |
| | 并发项目 | ✅ | state-store 并发测试 |
| | 未读 turn 接缝 | ✅ | sebas-webui `ws_test`（事件流）|
| | composer 只承诺进程能力 | ✅ | sebas-webui `agent_kinds_test`；J: workbench（agent-kinds）|
| | 会话来源可见 | ✅ | sebas-webui 内联测试 |
| | 项目视图真实工作副本上下文 | ✅ | browser `projects.spec.ts` 1.2「git branch shows, plain dir shows none」（API branch 值 + rail `.branch` 标签 + 30s TTL 过期后 reload 刷新）|
| | 原生内核会话执行 | ✅ | J: `native_agent_turn_via_router_journey`（E 级）|
| | 原生内核 gated call 审批 | ✅ | src `core_channel/tests.rs`（审批往返/fail-closed）|
| | 目录浏览器加项目 | ✅ | sebas-webui `api_endpoints_test`（browse-dirs）|
| | 无 prompt 新会话 | ✅ | J: `workbench_aggregate_journey`（占位会话）|
| | 会话归档 | ✅ | sebas-webui `api_endpoints_test`（archive 路由）|
| | 历史组即归档 | ✅ | 同上 |
| | 归档过期 | ✅ | src `archive.rs` 内联测试 |
| | 执行体可用性如实呈现 | ✅ | 前端 `workbench-composer.test.ts`（native 不可用禁用+cause、可用可选、下次 poll 恢复无需重载）；browser `first-paint.spec.ts`（native option disabled + unavailable + provider label 诚实降级）|
| | 原生内核模型选择 | ✅ | src `agent_backend.rs`（`dual_set_session_model_routes_native_key_and_rejects_unknown`、spawn 期模型落快照 override）；前端 `workbench-composer.test.ts` 模型下拉/切换 |
| | 会话前模型目录（backend catalog）| ✅ | 前端 `workbench-composer.test.ts`（`creation mode offers the catalog of the defaults provider before any session`、无目录时诚实标注）；webui `/api/agent-defaults` 代理（`routes.rs`）|
| permission-flow | Hook 驱动权限请求 | ✅ | `permission_flow_test`；fake-claude "perm" 场景 |
| | 三种决定结果 | ✅ | `permission_flow_test`、sebas-webui `acp_permission_roundtrip_test` |
| | allowlist 命中自动批准 | ✅ | `permission_flow_test` |
| | allowlist 作用域与生命周期 | ✅ | `permission_flow_test` |
| | 迟到点击处理 | ✅ | src `core_channel/tests.rs`（typed rejection）|
| | 无应答者 fail-closed | ✅ | src `core_channel/tests.rs`（fail-closed）；detached 审批闭环 browser `approval-detached.spec.ts`（cover-core-channel-test-gaps B1.2，原缺口 #1 已收口）|

### ④ 项目管理（核心）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| project-session-actions | 目录选择器加项目 | ✅ | J: `projects_session_journey` |
| | 无 prompt 新会话 | ✅ | J: `workbench_aggregate_journey` |
| | 会话归档 | ✅ | sebas-webui `api_endpoints_test` |
| | 历史组即归档 | ✅ | 同上 |
| | 归档过期 | ✅ | src `archive.rs` 内联测试 |
| state-store (projects 面) | DB 位置与单写者 | ✅ | sebas-router state_store 测试；J: lifecycle（迁移日志）|
| | schema 版本与自动迁移 | ✅ | 同上（migration 0→1 日志）|
| | 迁移前备份 | ✅ | 同上（backup 文件）|
| | 通道状态方法 | ✅ | src `core_channel/tests.rs` |
| | 变更持久性 | ✅ | state_store 测试 |
| | store 不可用诚实降级 | ✅ | state_store 测试 |
| | 损坏 store 不静默重置 | ✅ | state_store 测试 |
| webui (projects 面) | 项目注册降级如实提示 | ✅ | sebas-webui `session_endpoints_test`（`projects_add_degraded_when_core_unreachable`：201 + `degraded.cause`；正常路径无标记）；browser `deployment.spec.ts`（加项目降级提示，harden-core-channel-deployment）|

> ④ 的「webui (projects 面)」按主 spec 界定收 webui capability 下直接归属项目管理面
> 的 requirement（本期仅「项目注册降级如实提示」一条）；projects 的增删/排序等其余
> webui 行为经 project-session-actions 与 agent-workbench 的同名 requirement 命中，
> 不重复计分。

### ⑤ 通道与监督（核心，raise-core-coverage-to-90 新增簇）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| core-session-channel | Core 是唯一会话权威 | ✅ | src `core_channel/tests.rs`（`backend_methods_reach_the_right_handlers`、`client_converges_after_server_restart`：核心值权威、客户端收敛）|
| | 通道传输与鉴权 | ✅ | 同上（`missing_handshake_closes_connection_without_response`、`wrong_and_empty_secrets_are_rejected`、`cross_uid_rejected_live_process` 真实 fork+setuid 跨 uid 拒绝，`#[ignore]` root 实测通过；stale socket 回收：同路径重启用例 + E `no_secret_assembly`）；harden：密钥文件 0600 |
| | 会话观察方法 | ✅ | `subscription_delivers_every_mutation_after_the_snapshot`（snapshot 先行）、`lagging_subscriber_is_disconnected_and_can_resnapshot`；快照含执行体/模型（`create_placeholder_wires_a_zero_turn_session`）|
| | 会话驱动方法 | ✅ | `backend_methods_reach_the_right_handlers`、`create_placeholder_wires_a_zero_turn_session`、`cancel_rejects_unknown_and_accepts_live_session`、`ensure_message_attachments_are_validated`（缺失附件 typed rejection + 合法附件 Ok）；E `session_round_trip_via_webui_http` |
| | 回合内容获取 | ✅ | `backend_methods_reach_the_right_handlers`（turns 增量位置语义）；`full_e2e_test` 回合内容回读 |
| | 核心不可达诚实降级 | ✅ | `unreachable_causes_are_distinct`、`client_converges_after_server_restart`；E `reachability_flips_across_core_restart`、`secret_rotation_self_heal_across_core_restart`；webui 全局横幅 + browser `deployment.spec.ts`（横幅 cause/composer 门禁/恢复消隐）|
| | 协议使用中性会话键 | ✅ | `full_e2e_test`（ChannelKey 语义）；`core_channel/protocol.rs` 内联测试 |
| | 审批请求全执行体外显 | ✅ | `acp_permission_request_streams_and_answer_routes_back`；`permission_flow_test`；browser `approval-detached.spec.ts`（detached 审批 allow/deny，cover B1.2）|
| | Spawn backend 提示校验 | ✅ | src `agent_backend.rs`（`unknown_backend_hint_rejects_without_session`、`native_missing_credentials_rejection_names_the_backend`、缺省=ACP）|
| | 通道上的 gated-call 审批 | ✅ | src `core_channel/tests.rs`（审批往返/无应答 fail-closed/unknown rid typed rejection）；`permission_flow_test` |
| | 通道上的会话模型选择 | ✅ | src `agent_backend.rs`（native key 分发 + unknown 拒绝 + override 落快照）；E/browser `models.spec.ts`（fakeacp set_session_model happy-path + 未知模型 typed rejection，cover B2.2）|
| | 无注入密钥的通道自动武装（harden）| ✅ | `auto_arm_without_env_writes_secret_file_and_completes_handshake`、`auto_arm_with_env_uses_env_value_and_writes_matching_file`；E `no_secret_assembly_end_to_end`（0600 + 会话往返）|
| | 通道客户端密钥文件发现（harden）| ✅ | `client_discovers_secret_from_file_and_heals_key_rotation`、`discovery_both_missing_warns_once_and_uses_empty`；E `secret_rotation_self_heal_across_core_restart` |
| | bind 失败即硬启动失败（harden）| ✅ | `arm_fails_hard_when_socket_path_is_taken_by_live_listener`；supervisor `bind_failed_exit_code_marks_degraded` |
| | 状态库通道面（cover）| ✅ | `tests/state_channel_contract_test.rs`（StateSnapshot/StateMutation/StateMutationOk/Rejected 不静默吞错/StateSubscribe 订阅后 mutation 帧；fake engine 注入）；`state_subscription_serves_snapshot_frame_without_engine` |
| | reachability 三态区分启动失败（cover）| ✅ | `reachability_startup_failed_with_env_file`、`reachability_startup_failed_fallback`、`reachability_auth_rejected_after_handshake`、`reachability_disconnected_after_connected`（`kind`= startup_failed/auth_rejected/disconnected + 闩锁 enrich）；E `startup_failure_*` 75 契约 |
| | ensure_message IM 投递语义（cover）| ✅ | `ensure_message_unknown_key_auto_creates`、`ensure_message_dormant_resumes`、`message_unknown_key_rejected` |
| watchdog | Core 子进程监督 | ✅ | supervisor 单测（`spawn_failure_retries`、`spawn_failure_hits_limit_and_enters_failed_startup`、`spawn_failure_limit_is_configurable`、`ready_resets_the_spawn_failure_counter`、`early_fatal_before_ready_counts_toward_the_limit`、`post_ready_crash_never_enters_failed_startup`，fake spawner 直驱监督循环）；E `startup_failure_core/run_exits_75_with_summary`、`watchdog_supervised_core_recovery`（spawn-fail 进程级注入面转豁免，见豁免清单）|
| | 崩溃退避 | ✅ | supervisor `crash_policy_counters_and_cool_down`、`over_limit_cools_down_then_keeps_supervising`、`unexpected_exit_restarts_with_backoff`；E `watchdog_supervised_core_recovery`（SIGKILL → 1s 退避重启新 pid）|
| | 新二进制自动回滚 | ✅ | supervisor `unready_after_upgrade_invokes_hook_without_crash_count`；src `watchdog.rs`（`rollback_to_previous_restores_previous_version`、`rollback_without_backup_is_err`；hook 失败分支走同一条已测 `enter_failed_startup`→75 布线）|
| | 控制 RPC 传输与鉴权 | ✅ | `control_rpc.rs`（`forged_system_actor_rejected`、missing/invalid secret、unsupported version、`socket_rejects_wrong_secret`）|
| | 控制请求面 | ✅ | `control_rpc.rs`（Status/EventsSince/Update/Rollback/RestartCore/ServiceSet/ServiceRestart 往返；core 服务命令拒绝并指向 RestartCore；feishu confirm token 可赎回）|
| | 升级执行 | ✅ | `executor.rs`（成功/dry-run/失败落账/锁不死锁/panic 释放锁）；`updater.rs`（dev/release 超时区分、可配置、不为零）；`upgrade_dev_test`（dev 升级全链，`#[ignore]` 段含真实安装+回滚）|
| | 回滚执行 | ✅ | `executor.rs::rollback_reaches_the_runner_as_a_rollback_plan`；src `watchdog.rs` rollback 单测；`upgrade_dev_test::real_dev_install_then_reinstall_then_rollback` |
| | 事件时间线 | ✅ | `events.rs`（`timeline_evicts_old_events`、cursor、cancel、timeout、幂等）|
| | 危险操作确认 | ✅ | `confirmation.rs`（replay/cross-user/expired/changed-params 拒绝、并发确认仅一次）；`control_rpc.rs` feishu token 赎回 |
| | 服务生命周期 | ✅ | `services.rs`（persist 跨重启、config off → disabled、desired 文件读写）；`control_rpc.rs`（service_set/restart 生效与 core 拒绝）；E `watchdog_supervised_core_recovery`（core 崩溃重启期间 webui 不重启并恢复可达）|
| | 受管服务表 | ✅ | `services.rs::service_name_roundtrip`、`service_from_str_knows_all_rpc_service_names`（im 命名约定）；`service_status_*` 真实观测态 |
| | 裸 core 降级模式 | ✅ | src `main.rs`（`friendly_rpc_error_socket_not_found_mentions_running_watchdog`、connection-refused、permission-denied 三态可操作报错）；src `run.rs` 裸启动 warn；`upgrade_dev_test`（一次性 update 可用）|
| | ready 蕴含通道已武装（harden）| ✅ | core 侧 bind 先于 ready（`arm_fails_hard_when_socket_path_is_taken_by_live_listener` + supervisor `bind_failed_exit_code_marks_degraded`：bind 失败即退出、从不发 ready）；E `no_secret_assembly_end_to_end`/`watchdog_supervised_core_recovery`（监督形态收到 ready 后 socket 即可连接）|

### 其余能力（非核心：矩阵可见，不设数字门槛）

| 能力 | 状态 | 证据 / 缺口 / 豁免 |
|---|---|---|
| acp-driver | ✅ | `full_e2e_test`、`pump_unit_test`、`continue_session_test`、sebas-acp resume/timeout 测试（注：`kill_reaps_child_process` 在 Windows 既有失败，与本套件无关）|
| acp-session-mapping | ✅ | 见核心簇① |
| acp-model-selection | ✅ | 见核心簇② |
| agent-bench | ✅ | sebas-agent bench 内联测试；🚫 真实模型跑分豁免 |
| agent-core | ✅ | sebas-agent 91 项内联测试（policy/tools/turn loop/budgets/streaming）|
| agent-driver | ✅ | sebas-webui `agent_kinds_test`、src `agent_backend` 内联测试 |
| channels | ✅ | sebas-channels crate 测试 |
| cli-service | ✅ | `config_test`、`config_env_test`、`daemon_path_repro_test`、src `cli` 内联测试；E: startup_failure（fail-fast-on-startup-errors：garbage config → 退出码 75 + stderr 末行 `startup-failure:` 摘要 + `SEBAS_STARTUP_ERROR_FILE`，core 与 run 双形态，`testsuite_e2e_test`）|
| core-session-channel | ✅ | 见核心簇⑤ |
| dispatch-commands | ✅ | sebas-dispatch `commands_test.rs`（解析/路由）、`control_commands_test.rs`（控制命令 → watchdog Out 事件、bare confirm 用法回执、core 命令不路由）|
| feishu-bridge | 🚫+✅ | 真实 WS/HTTP 传输豁免（需真实凭据）；进程内注入面 ✅（`feishu_native_webui_test`、ws_loop 内联测试）|
| feishu-cards | ✅+🚫 | 卡模型/流式节流/轮转 ✅（`card_stream_e2e_test`、sebas-feishu 内联）；飞书端渲染 🚫 豁免 |
| feishu-option | ✅ | `feishu_native_webui_test`、config 测试 |
| feishu-reactions | ✅ | src `reactions.rs` 内联测试 |
| im-service | ✅ | sebas-im 内联测试（`reactions.rs` 等）；`im_cmd.rs` 控制命令直发 control RPC（失败回可操作错误）；`feishu_native_webui_test` 进程内注入面 |
| router-admin-api | ✅ | `admin_test`；J: provider_governance（/admin/stats）|
| router-auth-rate-limit | ✅ | `auth_test`、`rate_limit_test`；J: `router_downstream_auth_journey` |
| router-core | ✅ | `proxy_smoke_test`、`contract_test`、`debug_provider_test`、`failure_test`；E: router debug |
| router-metrics | ✅ | sebas-router `metrics` 测试；J: /admin/stats 200 |
| router-model-aliases | ✅ | 见核心簇② |
| opencode-agent | 🚫 | 需真实 opencode CLI（模拟桩不可用）；driver 抽象由 agent-driver 测试覆盖 |
| permission-flow | ✅ | 见核心簇③ |
| project-session-actions | ✅ | 见核心簇④ |
| provider-management | ✅ | 见核心簇② |
| replay-debug | ✅ | `record_test`、`replay_test`（既有测试命中；record/replay 独立旅程待补，⚠️ 非阻塞）|
| router-commands | ✅ | sebas-router 命令内联测试 |
| session-lifecycle | ✅ | 见核心簇① |
| session-persistence | ✅ | 见核心簇① |
| state-store | ✅ | 见核心簇④ |
| testsuite-acceptance | ✅ | 套件本体：`invoke testsuite-acceptance` 六旅程（`tests/testsuite_acceptance_test.rs`，`#[ignore]` 不进默认 cargo test、失败保留沙箱、`--case` 单跑）；达标复核即本文件统计段 |
| testsuite-process-e2e | ✅ | 套件约定（沙箱隔离/显式超时/失败保留现场/平台门控）由 `tests/testsuite_e2e_test.rs` + `tests/testsuite_acceptance_test.rs` 落地并被其用例遵循；E: 全部 e2e 用例 |
| watchdog | ✅ | 见核心簇⑤ |
| webui | ✅ | sebas-webui 全套端点测试（harden-core-channel-deployment：projects 注册降级标记 `degraded.cause`/正常路径无标记）；E: detached 双进程启动/健康/重连（`testsuite_e2e_test`）；浏览器级 UI 旅程 ✅（`testsuite-webui-browser`，含 web_spawn 失败内显 + deployment 部署韧性旅程，见下）|

### testsuite-webui-browser（非核心：浏览器级 UI 旅程，Playwright）

> 入口 `invoke testsuite-webui`（`tests/testsuite-webui/`，独立 pnpm 包）；后端为一次性沙箱
> （`sebas core --router --debug --webui` + fake-claude 桩），chromium headless。detached 双进程
> 形态（core + 独立 `sebas webui`、无 `SEBAS_CORE_SECRET`、自动装配 + 密钥文件发现）由
> `--case deployment`（`playwright.detached.config.ts`，端口 9897）承担；可复用双进程 fixture：
> `tests/helpers/detached.ts`（stopCore/startCore/isCoreAlive/waitForCoreReachability）。

树形账本（converge-webui-e2e-tree）：大功能 = spec 顶层 `test.describe`（对应
requirement），子功能 = 二层 `test.describe`，一行 = 一条用例；锚点格式
`<requirement>「<scenario>」`，与 `openspec/specs/testsuite-webui-browser/spec.md`
的 scenario 名逐字对应。

| 大功能 | 子功能 | 用例名 | spec 文件 | 锚点 |
|---|---|---|---|---|
| 工作台首屏 | 首屏结构与 reachability | renders rail, composer and honest reachability | `first-paint.spec.ts` | 工作台、项目与会话管理面旅程「首屏与 reachability」 |
| 鉴权闭环 | 深链重定向 | deep link under auth redirects to the login page | `auth.spec.ts` | 鉴权与访问旅程「登录闭环」 |
| 鉴权闭环 | 登录与登出 | session cookie survives reload until logout, then the gate returns | `auth.spec.ts` | 鉴权与访问旅程「登录闭环」 |
| 鉴权闭环 | 登录与登出 | wrong credentials rejected, admin/admin enters, logout returns | `auth.spec.ts` | 鉴权与访问旅程「登录闭环」 |
| agent 对话覆盖 | 首回合往返与重载恢复 | composer submit → reply → done → reload restores | `session-roundtrip.spec.ts` | agent 对话覆盖「首回合往返」、会话核心旅程「重载恢复」 |
| agent 对话覆盖 | 流式分批 | chunks arrive in batches and the turn converges to done | `streaming.spec.ts` | agent 对话覆盖「流式分批渲染」 |
| agent 对话覆盖 | 多轮连续 | 4.1 two consecutive rounds append in order and survive reload | `dialog.spec.ts` | agent 对话覆盖「同会话多轮连续」 |
| agent 对话覆盖 | 输入守卫 | 4.2 empty and blank input creates no turn, session stays usable | `dialog.spec.ts` | agent 对话覆盖「composer 输入守卫」 |
| agent 对话覆盖 | 输入守卫 | 4.2 special-char long text round-trips without loss or console errors | `dialog.spec.ts` | agent 对话覆盖「composer 输入守卫」 |
| agent 对话覆盖 | 错误诚实呈现 | refuse — non-terminal: session survives, next message works | `errors.spec.ts` | 会话核心旅程「错误呈现」 |
| agent 对话覆盖 | 错误诚实呈现 | crash — honest death: not-found presentation, no fake success | `errors.spec.ts` | 会话核心旅程「错误呈现」 |
| agent 对话覆盖 | spawn 失败内显 | spawn failure inline: error event in transcript, session stays as spawn-failed | `errors.spec.ts` | webui「web_spawn 失败的立即内显」（fail-fast-on-startup-errors） |
| 审批卡片旅程 | 拒绝路径 | deny path — refusal semantics, turn completes | `permission.spec.ts` | 审批卡片旅程「拒绝路径」 |
| 审批卡片旅程 | 单次允许路径 | allow-once path — allowed semantics, turn completes | `permission.spec.ts` | 审批卡片旅程「单次允许路径」 |
| 审批卡片旅程 | 会话级允许 | allow-session path — observed product gap: the follow-up call is gated again | `permission.spec.ts` | 审批卡片旅程「会话级允许」 |
| 项目管理覆盖 | 增删 | add via project dialog appears in rail; removed project disappears | `projects.spec.ts` | 项目管理覆盖「项目增删」 |
| 项目管理覆盖 | 异常拒绝 | 1.1 illegal path 400, duplicate 409, removal persists across reload | `projects.spec.ts` | 项目管理覆盖「项目异常拒绝」 |
| 项目管理覆盖 | 排序与持久化 | 1.2 reorder persists; git branch shows, plain dir shows none | `projects.spec.ts` | 项目管理覆盖「项目排序与分支呈现」 |
| 项目管理覆盖 | 选择器交互 | P1 tree expand, click-select fills path, submit lands in rail | `projects.spec.ts` | 项目管理覆盖「folder-picker 树展开点选回填」 |
| 项目管理覆盖 | 选择器交互 | P2 empty path disables submit; missing path errors inline, dialog stays | `projects.spec.ts` | 项目管理覆盖「选择器空路径与非法路径」 |
| 会话管理 | close 与 archive | close removes the session from the active list | `session-mgmt.spec.ts` | 会话管理覆盖「close 与 archive」 |
| 会话管理 | close 与 archive | archive hides from list, shows in History | `session-mgmt.spec.ts` | 会话管理覆盖「close 与 archive」 |
| 会话管理 | 深链与退役路径 | deep link renders the session via SPA fallback; /settings redirects to / | `session-mgmt.spec.ts` | 会话管理覆盖「深链与退役路径」 |
| 会话管理 | 模型面诚实缺省 | model honest absence — no selector, switch attempt keeps model absent | `session-mgmt.spec.ts` | 工作台、项目与会话管理面旅程「模型面诚实缺省」 |
| 会话管理 | 多会话切换 | 2.1 dual-session switch (rail + deep-link) does not crosstalk | `sessions.spec.ts` | 会话管理覆盖「多会话切换」 |
| 会话管理 | archive 写保护 | 2.2 archive → 400 write-protection → restore unhides honestly | `sessions.spec.ts` | 会话管理覆盖「archive 写保护与 restore 诚实语义」 |
| 模型管理覆盖 | 无模型诚实缺省 | 3.2 set_model on a model-less session fails terminally and honestly | `models.spec.ts` | 模型管理覆盖「无模型会话 set_model 诚实拒绝」 |
| 模型管理覆盖 | settings provider 只读 | 3.2 settings provider list matches API, zero probe traffic | `models.spec.ts` | 模型管理覆盖「settings provider 只读」 |
| 设置面 ¹ | 只读呈现 | S1 services rows match /api/admin/services truth (response-driven) | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S2 about table matches /api/about truth | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S3 env table renders placeholder semantics | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S6 bare core degrades: no-adapter banner, no rows, restart disabled | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 写降级 | S4 defaults read parity; sandbox write fails honestly | `settings.spec.ts` | 设置面写操作诚实降级覆盖「写降级失败外显且状态不变」¹ |
| 设置面 ¹ | 写降级 | S5a create/edit mutations: client validation + honest 503 | `settings.spec.ts` | 模型管理覆盖「settings provider 只读」 |
| 设置面 ¹ | 写降级 | S5b delete/probe mutations fail honestly, list unchanged | `settings.spec.ts` | 模型管理覆盖「settings provider 只读」 |
| 部署韧性 | 核心通道停启 | core 停 → 横幅含 cause、composer 门禁、加项目降级提示；core 恢复 → 横幅消失 | `deployment.spec.ts` | webui「全局核心可达性横幅」「项目注册降级如实提示」（harden-core-channel-deployment，detached 双进程形态）⁵ |
| 审批卡片旅程（detached） | detached 审批闭环 | allow path — ApprovalRequested 跨进程到达 review-card，allow 后 transcript 记录允许语义 | `approval-detached.spec.ts` | webui「approval_answer end-to-end (detached topology)」（cover-core-channel-test-gaps B1.2，detached 双进程形态）⁶ |
| 审批卡片旅程（detached） | detached 审批闭环 | deny path — deny 后 transcript 记录拒绝语义，回合完成 | `approval-detached.spec.ts` | webui「approval_answer rejects unknown request_id」⁶（cover B1.2，detached） |
| 审批卡片旅程（detached） | unknown rid | POST /api/permissions/{rid}/answer with an unknown rid → 404 typed rejection | `approval-detached.spec.ts` | webui「approval_answer rejects unknown request_id」（cover B1.2，detached）⁶ |
| 模型管理覆盖 | 有模型面的正向切换（acp:fakeacp） | set_session_model happy path — POST ok-model → ModelChanged → current_model 同步 + 选择器呈现 | `models.spec.ts` | webui「set_session_model happy-path via webui」（cover-core-channel-test-gaps B2.2）⁶ |
| 模型管理覆盖 | 有模型面的正向切换（acp:fakeacp） | set_session_model rejects unknown model — typed rejection 到达但 webui 无内联错误面（observed product gap），会话存活、current_model 不变 | `models.spec.ts` | webui「set_session_model rejects unknown model」（cover B2.2）⁶ |

> Settings 语义修正（fix-settings-menu-and-services-semantics，spec 改动 `1e4a807`）：
> S1 改写为 `/api/admin/services` 响应驱动（响应即真源，不枚举具体服务）、S6 新增裸
> core 退化覆盖（横幅 + 零行 + 重启 disabled）；实施清单见
> `openspec/changes/fix-settings-menu-and-services-semantics/tasks.md`；本表 41 行 =
> 全套件 `it` 总数 41（= 主 config 34 + auth 3 + detached 4；含 S6 与 cover-channel
> 新增 5 例）。

> ⁶ cover-core-channel-test-gaps（2026-09-08）：detached 审批三例 + 模型面两例；
> detached spec（`approval-detached.spec.ts`）由 `playwright.detached.config.ts` 专属承担
> （主 config testIgnore 反向排除）；沙箱 harness 新增 `[acp.agents.fakeacp]`（generic-ACP
> 驱动 + `sebas-acp` 的 fake-acp-agent，通告 bad-model/ok-model 且拒绝 bad-model）。
> observed product gap：generic-ACP 驱动不产 `Finished`（会话停留 working）且 agent 级
> non-terminal Error 在 webui 无渲染面——用例按可观测契约断言（无假成功），缺口待后续。

> cover-core-channel-test-gaps 实施账（2026-09-08）：A 批 commit `049672e`（reachability
> 三态 + kind、State 三件 contract、ensure_message/cross-uid；`tests/state_channel_contract_test.rs`）；
> B 批 commit `24ec929`（detached 审批旅程 `approval-detached.spec.ts` + 模型面两例
> `models.spec.ts` + 沙箱 `[acp.agents.fakeacp]`）；实施清单
> `openspec/changes/cover-core-channel-test-gaps/tasks.md`。全套件 `invoke testsuite-webui`
> 3 连绿（exit 0：34 + 3 + 4，detached 首例首试由 retry 吸收）。

> ¹ 设置面（S1–S4, S6）的锚点指向尚未归档的 change `expand-webui-e2e-settings` 的 delta
> scenario（该 change 尚未同步进主 spec，主 spec 暂无设置面 requirement）；S5a/S5b 的
> provider 写 503 语义已由主 spec `模型管理覆盖`「settings provider 只读」吸收，故锚到
> 主 spec。会话核心旅程的「首回合往返」「流式分批渲染」与 `agent 对话覆盖` 同名
> scenario 文本相同（工作台旅程的「项目增删」「close 与 archive」「深链与退役路径」
> 同理），经上表 tree 侧行同源锚定。harness 级 scenario（沙箱装配：启动即隔离 /
> 成功退出清理 / 失败保留现场；一键入口：一键运行 / 单旅程过滤 / 渐进补用例不改入口 /
> 顶层裸 test 拒绝）由入口机制承担、鉴权「免登录直达」由全部主 config 用例隐式承担，
> 均不设单用例行。

原「浏览器级 UI 渲染」豁免条目：workbench 首屏、审批卡片操作、登录页闭环等
浏览器面由本套件覆盖（豁免范围收窄为「飞书端卡片渲染」等其余条目）。

> Settings 语义修正（fix-settings-menu-and-services-semantics）：实施 commit
> `93bdfdc`（projects 增删断言对齐 dashboard basename 渲染、解除 §4.4 blocker 后
> 全量 3 连绿 35/35）；实施清单 `openspec/changes/fix-settings-menu-and-services-semantics/tasks.md`。

## 豁免清单（cause + 替代验证）

| 面 | cause | 替代验证 |
|---|---|---|
| 飞书真实 WS/HTTP 传输 | 需真实 app 凭据，沙箱不可得 | 进程内注入级测试（router 派发/出站事件即生产意图）|
| 飞书端卡片渲染（含 provider-management「/provider 主卡布局」）| 同上 | 卡片 JSON 生成单测（`card_stream_e2e_test`、sebas-feishu 内联）|
| 浏览器级 workbench UI 渲染 | ~~簇 C 另行立项~~ 已由 testsuite-webui-browser 落地（`invoke testsuite-webui`）| `tests/testsuite-webui/` 旅程套件（Playwright + fake-claude 沙箱）|
| opencode-agent 真实代理 | 需真实 opencode CLI | AcpDriver 抽象层测试 |
| agent-bench 真实模型跑分 | 需真实凭据 | bench 断言逻辑单测 |
| watchdog「连续 3 次 spawn fail → 整体退出 75」进程级注入（raise-core-coverage-to-90 起计入豁免）| spawner 用 `current_exe()` 派生子进程，真实二进制下 spawn 系统调用无法注入失败；早期退出又全部收敛为 75（Degraded 语义）或 crash 退避（计数清零），进程级不可区分 | supervisor 单测以 fake spawner 直驱监督循环（`spawn_failure_hits_limit_and_enters_failed_startup`、`spawn_failure_limit_is_configurable`、`ready_resets_the_spawn_failure_counter`、`early_fatal_before_ready_counts_toward_the_limit`）+ 终态事件通道；E `startup_failure_core/run_exits_75_with_summary` 验证 75 退出契约（fail-fast-on-startup-errors）|

## 缺口清单（未命中且未豁免）

requirement 级残留：**0 条**（五簇复核 2026-09-08 收口）。历史缺口去向：

1. ~~detached 审批通道旅程~~（permission-flow / agent-workbench）— 已收口：
   cover-core-channel-test-gaps B1.2 交付 `approval-detached.spec.ts`（detached
   双进程 allow/deny/unknown-rid 三例，`playwright.detached.config.ts`），复用
   harden 交付的 `tests/testsuite-webui/tests/helpers/detached.ts` fixture。
2. ~~项目视图工作副本上下文~~（agent-workbench）— 已收口：browser
   `projects.spec.ts` 1.2 断言 API branch 值、rail `.branch` 标签与 TTL 刷新。
3. ~~watchdog 监督循环进程级旅程~~（watchdog）— 已收口：harden 5.3 交付
   `watchdog_supervised_core_recovery`（SIGKILL core → supervisor 重启新 pid →
   webui 恢复）；剩余「连续 3 次 spawn fail → 整体退出 75」的进程级注入面转豁免
   （真实二进制下 `current_exe()` spawn 系统调用无法注入失败，见豁免清单）。
4. replay-debug 独立旅程（replay-debug）：requirement 已由 `record_test`/
   `replay_test` 命中；端到端旅程仍待补（⚠️ 旅程级注记，非核心、非阻塞）。

## 实施期发现（只记录不顺手修，见 design Non-goals）

1. **native 会话状态卡在 Queued**：native 回合完成（turn summary 已写、模型调用已完成），但 `src/native_router_bridge.rs` 从不设置 phase=DONE，workbench 状态恒为 "Queued"（models.rs derive：active+"" → Queued）。建议立项修复后，`native_agent_turn_via_router_journey` 的断言可升级为 status_slug=done。
2. **`run --router` 忽略 `SEBAS_ROUTER_LISTEN`**（run.rs:87 写死 127.0.0.1:0）：detached 形态下 `SEBAS_AGENT_ROUTER_URL` 无法预注入（router 地址只能事后从日志读）。native 走 `SEBAS_AGENT_PROVIDER_BASE_URL` 直连路径作为替代（本套件已覆盖）。
3. **会话状态落盘仅在优雅退出**：硬杀（TerminateProcess）不产生状态转储；Windows 无便携优雅信号，故重启恢复段 unix 门控。
4. **路由状态已入 SQLite**：`[router] state_file`（sessions.json）不再是重启恢复的活性来源，state store DB（sebas.db）承担持久化——矩阵断言已按此更新。
5. **restore 不复活会话**（二期浏览器旅程发现）：`archive` 先 close（mapping + transcript 丢弃），`restore` 只删归档条目；恢复后详情页如实 404（`sessions.spec.ts` 2.2 已按此诚实语义断言）。与 project-session-actions「History 点击恢复可写」条文不一致，产品语义变更另立项。
6. **无模型会话 set_model 是终态杀伤**（二期浏览器旅程发现）：webui 只投递（200 ok），Claude 驱动以终态 Error 应答 SetModel，会话被拆除（`models.spec.ts` 3.2 已按终态诚实断言）。正向模型切换需驱动模型面（configOptions 透出 + ModelChanged），待另立项。
7. **sebas-agent 内联测试环境依赖**（fail-fast-on-startup-errors 实施期发现）：`cargo test -p sebas-agent` 在 Windows Git Bash 环境下 17 项失败（tools::bash / tools::search / loop_ 等，依赖 POSIX shell 语义）；清理树同样失败（71 passed / 17 failed 前后一致），与 change 无关，属环境缺口。同批发现 `card_stream_e2e_test` 在并行负载下偶发 5s deadline 超时（隔离运行稳定通过）。

## 变更账本（core-set 增删与能力面变更说明）

| 日期 | change | commit | 说明 |
|---|---|---|---|
| 2026-09-08 | raise-core-coverage-to-90 | `feat/raise-core-coverage-to-90`（基数：fail-fast `5eb1d85`、harden `52847f1`、cover-channel `f14767e`） | 核心集门槛 80%→90% 且扩为五簇（新增 ⑤ 通道与监督 = core-session-channel + watchdog，界定写死进主 spec「核心功能集界定」）；五簇 requirement 级全量重数入账（100/100 = 100%，豁免 1 条不计分母）；三期变更新用例证据归行（fail-fast → watchdog/webui/testsuite 行；harden → ⑤/webui/④/testsuite 行；cover-channel → ⑤/webui 行）；缺口清单收口（detached 审批、projects_branch、watchdog 监督旅程均已有交付命中；spawn-fail 进程级注入转豁免）；复核 grep 清单落 tasks 1.1。实施清单为本期 `tasks.md`。 |
| 2026-09-09 | harden-core-channel-deployment | `feat/harden-core-channel-deployment`（694c2a5 起） | 核心通道部署加固：core 无条件自动装配（生成密钥写 `<config dir>/core.secret` 0600，env 优先）、客户端每次连接 env→文件发现（轮换自愈）、ready 后移至 bind 成功后（bind 失败→75）、im/router/webui 订阅侧共享 resolver、webui 全局核心不可达横幅 + 项目注册降级标记/提示。证据：E `no_secret_assembly_end_to_end` / `secret_rotation_self_heal_across_core_restart` / `watchdog_supervised_core_recovery`（`testsuite_e2e_test`，3 连绿 11 passed ×3）；browser `deployment.spec.ts` 部署韧性旅程（detached 双进程，3 连绿）；`testsuite-webui/tests/helpers/detached.ts` 双进程 fixture（交付物，cover-B 复用）。实施清单为本期 `tasks.md`。 |
| 2026-09-07 | fail-fast-on-startup-errors | `feat/fail-fast-startup-errors`（2e4d952 起，5.3 收尾 commit 为该分支末端） | 启动失败 fail-fast 规约：生命周期子命令启动失败统一退出码 75 + stderr 末行 `startup-failure:` 摘要 + `SEBAS_STARTUP_ERROR_FILE` 覆盖写；watchdog `[watchdog] max_spawn_failures`（默认 3）→ 服务终态 `failed-startup` + watchdog 整体退出 75；rollback 失败不再 silently continue；`sebas ctl status` 新增 `startup_failure` 摘要、`/api/summary` 的 `reachability.cause` 富化；webui web_spawn 失败 inline 进 transcript（spawn-failed 状态 + error 事件 + 相邻同类合并计数）。证据：E `startup_failure_core/run`（`testsuite_e2e_test`）、browser「spawn 失败内显」（`errors.spec.ts`）、supervisor 终态策略/`SpawnFailurePolicy` 单测。实施清单为本期 `tasks.md`。 |
