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

## 核心功能集统计（达标复核 2026-09-11，五簇 90% 口径）

复核基数 = main 最新（workbench-agent-wire-fix 归档 `61feb1a`）+ 五个 change 归档后的
工作树 specs（make-core-own-provider-data、add-fetch-models、redesign-provider-models-settings、
workbench-turn-queue、workbench-conversation-view；specs 同步随归档落在工作树）。复核为
requirement 级全量重数（`grep -c '^### Requirement:'` 对应 capability 目录，可重跑）；
豁免不计分母；命中口径不变：任一测试层（验收旅程 J、进程级 e2e E、集成/单元/前端
单测、浏览器旅程）完整命中该 requirement 即计入。router-admin-api 的 4 条 REMOVED
（Provider CRUD endpoints、Model alias CRUD endpoints、Model probe endpoint、
Write-then-apply semantics）退役出账；webui 只计 projects 面（界定不变）。

| 核心簇 | requirement 数（计分分母） | 命中 | 命中率 | 豁免 | 套件内旅程 |
|---|---|---|---|---|---|
| ① 会话管理 | 14 | 14 | 100% | 0 | `session_lifecycle_journey` |
| ② models 管理 | 22 | 22 | 100% | 1 | `provider_governance_journey`、`native_agent_turn_via_router_journey` |
| ③ agent workbench 相关 | 33 | 33 | 100% | 0 | `workbench_aggregate_journey`、`projects_session_journey` |
| ④ 项目管理 | 17 | 17 | 100% | 0 | `projects_session_journey`；browser `deployment.spec.ts`（降级注册） |
| ⑤ 通道与监督 | 31 | 31 | 100% | 0 | E: `no_secret_assembly_end_to_end`、`secret_rotation_self_heal_across_core_restart`、`watchdog_supervised_core_recovery` |
| **核心合计** | **117** | **117** | **100%** | **1** | 每簇 ≥1 条 ✓ |

上一次复核（2026-09-08，100/100）之后本期重数差异来源（+17，100 → 117）：

- **五 change 归档新增 requirement 入账 +7**：① +1「Pending submissions are
  observable and manageable」；② +2「Core owns provider and model data」「Model
  entries carry capability tags」；③ +3「Pending submissions stack above the
  composer」「Workbench renders the focused session as a conversation」「Workbench
  is the single conversation surface」；⑤ +1「Model list fetch over the channel」。
- **上期基数后归档、本期补入账 +10**：③ agent-workbench +6（审计归档 `982b4cc`
  的 Rail project removal entry、Rail session close entry、Placeholder session is
  immediately writable；workbench-agent-wire-fix `bad057e` 的 Session agent binding
  is immutable、Composer submissions always deliver、Project-level default agent）；
  ④ +4（`982b4cc` 的 project-session-actions 三条 rail 面同名 requirement +
  retire-session-persistence-record `c01211a` 给 state-store 增「Runtime state
  boundaries for persisted session state」）。
- **router-admin-api 非核心面退役 −4**（REMOVED，不进核心分母，见非核心表行）。

本期两处真实缺口，均已补测收口：router 订阅 core provider 数据的进程级闭环此前
在任何测试层都无覆盖（`Configuration source`「card-edited provider reaches
router」/`External change hot reload`「card edit hot-applies」/Core owns「router
reads a core change without a restart」共同指向），本期补 E
`core_owned_provider_reaches_router_without_restart`（watchdog 形态 router 子进程
订阅通道 → webui BFF 写库 → 不重启可路由、不写 provider 文件）；「Project-level
default agent」（wire-fix 引入，写路径/记忆/预选全无测试），本期补
`session_endpoints_test::project_default_agent_follows_last_use`（API 写路径）+
前端 composer 预选单测。收口后 requirement 级「未命中且未豁免」残留 0 条。

全量 35 个能力目录、339 条 requirement（`grep -c '^### Requirement:'` 汇总，可重跑）。
豁免 23 条（飞书真实传输/端上卡片渲染、opencode CLI、真实模型跑分、watchdog
spawn-fail 进程级注入，见豁免清单）；requirement 级「未命中且未豁免」残留 0 条，
唯一保留的旅程级注记为 replay-debug 独立旅程（requirement 已由既有测试命中，
见其行内 ⚠️ 注记，非阻塞）。非核心簇维持「可见不设门槛」。

## 能力矩阵

### ① 会话管理（核心）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| session-lifecycle | 身份按会话/线程 | ✅ | `full_e2e_test`（ChannelKey 语义）|
| | 首条消息懒 spawn | ✅ | `full_e2e_test`；J: `session_lifecycle_journey`；`state_test::dormant_first_text_claims_resume_then_queues`（映射激活排空队列）|
| | 双 spawn 竞争保护 | ✅ | `spawn_race_test`（`second_text_during_spawn_is_queued_not_spawned`、`rapid_double_new_emits_single_spawn`、`pending_queue_capped_at_16`）；`state_test::overflow_rejects`（溢出丢最新）|
| | 会话并行 spawn 活性（session-parallel-liveness-and-unread-polish 新增）| ✅ | sebas（core 层）`dispatch::tests::web_spawn_instruction_is_not_blocked_by_a_stalled_handshake`（出站泵不再同步 await 握手：WebSpawn/SpawnResume 投递独立任务，第一会话僵住时第二 spawn 指令照发不阻塞）；spawn 失败 wire 透传 `server::workspace_root_tests::spawn_failed_session_row_and_detail_carry_the_reason`（失败行/详情带 status_slug=failed + spawn_failure_reason 原文）；E `two_sessions_spawn_and_turn_concurrently`（testsuite_e2e_test，--ignored，fake-claude 双会话握手重叠）|
| | Dormant 懒恢复 | ✅ | `restart_recovery_test`；J: lifecycle（unix 段）|
| | 终态错误拆除 | ✅ | `error_test`、sebas-router 内联测试；`turn_card_test::terminal_error_clears/abandons_queued_turns`（终态清队列）|
| | 正常回合完成保活 | ✅ | `full_e2e_test`；J: lifecycle |
| | 流式背压排队 | ✅ | `state_test`（fifo/priority 队列）、`turn_card_test::btw_command_queues_with_priority_ahead_of_existing_fifo`（/btw 插队）；E `turn_queue_timing_and_dropped_accounting`（忙中提交不切开在跑回合 + 排队回合自动开轮 + 丢弃记账）|
| | 并发会话容量上限 | ✅ | sebas-router 内联测试 |
| | 重启恢复与损坏容忍 | ✅ | `restart_recovery_test`、`state_persistence_test`；J: lifecycle |
| | Pending submissions 可观察可管理（turn-queue 新增）| ✅ | src `core_channel/tests.rs::pending_submissions_visible_and_manageable_over_the_channel`（快照可见 + 移除/已开始 typed rejection）；`session_endpoints_test::session_payloads_carry_pending_submissions_in_delivery_order`、`pending_remove_and_move_endpoints`；前端 `pending-stack.test.ts`（顺序/priority/乐观移除/AlreadyStarted 静默和解/拖拽越组拒绝）|
| session-persistence | 默认选择语义 | ✅ | `state_persistence_test` |
| | 运行态不入该库 | ✅ | `state_persistence_test` |
| acp-session-mapping | 路由 id ↔ ACP id 映射 | ✅ | `acp_session_mapping_test` |
| | 缺映射诚实回退 | ✅ | `acp_session_mapping_test` |

### ② models 管理（核心）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| acp-model-selection | 会话模型清单暴露 | ✅ | sebas-acp `acp_resume_test::spawn_outcome_carries_model_info_from_config_options`（ConfigOptions 进快照）；claude 驱动别名模型面（workbench-composer-input-polish 2.1–2.3）：`claude_model_selection.rs`（内置表 spawn 拼装/配置覆盖替换/帧观察 ModelChanged/set_model 控制链 + 帧纠偏）、`tests/config_test.rs`（models 键缺省/覆盖/空表回退）；E `claude_model_surface_reaches_snapshot_and_switch_round_trips`（别名表随快照可达 + 切换链路 200 + 帧确认）；前端 `workbench-composer.test.ts`（dropdown 喂清单/无 option 无 dropdown/只读 current 芯片/会话清单与创建目录独立 D8）；browser `models.spec.ts`（set_session_model happy-path + claude 别名表切换旅程）|
| | set_config_option 换模型 | ✅ | add-acp-model-selection 测试（sebas-acp）；claude 控制协议等价通道（workbench-composer-input-polish 2.3）：`claude_model_selection.rs::set_model_switches_via_control_protocol_and_frames_correct`（journal 记 set_model、乐观 ModelChanged、帧纠偏）+ E 同上 |
| | 模型选择存活于会话生命周期 | ✅ | 同上 |
| router-model-aliases | 别名实体与持久化 | ✅ | sebas-router `config` overlay 测试；state_channel_contract `alias_mutation_rejects_invalid_entry`（core 落库面）|
| | 别名解析优先级 | ✅ | 同上 |
| | 上游模型翻译 | ✅ | J: `provider_governance_journey`（my-claude→stub-model）|
| | 别名校验 | ✅ | state_channel_contract alias mutation 校验；router 侧 resolve 管线内联测试 |
| | 别名作用域 | ✅ | sebas-router 内联测试 |
| provider-management | /provider 主卡布局 | 🚫 | 飞书端卡片渲染（豁免，见豁免清单；替代：卡片 JSON 单测）|
| | 模式切换 | ✅ | sebas-webui 内联/admin 测试 |
| | Provider CRUD 表单（API 面）| ✅ | sebas-webui `gateway_bff_test::provider_mutation_semantics_preserved_over_core_store`（create/update/delete/aliases 走 core store seam）；state_channel_contract provider mutation 三态（未知字段/类型错/合法通过）；J: provider_governance |
| | 密钥脱敏 | ✅ | `provider_card.rs::probe_cards_never_carry_key_material`；`gateway_bff_test::provider_probe_fetches_models_without_persisting`（响应无 key、上游带 Bearer）；settings-modal 编辑态 |
| | 模型探测 | ✅ | `provider_card.rs` 内联（probe 按钮单 URL 选择/写回目录与默认/401 错误卡/仅 anthropic URL 隐藏入口）；`gateway_bff_test::provider_probe_unknown_404 / unreachable_503 / without_base_url_400`；state_store `preset_derived_provider_resolves_fetch_target_from_code_table` |
| | Off 模式解析 | ✅ | src 内联测试 |
| | 直连模式 env 翻译 | ✅ | src 内联测试；J: native（SEBAS_AGENT_PROVIDER_* 直连 stub）|
| | 模型旗标优先级 | ✅ | src 内联测试 |
| | Router 模式 env 翻译 | ✅ | src `agent_backend` 内联测试 |
| | Provider 错误中止 | ✅ | src 内联测试 |
| | Provider 卡片反映 store 可用性 | ✅ | sebas-router `admin_test` |
| | Preset 数据跟随代码表 | ✅ | sebas-router `config.rs` 内联（`preset_fills_all_slots_and_models_from_code_table`、`preset_explicit_url_override_errors`、`preset_alias_reuses_table_defaults`、`preset_explicit_api_key_skips_default_env`、`preset_table_carries_vision_entries`）；`gateway_bff_test::presets_served_from_code_table_via_backend`；settings-modal「untouched preset entries stay unsubmitted」|
| | 页面设置默认 provider/model | ✅ | `gateway_bff_test::router_api_defaults_reads_core_store_and_degrades_honestly`（读 core store + 诚实降级）；browser `settings.spec.ts` S4（defaults 读对账 + set-default 语义）；state_store `default_selection_wire_shape`、`delete_provider_atomically_clears_default`；composer 创建模式目录预选单测（4.3）|
| | Core owns provider and model data（core-owned 新增）| ✅ | `gateway_bff_test`（3.2 create 立即可见无重启、3.3 不可达 503 不回陈数据）；`spawn_env_store_authority_test`（store 压过 legacy 文件/墓碑语义）；sebas-router `admin_test`（retired 404 + 无通道不写文件）；E `core_owned_provider_reaches_router_without_restart`（router 订阅 core 变更无重启可路由、不写 provider 文件，2026-09-11 补）|
| | Model entries carry capability tags（redesign 新增）| ✅ | state_store `provider_models_legacy_strings_read_and_normalize_to_entries`（legacy 裸串归一）、`provider_models_entry_writes_canonical_unknown_tags_rejected`；sebas-router `models.rs::capability_tags_do_not_affect_env_mapping`、`unknown_capability_tag_is_rejected`；settings-modal 标记编辑单测；browser `models.spec.ts` 3.2（条目 + tags 持久化）|

### ③ agent workbench 相关（核心）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| agent-workbench | 项目为组织单元 | ✅ | J: `projects_session_journey`；state-store projects |
| | 项目注册表 webui 持有 | ✅ | state-store 测试；J: workbench |
| | 会话归属 | ✅ | sebas-webui `session_endpoints_test` |
| | 并发项目 | ✅ | state-store 并发测试；`session_endpoints_test::concurrent_project_sessions_run_simultaneously_and_leave_a_untouched` |
| | 未读 turn 接缝 | ✅ | sebas-webui `ws_test`（事件流）；前端 `transcript-view.test.ts`（多 chunk 未读回合计一次、seam 不切开回合、mark-all-seen 写入并消隐、msg_count 随 payload 推进共享锚）；`unread-cursor.test.ts`（rail-declutter-unread D3：seam 与徽标共用游标、首访/清缓存不冒红点、单调写）；browser `unread-badge.spec.ts`（跨层旅程：fake-claude 完整回复 → API `msg_count` 投影 +1 → 离焦会话徽标免刷新亮起「1」→ 聚焦清零且 localStorage `anchor_count` 推进到服务端当前值）|
| | 徽标与相位帧驱动（session-parallel-liveness-and-unread-polish 新增）| ✅ | `ws_rpc_contract_test::events_serialize_with_dotted_type_tag` + `all_seven_events_travel_as_notifications_with_params_verbatim`（session.created/updated 五键相位帧：status_slug/turn_engaged/msg_count/pending 每帧必带、旧 `status` 字段删除）；前端 `project-rail.test.ts`（session.updated 帧免 HTTP 刷新推进未读徽标 + 圆点翻转；未读行 accent-soft 强调与已读行可分辨）；`dashboard.test.ts`（帧同步补丁 focusedDetail、turn_engaged 真读无 slug 回退链）；`transcript-view.test.ts`（单字段 `{anchor_count}` 段锚：流式推进=聚焦推进=手动读到底同字段，旧 seen_ts 数据读为 fully-read 并被覆写）|
| | composer 只承诺进程能力 | ✅ | sebas-webui `agent_kinds_test`；J: workbench（agent-kinds）|
| | 会话来源可见 | ✅ | sebas-webui 内联测试 |
| | 项目视图真实工作副本上下文 | ✅ | browser `projects.spec.ts` 1.2（rail-declutter-unread D8 改写：分支探测链路 + 30s TTL 过期后 API 刷新仍断言；rail 不再显示分支名，可达性标记仍由探测驱动）|
| | 原生内核会话执行 | ✅ | J: `native_agent_turn_via_router_journey`（E 级）|
| | 原生内核 gated call 审批 | ✅ | src `core_channel/tests.rs`（审批往返/fail-closed）|
| | 目录浏览器加项目 | ✅ | sebas-webui `api_endpoints_test`（browse-dirs）|
| | 无 prompt 新会话（interaction-polish 修订：创建收拢进项目行「+」对话框）| ✅ | J: `workbench_aggregate_journey`（占位会话）、`creation_dialog_journey`（对话框确认 wire 面：agent+mode 落表、确认即激活、首条消息 spawn、未知 agent 4xx）；前端 `project-rail.test.ts`（+ 打开对话框/确认创建/失败留盒/取消不创建）、`new-session-dialog.test.ts`（agent 必选禁用/预选/目录两级/诚实降级）；browser `first-paint.spec.ts`、`session-roundtrip.spec.ts`（对话框旅程）|
| | 会话归档 | ✅ | sebas-webui `api_endpoints_test`（archive 路由）|
| | 历史组即归档 | ✅ | 同上 |
| | 归档过期 | ✅ | src `archive.rs` 内联测试 |
| | 执行体可用性如实呈现 | ✅ | 前端 `workbench-composer.test.ts`（native 不可用禁用+cause、可用可选、下次 poll 恢复无需重载）；browser `first-paint.spec.ts`（native option 在创建对话框内 disabled + unavailable，composer 不再承载 provider label）|
| | 原生内核模型选择 | ✅ | src `agent_backend.rs`（`dual_set_session_model_routes_native_key_and_rejects_unknown`、spawn 期模型落快照 override）；前端 `workbench-composer.test.ts` 模型下拉/切换 |
| | 会话前模型目录（backend catalog；interaction-polish 修订：目录选择移入创建对话框，loadModelCatalog 共用）| ✅ | 前端 `new-session-dialog.test.ts`（两级选择/预选/目录不可得诚实标注/不伪造 default）；`model-catalog.test.ts`（providers × models 展平、空目录不造假、loadModelCatalog 读取失败不抛）；browser `conversation.spec.ts`「the creation dialog offers provider → model from the Settings catalog」+ `models.spec.ts`「a fetched-and-saved catalog reaches the creation dialog without a restart」（编辑器抓取→保存→同页免刷新可读的设置↔工作台接缝旅程，2026-09-11 补）|
| | Rail project removal entry（审计补行；rail-declutter-unread D5 修订）| ✅ | `session_endpoints_test::projects_remove_project / projects_remove_unknown_returns_404 / projects_remove_blocked_while_live_sessions_exist`（有存活会话 409 typed rejection 带会话数、无会话放行）；browser `projects.spec.ts`「removed project disappears」+「… menu removal: blocked while live sessions exist」（… 菜单 → 移除弹窗就地预检 + 后端拒绝一致）；前端 `project-rail.test.ts`（预检文案/旧「迁移 Inbox」文案废除/拒绝内联）|
| | Rail session close entry（turn-queue 修订；rail-declutter-unread 3.2 修订）| ✅ | 前端 `project-rail.test.ts`（close 移入会话行 `…` 菜单：inactive 直删/active 先确认/关闭 danger、点名丢弃 pending 条数/无 pending 省略、无行内直删按钮）；`dashboard.test.ts`（close 确认点名丢弃数）；`session_endpoints_test::close_response_names_discarded_pending_count`；browser `session-mgmt.spec.ts`、`pending-stack.spec.ts`（archive names the loss，经 `…` 菜单归档确认——workbench-live-conversation-flow 4.2 归档即关闭，点名语义由归档确认框承接）|
| | Placeholder session is immediately writable（审计补行）| ✅ | `spawn_race_test`（占位首条消息必须 SpawnNew、0-turn 占位同样 spawn、标记 dump/restore 存活）；J: workbench（占位会话）；browser `session-roundtrip.spec.ts` |
| | Session agent binding is immutable（wire-fix 补行；interaction-polish 修订：agent 选择唯一入口在创建对话框）| ✅ | 前端 `workbench-composer.test.ts`（🔒 只读小字、工具条无任何 agent/创建/设置控件）、`new-session-dialog.test.ts`（对话框是唯一选 agent 处）；browser `models.spec.ts`「detail head shows the bound agent with the lock affordance」|
| | Composer submissions always deliver（wire-fix 补行）| ✅ | `spawn_race_test`（占位标记一次性消费/生产发射路径 WebSpawn 携带 kind/model/0-turn 占位同样 spawn——「输入框发不出消息」回归锁）；E `session_round_trip_via_webui_http` |
| | Project-level default agent（wire-fix 补行）| ✅ | `session_endpoints_test::project_default_agent_follows_last_use`（创建即记录/跟随最近一次/项目间互不串/无记录如实空，2026-09-11 补）；前端 `new-session-dialog.test.ts`（defaultAgent 预选、无记录兜底首个可达）、`project-rail.test.ts`（对话框绑定项目 default_agent）|
| | Pending submissions stack above the composer（turn-queue 新增）| ✅ | 前端 `pending-stack.test.ts`（顺序/disposition 措辞/priority 钉住/移除与重排和解）；browser `pending-stack.spec.ts`（忙中提交上栈、刷新存活、归档点名损失）|
| | Workbench renders the focused session as a conversation（conversation-view 新增）| ✅ | 前端 `transcript-view.test.ts`（两侧交替、N chunk 一气泡、tool 组可展开不混排、submission 回合开始即现）；`dashboard.test.ts`（聚焦会话内联 conversation）；browser `conversation.spec.ts`（两侧按序交替 + gated tool 组展开）|
| | Workbench is the single conversation surface（conversation-view 新增）| ✅ | 前端 `project-rail.test.ts`（点击就地切换留在 workbench 3.1、current 标记跟随焦点指针 3.2）；`dashboard.test.ts`（深链 `/sessions/:key` 经 deepLinkKey 渲染、聚焦会话 head 带 close+archive）；browser `conversation.spec.ts`「rail click focuses the session in place on the workbench」|
| permission-flow | Hook 驱动权限请求 | ✅ | `permission_flow_test`；fake-claude "perm" 场景 |
| | 三种决定结果 | ✅ | `permission_flow_test`、sebas-webui `acp_permission_roundtrip_test`；E `permission_loop_allow_once/deny/allow_session_over_core_channel`（fake-claude "perm" 真实泊车经核心通道全环：allow 执行 / deny 拒绝 / allow_session 切 auto 后第二回合免审批，飞书无关面，2026-09-18 补） |
| | allowlist 命中自动批准 | ✅ | `permission_flow_test` |
| | allowlist 作用域与生命周期 | ✅ | `permission_flow_test` |
| | 迟到点击处理 | ✅ | src `core_channel/tests.rs`（typed rejection）|
| | 无应答者 fail-closed | ✅ | src `core_channel/tests.rs`（fail-closed）；detached 审批闭环 browser `approval-detached.spec.ts`（cover-core-channel-test-gaps B1.2，原缺口 #1 已收口）|
| | Session mode gates whether a decision is requested（add-agent-mode-selection 修订）| ✅ | E `mode_threads_to_agent_argv`（argv 透传 + allow 免审批 + 未知 mode 400）；E/J `mode_mid_session_switch` / `remote_node_mode_journey`（远端 allow 免门控↔ask 恢复 waiting）；sebas-acp driver 单测（词汇映射/argv 解析）；browser `mode.spec.ts` |
| | webui：会话创建携带 mode | ✅ | `session_endpoints_test::create_session_with_mode_threads_spawn_and_mid_session_mode_switch_works`（wire 透传 + 未知 mode 400）；E `mode_threads_to_agent_argv`；browser `mode.spec.ts` |
| | webui：会话中途切换 mode | ✅ | 同上（SetMode 经 SendAcp）；E `mode_mid_session_switch`（journal 记录运行时切换 + effective 落定）|
| | webui：会话 mode 在 dashboard 可见可切 | ✅ | 前端 `dashboard.test.ts`（mode-tag 本机/远端同通道）；browser `mode.spec.ts`（head tag 展示 + composer 底沿 mode-switch 切换）|
| | webui：SessionBackend seam 承载 mode | ✅ | `spawn_race_test`（WebSpawn 携带 mode=None 透传）；src `core_channel/tests.rs`（Spawn/CreatePlaceholder/SetSessionMode 帧分发）|
| | Submit control reflects submission and turn state（interaction-polish 新增）| ✅ | 前端 `workbench-composer.test.ts`（disabled/send/sending/stop/queued 逐态、停止走 cancelSession、排队复用 sendMessage、turn 结束复位）；browser `submit-control.spec.ts`（working 停止方块 → cancel 链路 → 会话存活可继续；有字排队形态 → pending-stack；取消不丢排队提交）|
| | Composer toolbar composition（interaction-polish 新增）| ✅ | 前端 `workbench-composer.test.ts`（左下 🔒 agent、右下模型芯片+发送；无创建/设置/mode 控件）；browser `mode.spec.ts`（无聚焦时 composer 无 mode 控件）、`first-paint.spec.ts`（无聚焦 = 指向 rail 创建入口的提示，无任何创建控件）|
| | Workbench layout is resizable（interaction-polish 新增）| ✅ | 前端 `split-persist.test.ts`（假 storage 持久化与恢复、180–480/120–半高 clamp、隐私模式退化）、`app-shell.test.ts`（frame 分割面板 + railWidth clamp）、`dashboard.test.ts`（vsplit + composerHeight）；browser `layout.spec.ts`（两道分割线拖拽、localStorage `sebas.rail-width`/`sebas.composer-height`、刷新恢复、<640px 禁拖退化）|
| | Workbench regions read as floating islands（interaction-polish 新增）| ✅ | 前端 `app-shell.test.ts`（nav 圆角浮岛、无通高 border-right、frame 分割缝）；browser `layout.spec.ts`（canvas 与浮岛异色、分隔缝 rest 透明 hover 亮起、舞台浮岛在位）|

### ④ 项目管理（核心）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| project-session-actions | 目录选择器加项目 | ✅ | J: `projects_session_journey` |
| | 无 prompt 新会话 | ✅ | J: `workbench_aggregate_journey` |
| | 会话归档 | ✅ | sebas-webui `api_endpoints_test` |
| | 历史组即归档（rail-declutter-unread 修订：倒序 + Inbox 移除）| ✅ | sebas-webui `api_endpoints_test`；前端 `project-rail.test.ts`（History 按 archived_at 降序渲染、Inbox 组不存在断言、无项目会话无 rail 展示位）；browser `session-mgmt.spec.ts`「History lists newly archived sessions newest-first」（两次归档间隔 >1s，倒序相对位置在真实 rail 中钉住）|
| | 归档过期 | ✅ | src `archive.rs` 内联测试 |
| | Project removal from the rail（rail-declutter-unread D5 修订）| ✅ | 见簇③「Rail project removal entry」同行证据|
| | Session close from the rail（审计补行）| ✅ | sebas-dispatch `web_close_test`（active/dormant/spawning/unknown/focused 全语义）；`session_endpoints_test::close_active_session_drops_mapping_and_returns_200` 等 close 族；browser `session-mgmt.spec.ts`「close removes the session from the active list」|
| | Placeholder session is immediately writable（审计补行）| ✅ | `spawn_race_test` 占位三态（见簇③同行）；browser `session-roundtrip.spec.ts` |
| state-store (projects 面) | DB 位置与单写者 | ✅ | sebas-router state_store 测试；J: lifecycle（迁移日志）|
| | schema 版本与自动迁移 | ✅ | 同上（migration 0→1 日志）|
| | 迁移前备份 | ✅ | 同上（backup 文件）|
| | 通道状态方法 | ✅ | src `core_channel/tests.rs` |
| | 变更持久性 | ✅ | state_store 测试 |
| | store 不可用诚实降级 | ✅ | state_store 测试 |
| | 损坏 store 不静默重置 | ✅ | state_store 测试 |
| | Runtime state boundaries for persisted session state（retire-session-persistence 补行）| ✅ | `state_persistence_test`（allowlist/占位不落盘、会话图落 `[dispatch] state_file` 快照语义）；`spawn_race_test::dump_filters_spawning_and_persists_mapping_dto` |
| webui (projects 面) | 项目注册降级如实提示 | ✅ | sebas-webui `session_endpoints_test`（`projects_add_degraded_when_core_unreachable`：201 + `degraded.cause`；正常路径无标记）；browser `deployment.spec.ts`（加项目降级提示，harden-core-channel-deployment）；前端 `project-rail.test.ts`（降级 hint 三态）|

> ④ 的「webui (projects 面)」按主 spec 界定收 webui capability 下直接归属项目管理面
> 的 requirement（本期仅「项目注册降级如实提示」一条）；projects 的增删/排序等其余
> webui 行为经 project-session-actions 与 agent-workbench 的同名 requirement 命中，
> 不重复计分。

### ⑤ 通道与监督（核心，raise-core-coverage-to-90 新增簇）

| 能力 | requirement 簇 | 状态 | 证据 |
|---|---|---|---|
| core-session-channel | Core 是唯一会话权威 | ✅ | src `core_channel/tests.rs`（`backend_methods_reach_the_right_handlers`、`client_converges_after_server_restart`：核心值权威、客户端收敛）|
| | 通道传输与鉴权 | ✅ | 同上（`missing_handshake_closes_connection_without_response`、`wrong_and_empty_secrets_are_rejected`、`cross_uid_rejected_live_process` 真实 fork+setuid 跨 uid 拒绝，`#[ignore]` root 实测通过；stale socket 回收：同路径重启用例 + E `no_secret_assembly`）；harden：密钥文件 0600 |
| | 会话观察方法 | ✅ | `subscription_delivers_every_mutation_after_the_snapshot`（snapshot 先行）、`lagging_subscriber_is_disconnected_and_can_resnapshot`；快照含执行体/模型（`create_placeholder_wires_a_zero_turn_session`）+ pending（`pending_submissions_visible_and_manageable_over_the_channel`，turn-queue 修订）|
| | 会话驱动方法 | ✅ | `backend_methods_reach_the_right_handlers`、`create_placeholder_wires_a_zero_turn_session`、`cancel_rejects_unknown_and_idle_and_cancels_working`（interaction-polish 修订：未知/空闲 typed 拒绝 + WORKING 取消 Ok 且会话保留）、`ensure_message_attachments_are_validated`（缺失附件 typed rejection + 合法附件 Ok）；pending 管理两 scenario（移除 + 已开始拒绝，turn-queue 修订，同上用例）；E `session_round_trip_via_webui_http`、`cancel_typed_rejections_over_webui_http`、`cancel_interrupts_in_flight_turn_over_webui_http`（stub 级中断，如实标注）、`cancel_without_core_answers_503` |
| | 回合内容获取 | ✅ | `backend_methods_reach_the_right_handlers`（turns 增量位置语义）；`session_events_test::tool_events_are_labelled_tool_in_turn_content`（submission 与 tool 条目可区分，conversation-view 修订）、`turns_are_incremental_by_position`；`full_e2e_test` 回合内容回读 |
| | 核心不可达诚实降级 | ✅ | `unreachable_causes_are_distinct`、`client_converges_after_server_restart`；E `reachability_flips_across_core_restart`、`secret_rotation_self_heal_across_core_restart`；webui 全局横幅 + browser `deployment.spec.ts`（横幅 cause/composer 门禁/恢复消隐）|
| | 协议使用中性会话键 | ✅ | `full_e2e_test`（ChannelKey 语义）；`core_channel/protocol.rs` 内联测试 |
| | 审批请求全执行体外显 | ✅ | `acp_permission_request_streams_and_answer_routes_back`；`permission_flow_test`；browser `approval-detached.spec.ts`（detached 审批 allow/deny，cover B1.2）；E `permission_loop_allow_once_over_core_channel`（真实泊车非合成：订阅流 ApprovalRequested 带 request_id/tool/args）|
| | Spawn backend 提示校验 | ✅ | src `agent_backend.rs`（`unknown_backend_hint_rejects_without_session`、`native_missing_credentials_rejection_names_the_backend`、缺省=ACP）|
| | 通道上的 gated-call 审批 | ✅ | src `core_channel/tests.rs`（审批往返/无应答 fail-closed/unknown rid typed rejection）；`permission_flow_test`；E `permission_loop_allow_once/deny/allow_session_over_core_channel`（跨进程全环：帧下发→ApprovalAnswer→泊住 hook 复活→transcript 记录决定语义）|
| | 通道上的会话模型选择 | ✅ | src `agent_backend.rs`（native key 分发 + unknown 拒绝 + override 落快照）；E/browser `models.spec.ts`（fakeacp set_session_model happy-path + 未知模型 typed rejection，cover B2.2）|
| | 无注入密钥的通道自动武装（harden）| ✅ | `auto_arm_without_env_writes_secret_file_and_completes_handshake`、`auto_arm_with_env_uses_env_value_and_writes_matching_file`；E `no_secret_assembly_end_to_end`（0600 + 会话往返）|
| | 通道客户端密钥文件发现（harden）| ✅ | `client_discovers_secret_from_file_and_heals_key_rotation`、`discovery_both_missing_warns_once_and_uses_empty`；E `secret_rotation_self_heal_across_core_restart` |
| | bind 失败即硬启动失败（harden）| ✅ | `arm_fails_hard_when_socket_path_is_taken_by_live_listener`；supervisor `bind_failed_exit_code_marks_degraded` |
| | 状态库通道面（cover；core-owned 修订）| ✅ | `tests/state_channel_contract_test.rs`（StateSnapshot/StateMutation/StateMutationOk/Rejected 不静默吞错/StateSubscribe 订阅后 mutation 帧；fake engine 注入）；provider mutation 三态 + `settings_defaults_mutation_requires_provider`（core-owned 新 scenario：provider 管理走通道 + defaults 与 provider 数据同库）|
| | reachability 三态区分启动失败（cover）| ✅ | `reachability_startup_failed_with_env_file`、`reachability_startup_failed_fallback`、`reachability_auth_rejected_after_handshake`、`reachability_disconnected_after_connected`（`kind`= startup_failed/auth_rejected/disconnected + 闩锁 enrich）；E `startup_failure_*` 75 契约 |
| | ensure_message IM 投递语义（cover）| ✅ | `ensure_message_unknown_key_auto_creates`、`ensure_message_dormant_resumes`、`message_unknown_key_rejected` |
| | Pending queue advances without depending on a single terminal event（fix-pending-queue-liveness 新增）| ✅ | sebas-dispatch `turn_stall_test`（停滞收尾 SEED/WORKING→DONE + drain 开轮队头 + TurnStalled 通知点名会话与释放数 + 泊车豁免/解除重计 + timeout=0 短路 + 终态清事实）与 `engine/stall.rs` 单测（泊车挂起计时/解除重计）；`turn_engaged` 快照事实：`session_endpoints_test::session_payloads_carry_turn_engaged_fact`；E `stalled_turn_force_settles_and_the_queue_self_heals`（进程级：沉默子进程 + 忙中提交 → 看门狗收尾 → 队列自愈前进 + core.log warn）、E `parked_permission_does_not_trip_the_stall_guard`（泊车豁免进程级）；呈现面：前端 composer/pending-stack 单测 + browser `pending-stack.spec.ts`「deterministic rejection…notice」（拒绝反馈旅程，2026-09-17 补）|
| | Model list fetch over the channel（fetch-models 新增）| ✅ | `tests/state_channel_contract_test.rs::fetch_models_op_returns_ids_and_injects_key_upstream / rejects_provider_without_base_url / sanitizes_upstream_failure / persists_nothing`；core 侧 `sebas-dispatch/src/state_store.rs::providers_fetch_models` + `provider_fetch_target_rejects_when_no_usable_base_url`；webui 全栈 `gateway_bff_test::provider_probe_fetches_models_without_persisting`（本地 mock 上游）|
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
| | └ add-agent-mode-selection：启动权限模式 / 运行时切换 / 探针不覆盖 | ✅ | E `mode_threads_to_agent_argv`（argv 断言）+ `mode_mid_session_switch`（ModeChanged + 探针共存）；sebas-acp driver 单测（control_mode_flag/parse/strip）|
| acp-session-mapping | ✅ | 见核心簇① |
| session-slash-commands | ✅ | 前端 `workbench-composer.test.ts`（面板触发/前缀过滤/两段式补全/拦截与诚实退化 + workbench-composer-input-polish 3.1/3.2：行收敛单行化守卫、hover∨高亮同源 markdown 气泡、Esc/移开收泡、360×240 内部滚动、sanitize 结构守卫）；sebas-acp `canned/no_duplicate_prompt`（命令表先导事件契约）；claude 命令发现 E `slash_commands_advertise_and_reach_stub` |
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
| router-admin-api | ✅（make-core-own-provider-data 退役 4 条：Provider CRUD endpoints、Model alias CRUD endpoints、Model probe endpoint、Write-then-apply semantics——出账）| 退役面：sebas-router `admin_test`（`retired_provider_surface_answers_404`、`probe_endpoint_is_gone`、`agent_defaults_surface_is_gone`、`admin_mutations_write_no_files_without_core_channel`）；「Configuration source」：E `core_owned_provider_reaches_router_without_restart`（card-edited provider 经通道订阅无重启可路由，2026-09-11 补）+ `spawn_env_store_authority_test`（store 权威）+ state_store 迁移单测；「External change hot reload」：同 E 用例（change notification → 热交换）+ `hot_reload_external_write_and_failure_recovery`；「Admin authentication」「Preset table endpoint」：`admin_test`、J: provider_governance |
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
| testsuite-acceptance | ✅ | 套件本体：`invoke testsuite-acceptance` 旅程（`tests/testsuite_acceptance_test.rs`，`#[ignore]` 不进默认 cargo test、失败保留沙箱、`--case` 单跑）；达标复核即本文件统计段 |
| | └ add-agent-mode-selection：远端节点 mode 旅程 | ✅ | J: `remote_node_mode_journey`（allow 免门控 → 中途切 ask 恢复 waiting → 离线拒绝点名节点；EchoBody 桩零 token）|
| testsuite-process-e2e | ✅ | 套件约定（沙箱隔离/显式超时/失败保留现场/平台门控）由 `tests/testsuite_e2e_test.rs` + `tests/testsuite_acceptance_test.rs` 落地并被其用例遵循；E: 全部 e2e 用例 |
| watchdog | ✅ | 见核心簇⑤ |
| webui | ✅ | sebas-webui 全套端点测试（harden-core-channel-deployment：projects 注册降级标记 `degraded.cause`/正常路径无标记；turn-queue：`session_payloads_carry_pending_submissions_in_delivery_order`、`pending_remove_and_move_endpoints`（已开始 409 typed rejection）、`message_overflow_is_a_visible_rejection`；conversation-view：`summary_focused_session_matches_detail_entry_view` 等 payload 两侧条目同序/退役字段不回流断言）；E: detached 双进程启动/健康/重连 + `turn_queue_timing_and_dropped_accounting` + `core_owned_provider_reaches_router_without_restart`（`testsuite_e2e_test`）；浏览器级 UI 旅程 ✅（`testsuite-webui-browser`，含 conversation/pending-stack 新 spec，见下）|

### testsuite-webui-browser（非核心：浏览器级 UI 旅程，Playwright）

> 入口 `invoke testsuite-webui`（`tests/testsuite-webui/`，独立 pnpm 包）；后端为一次性沙箱
> （`sebas core --webui` + 独立 `sebas router --config … --debug` 两进程 + fake-claude 桩），
> chromium headless。detached 双进程形态（core + 独立 `sebas webui`、无
> `SEBAS_CORE_SECRET`、自动装配 + 密钥文件发现）由
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
| 鉴权闭环 | 免登录直达 | the workbench is the homepage and no gate ever appears | `auth-off.spec.ts` | 鉴权与访问旅程「免登录直达」 |
| 首启 root 引导 | 首启门禁 | homepage shows the setup card and never flips to the login gate | `auth-setup.spec.ts` | 鉴权与访问旅程「首启 root 引导」 |
| 首启 root 引导 | 校验与建户 | in-place validation rejects short password and mismatch without submitting | `auth-setup.spec.ts` | 鉴权与访问旅程「首启 root 引导」 |
| 首启 root 引导 | 校验与建户 | creating root enters the workbench and the session survives reload | `auth-setup.spec.ts` | 鉴权与访问旅程「首启 root 引导」 |
| 首启 root 引导 | 校验与建户 | a second setup POST is refused after root exists (409) | `auth-setup.spec.ts` | 鉴权与访问旅程「首启 root 引导」 |
| agent 对话覆盖 | 首回合往返与重载恢复 | composer submit → reply → done → reload restores | `session-roundtrip.spec.ts` | agent 对话覆盖「首回合往返」、会话核心旅程「重载恢复」 |
| agent 对话覆盖 | 流式分批 | chunks arrive in batches and the turn converges to done | `streaming.spec.ts` | agent 对话覆盖「流式分批渲染」 |
| agent 对话覆盖 | 多轮连续 | 4.1 two consecutive rounds append in order and survive reload | `dialog.spec.ts` | agent 对话覆盖「同会话多轮连续」 |
| agent 对话覆盖 | 输入守卫 | 4.2 empty and blank input creates no turn, session stays usable | `dialog.spec.ts` | agent 对话覆盖「composer 输入守卫」 |
| agent 对话覆盖 | 输入守卫 | 4.2 special-char long text round-trips without loss or console errors | `dialog.spec.ts` | agent 对话覆盖「composer 输入守卫」 |
| agent 对话覆盖 | 错误诚实呈现 | refuse — non-terminal: session survives, next message works | `errors.spec.ts` | 会话核心旅程「错误呈现」 |
| agent 对话覆盖 | 错误诚实呈现 | crash — honest death: mapping torn down, row gone, open view retains transcript | `errors.spec.ts` | 会话核心旅程「错误呈现」 |
| agent 对话覆盖 | spawn 失败内显 | spawn failure inline: error event in transcript, session stays as spawn-failed | `errors.spec.ts` | webui「web_spawn 失败的立即内显」（fail-fast-on-startup-errors）；聚焦触发的 activate 会重排 SpawnFailed 致转录换址（sebas-il7s），旅程暂以 route 拦截 stabilize，产品修复后移除 |
| 审批卡片旅程 | 拒绝路径 | deny path — refusal semantics, turn completes | `permission.spec.ts` | 审批卡片旅程「拒绝路径」 |
| 审批卡片旅程 | 单次允许路径 | allow-once path — allowed semantics, turn completes | `permission.spec.ts` | 审批卡片旅程「单次允许路径」 |
| 审批卡片旅程 | 会话级允许 | allow-session path — session switches to auto mode, follow-up is no longer gated | `permission.spec.ts` | 审批卡片旅程「会话级允许」 |
| 项目管理覆盖 | 增删 | add via project dialog appears in rail; removed project disappears | `projects.spec.ts` | 项目管理覆盖「项目增删」 |
| 项目管理覆盖 | 异常拒绝 | 1.1 illegal path 400, duplicate 409, removal persists across reload | `projects.spec.ts` | 项目管理覆盖「项目异常拒绝」 |
| 项目管理覆盖 | 排序与持久化 | 1.2 reorder persists; branch probe drives state, rail hides the branch name (D8) | `projects.spec.ts` | 项目管理覆盖「项目排序与分支呈现」（rail-declutter-unread 改写：分支名不再上 rail） |
| 项目管理覆盖 | 增删 | … menu removal: blocked while live sessions exist, precheck and typed rejection agree | `projects.spec.ts` | project-session-actions「Project removal from the rail」removal-blocked/rejection scenarios（rail-declutter-unread） |
| 项目管理覆盖 | 选择器交互 | P1 tree expand, click-select fills path, submit lands in rail | `projects.spec.ts` | 项目管理覆盖「folder-picker 树展开点选回填」 |
| 项目管理覆盖 | 选择器交互 | P2 empty path disables submit; missing path errors inline, dialog stays | `projects.spec.ts` | 项目管理覆盖「选择器空路径与非法路径」 |
| 会话管理 | close 与 archive | close removes the session from the active list | `session-mgmt.spec.ts` | 会话管理覆盖「close 与 archive」 |
| 会话管理 | close 与 archive | archive hides from list, shows in History | `session-mgmt.spec.ts` | 会话管理覆盖「close 与 archive」 |
| 会话管理 | 深链与退役路径 | deep link renders the session via SPA fallback; /settings redirects to / | `session-mgmt.spec.ts` | 会话管理覆盖「深链与退役路径」 |
| 会话管理 | claude 芯片在场诚实形态（原「模型面诚实缺省」随 claude 自报模型面退役）| model surface honest default — the claude chip renders the observed current, never the placeholder | `session-mgmt.spec.ts` | 工作台、项目与会话管理面旅程「模型面诚实缺省」（workbench-composer-input-polish 改写：chip 在场 + 误导占位不得出现；无模型诚实缺省由前端单测 + 无 configOptions 通用 ACP 承载）|
| 会话管理 | close 与 archive | History lists newly archived sessions newest-first (rail-declutter-unread D7) | `session-mgmt.spec.ts` | project-session-actions「History is sorted newest-first」（rail-declutter-unread） |
| 未读徽标 | 真实回复点亮与聚焦清零 | a reply to an unfocused session lights the badge; focusing clears it and parks the anchor | `unread-badge.spec.ts` | session-unread-badge「new reply on an unfocused session」「focusing the session clears the badge」「first visit shows no unread」（rail-declutter-unread，跨层旅程：fake-claude 回复 → API `msg_count` 投影 → 徽标免刷新亮起 → 聚焦清零且锚落 localStorage） |
| 未读徽标 | 聚焦到达与重复聚焦 | focused arrivals never badge; repeated same-session clicks keep it cleared | `unread-badge.spec.ts` | session-unread-badge「Streaming arrival into the focused session does not badge」+「Repeated focus keeps the badge cleared」（fix-webui-qa-defects-round3 delta⁷：聚焦 + 可见不点灯且锚被 transcript 贴底真推进——localStorage 佐证，非「抑制遮旧账」；重复同会话点击保持清零。⚠ detail 读取即设焦点的探针纪律见 waitListStatus⁷） |
| 会话管理 | 多会话切换 | 2.1 dual-session switch (rail + deep-link) does not crosstalk | `sessions.spec.ts` | 会话管理覆盖「多会话切换」 |
| 会话管理 | archive 写保护 | 2.2 archive → 400 write-protection → restore unhides honestly | `sessions.spec.ts` | 会话管理覆盖「archive 写保护与 restore 诚实语义」（restore 语义随 fix-webui-qa-defects 翻转为重建会话行，见下行） |
| 会话管理 | 归档恢复重建 | archive → restore: rail row back, transcript intact, History cleared, writable again | `archive-restore.spec.ts` | project-session-actions「restore archived session」+「restore preserves the transcript」（fix-webui-qa-defects delta²：恢复确认弹窗含重建文案、rail 行回原项目、detail 全 N 条、History 清空、首条消息可写） |
| 会话管理 | rail 切换即时聚焦 | 4.2 focusing A and clicking B in the rail renders B within the throttle window, with no other events | `rail-focus.spec.ts` | agent-workbench「rail selection renders the conversation immediately」（fix-webui-qa-defects delta²：switch 后无任何会话事件介入，`sebas:rail-focus` → 节流刷新 → 主区渲染 B） |
| 审批卡片旅程 | 审批读模型恢复 | reload rebuilds the review card from the read model under the same request_id, and deciding clears it for good | `approval-restore.spec.ts` | permission-flow「刷新或重连后仍可取得」+ agent-workbench「刷新后审批面从读模型重建」（fix-webui-approval-restore-and-session-identity delta³：刷新后卡片按同 request_id 重建、唯一决策面、批复后读模型清空、rail waiting 投影） |
| 审批卡片旅程 | 相位对账与挂载去重 | waiting-phase empty pull backs off and retries until the card lands without any phase change (round3 7.1) | `approval-reconcile.spec.ts` | fix-webui-qa-defects-round3 tasks 7.1⁷（无 delta scenario，task 级验收）：route 模拟「拉取先于落库」——waiting 下 ≥2 次扑空拉取证明退避环在转，放行后 ≤4s 免刷新免相位翻转补卡（sessionPhase 钉在 waiting、reload 探针）。实施期曾发现 detail 投影缺泊车合并把 sessionPhase 遮成 working（当时以 fulfill detail 舞台绕行上报）；实现修复（api.rs detail 补 `with_parked_approvals`，与 session_phase_frame 同款）落地后已拆舞台直跑真实投影 |
| 审批卡片旅程 | 相位对账与挂载去重 | mount and session switch pull the approvals read model exactly once (round3 7.2) | `approval-reconcile.spec.ts` | fix-webui-qa-defects-round3 tasks 7.2⁷（无 delta scenario，task 级验收）：rail 切换与冷深链各恰好一次 approvals GET 且卡片照常从读模型重建（挂载期三路并发收敛为单次请求，pullSeq 防陈旧语义不回归） |
| 停止收尾 | 回合被停止条目 | stopping a streaming turn appends the stop entry, resets the control, and stays settled across reloads | `stop-settle.spec.ts` | agent-workbench「停止后 transcript 有停止条目」+「刷新后不复活在飞状态」（同上 delta³：错误类停止条目、控件复位、reload 稳定、会话存活可继续） |
| 停止收尾 | 停止释放泊车审批 | stopping a parked turn releases the approval fail-closed: read model drains, late decision is rejected, stop entry lands | `stop-settle.spec.ts` | permission-flow「停止回复清空未决审批」+「释放的请求不可再批复」（同上 delta³：读模型清空、迟到批复 404、卡面 expired、turn_engaged 复位） |
| 会话管理 | 聚焦联动与展开持久 | rail switch and creation landing drive the project title; project-row clicks stay independent | `rail-expand.spec.ts` | agent-workbench「rail 切换会话后项目标题跟随」+「项目行点击仍独立生效」（同上 delta³：`sebas:project-follow` 反投影 shell selectedPath，rail 点击/新建落地均跟随） |
| 会话管理 | 聚焦联动与展开持久 | expansion persists across reloads; the focused project defaults to expanded with no record | `rail-expand.spec.ts` | agent-workbench「展开状态跨刷新保持」+「聚焦会话所在项目缺省展开」（同上 delta³：`sebas.rail-expanded` 持久、聚焦缺省展开并物化、无操作不自行收起） |
| 会话管理 | 聚焦联动与展开持久 | deep link lands the focused session project in the main title, never 「未选择项目」 | `rail-expand.spec.ts` | agent-workbench「rail 切换会话后项目标题跟随」深链/刷新直达半边（fix-webui-qa-defects-round3 tasks 4.2/6.3⁷：detail 先于 projects.list 落地的竞态不再把标题钉死在「未选择项目」，冷加载与刷新双查） |
| 会话管理 | 行重命名 | label takes precedence over the first-prompt preview, survives reloads, and clearing falls back | `session-label.spec.ts` | project-session-actions「operator label takes precedence」+「renaming from the rail」（同上 delta³：rail … 菜单改名就地生效、对话框同源命名、清空回退首条 prompt 预览） |
| 会话管理 | 行重命名 | a label write through the API flips the rail row live, without a reload (round5 C) | `session-label.spec.ts` | project-session-actions「label writes through any path update the row live」（fix-webui-qa-defects-round5 delta⁷：API 写 label → session.updated 帧 → rail 400ms 防抖重取就地翻名，≤3s 窗口钉死帧链路、清空同形，reload 探针免刷新） |
| 会话管理 | 行菜单可达性 | closed row menus expose no menu items to the a11y tree; open ones do | `row-menu-a11y.spec.ts` | project-session-actions「renaming from the rail」（fix-webui-qa-defects-round5 tasks 4.1⁷：`wa-dropdown:not([open]) wa-dropdown-item { display:none }`，ariaSnapshot 关闭态无「重命名/归档/移除项目」、会话行与项目行展开态在场、Escape 再收） |
| 会话管理 | 归档恢复身份 | archived→restored keeps agent_kind, mode and the model catalog; legacy entries fall back honestly | `archive-identity.spec.ts` | project-session-actions「恢复保留 agent 身份与模型面」+「旧归档条目如实回退」（同上 delta³：fakeacp+ask 身份四项随归档→恢复原样带回、头部如实显示；旧档 strip 身份后 default agent 回退） |
| 项目管理覆盖 | 越界禁用原因 | out-of-root and missing paths state their reason with submit disabled; a valid path enables it | `add-scope-reason.spec.ts` | workspace-root「手填越界路径给出禁用原因」（同上 delta³：越界/不存在各给原因、提交保持禁用、合法路径恢复可用；⚠ 当前被 Add project 手填绑定缺陷阻塞，见 change 报告） |
| 工作台首屏 | 图标本地化 | blocking the icon CDN changes nothing: zero CDN requests, local /icons assets, icons still render | `icons-local.spec.ts` | fix-webui-approval-restore-and-session-identity tasks 5.4（无 delta scenario：阻断 ka-f.fontawesome.com 零请求 + wa-icon 资源同源 /icons；⚠ 当前被 `/icons/{*path}` 路由前缀缺陷阻塞，见 change 报告） |
| 模型管理覆盖 | claude 别名模型面与切换 | claude alias table reaches the snapshot, switch rides the control protocol, and the chip follows | `models.spec.ts` | acp-model-selection「Claude session exposes the alias table」+「Claude switch applies from the next turn」（workbench-composer-input-polish；原「无模型会话 set_model 诚实拒绝」的浏览器载体随 claude 自报模型面退役——claude 现在有别名表，无模型诚实缺省由前端单测 + 无 configOptions 的通用 ACP agent 承载）|
| 模型管理覆盖 | settings provider 抓取 + 条目编辑 | 3.2 providers are editable: model entries with capability tags persist; browsing stays probe-free | `models.spec.ts` | webui「Provider management page」（redesign-provider-models-settings）|
| 模型管理覆盖 | settings provider 抓取 + 条目编辑 | fetch lists the official ids without persisting; picking joins the catalog via an ordinary edit | `models.spec.ts` | webui「Fetch models from the provider's official base URL」 |
| 模型管理覆盖 | settings provider 抓取 + 条目编辑 | a fetched-and-saved catalog reaches the creation dialog without a restart | `models.spec.ts` | webui「Fetch models…」+ agent-workbench「Model selector offers the backend catalog before any session」reflected-without-restart（revamp-settings-nav-and-models-editor；设置编辑器抓取→保存→创建对话框同页免刷新可读的接缝旅程） |
| 模型管理覆盖 | settings provider 抓取 + 条目编辑 | a provider without any usable base URL renders no fetch entry | `models.spec.ts` | webui「Fetch models from the provider's official base URL」 |
| agent 绑定不可变 | 锁定提示 | detail head shows the bound agent with the lock affordance | `models.spec.ts` | agent-workbench「Session agent binding is immutable」 |
| 设置面 ¹ | 只读呈现 | S1 services rows match /api/admin/services truth (response-driven) | `settings.spec.ts` | webui「Services 分区与 router 状态归属」¹ |
| 设置面 ¹ | 只读呈现 | S2 about table matches /api/about truth | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S3 env table renders placeholder semantics under Env Vars | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S6 bare core degrades: no-adapter banner, no rows, restart disabled | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 分区导航 IA | nav order Generic→Appearance→Services→Models→Env Vars→About(pinned), default focus, memory, stale-value fallback | `settings.spec.ts` | webui「设置弹窗分区与缺省首项」：缺省聚焦 Generic 分区／历史记忆恢复上次分区／缺省聚焦 Settings 分区（旧值回退）／分区顺序与 About 压底（revamp-settings-nav-and-models-editor） |
| 设置面 ¹ | 写降级 | S4 defaults read parity; set-default stays local, provider seeded via API | `settings.spec.ts` | provider-management「Set default provider and model from the page」¹ |
| 设置面 ¹ | 写降级 | S5a create/edit journeys persist through the core store (minimal forms) | `settings.spec.ts` | webui「Provider management page」（core store 持久化）¹ |
| 设置面 ¹ | 写降级 | S5b delete persists; fetch entry hidden without a base URL | `settings.spec.ts` | webui「Provider management page」+「Fetch models…」¹ |
| 对话视图 | 两侧交替 | submissions and replies alternate in order across several turns | `conversation.spec.ts` | agent-workbench「Workbench renders the focused session as a conversation」 |
| 对话视图 | 工具组展开 | the gated tool call renders as the turn tool group and expands | `conversation.spec.ts` | 同上（tool 组不混排 scenario）|
| 对话视图 | 就地聚焦 | rail click focuses the session in place on the workbench | `conversation.spec.ts` | agent-workbench「Workbench is the single conversation surface」 |
| 对话视图 | 模型两级选择 | creation mode offers provider → model from the Settings catalog; empty catalog is stated honestly | `conversation.spec.ts` | agent-workbench「Model selector offers the backend catalog before any session」 |
| agent mode 选择 | composer 模式下拉形态 | composer mode dropdown options stay single-line (round3 6.2) | `mode.spec.ts` | fix-webui-qa-defects-round3 tasks 4.1/6.2⁷（无 delta scenario，task 级验收）：展开真实下拉量测五选项盒高度 <40px（单行 ≈26px 量级，折行 ≥2×行高必越线）+ label part computed white-space:nowrap（shadow 内样式表命中证据——4.1 的 document 级规则够不到组件 shadow 树，agent 下拉当时的单行是短文案假阳性） |
| 待执行堆叠区 | 忙中提交上栈 | busy-time submission rides the stack, survives refresh, and archive names the loss | `pending-stack.spec.ts` | agent-workbench「Pending submissions stack above the composer」+「Rail session close entry」 |
| 待执行堆叠区 | 确定性拒绝可见 | deterministic rejection of a removal surfaces a low-severity notice naming the entry | `pending-stack.spec.ts` | agent-workbench「Pending submissions stack above the composer」rejection 两级反馈（fix-pending-queue-liveness 3.3：拦截 remove → 404 类型化拒绝 → warn 通知点名条目与原因 + 对账） |
| 待执行堆叠区 | 队列管理面 | move up and remove ride the composite to the hosting backend; queue and API agree (round5 A) | `pending-stack.spec.ts` | core-session-channel「reorder from the web UI in an embedded deployment」+「removing a pending submission over the channel」（fix-webui-qa-defects-round5 delta⁷：bare-core embedded 形态下 GUI 点「上移/移除」→ 复合后端按 key 转发 acp 桥，队尾相对序换位与条目消失经 GET pending 全量对账、GUI 行序一致） |
| 分级通知层 | error toast 寿命 | an error toast auto-dismisses after ~8s without manual closing | `error-toast.spec.ts` | webui「error 默认自动消失且参与挤占」的寿命半边（fix-webui-qa-defects-round5 delta⁷：真实 error 生产者 = 归档恢复失败上报，restore 注入 404 → toast 出现、2s 驻留、≈8s 自动消失（8–20s 断言窗），全程无手动关闭；挤占半边由 notify/notice-layer 单测承载。本套件无时钟 fake 惯例，WS 驱动的 Lit 应用挂 fake clock 会误伤重连/节流计时器——8s 真实等待） |
| 部署韧性 | 核心通道停启 | core 停 → 横幅含 cause、composer 门禁、加项目降级提示；core 恢复 → 横幅消失 | `deployment.spec.ts` | webui「全局核心可达性横幅」「项目注册降级如实提示」（harden-core-channel-deployment，detached 双进程形态）⁵ |
| 审批卡片旅程（detached） | detached 审批闭环 | allow path — ApprovalRequested 跨进程到达 review-card，allow 后 transcript 记录允许语义 | `approval-detached.spec.ts` | webui「approval_answer end-to-end (detached topology)」（cover-core-channel-test-gaps B1.2，detached 双进程形态）⁶ |
| 审批卡片旅程（detached） | detached 审批闭环 | deny path — deny 后 transcript 记录拒绝语义，回合完成 | `approval-detached.spec.ts` | webui「approval_answer rejects unknown request_id」⁶（cover B1.2，detached） |
| 审批卡片旅程（detached） | unknown rid | POST /api/permissions/{rid}/answer with an unknown rid → 404 typed rejection | `approval-detached.spec.ts` | webui「approval_answer rejects unknown request_id」（cover B1.2，detached）⁶ |
| 模型管理覆盖 | 有模型面的正向切换（acp:fakeacp） | set_session_model happy path — POST ok-model → ModelChanged → current_model 同步 + 选择器呈现 | `models.spec.ts` | webui「set_session_model happy-path via webui」（cover-core-channel-test-gaps B2.2）⁶ |
| 模型管理覆盖 | 有模型面的正向切换（acp:fakeacp） | set_session_model rejects unknown model — typed rejection 到达但 webui 无内联错误面（observed product gap），会话存活、current_model 不变 | `models.spec.ts` | webui「set_session_model rejects unknown model」（cover B2.2）⁶ |

> Settings 语义修正（fix-settings-menu-and-services-semantics，spec 改动 `1e4a807`）：
> S1 改写为 `/api/admin/services` 响应驱动（响应即真源，不枚举具体服务）、S6 新增裸
> core 退化覆盖（横幅 + 零行 + 重启 disabled）。redesign-provider-models-settings 归档
> （2026-09-11）后 S1 锚点改指 webui「Services 分区与 router 状态归属」（router
> 状态只在 Services 呈现，S4/S5a/S5b 经 core store 持久化）。本期树形账本 49 行 =
> 全套件 `it` 总数 49（主 config 42 + auth 3 + detached 4；较上期 41 行 +8：
> conversation 4、pending-stack 1、models +3（条目编辑/抓取两用例与锁定提示，
> 替换旧「settings provider 只读」对账行）、settings S4/S5a/S5b 与 session-mgmt
> 模型面用例按归档 spec 改写为同行数）。
> conversation.spec.ts / pending-stack.spec.ts 为本期新增 spec。

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
> 主 spec；「分区导航 IA」行锚到工作树 change `revamp-settings-nav-and-models-editor`
> 的 delta scenario。会话核心旅程的「首回合往返」「流式分批渲染」与 `agent 对话覆盖` 同名
> scenario 文本相同（工作台旅程的「项目增删」「close 与 archive」「深链与退役路径」
> 同理），经上表 tree 侧行同源锚定。harness 级 scenario（沙箱装配：启动即隔离 /
> 成功退出清理 / 失败保留现场；一键入口：一键运行 / 单旅程过滤 / 渐进补用例不改入口 /
> 顶层裸 test 拒绝）由入口机制承担，均不设单用例行。
>
> ³ 审批读模型恢复 / 停止收尾 / 聚焦联动与展开持久 / 行重命名 / 归档恢复身份 /
> 越界禁用原因各行的锚点指向尚未归档的 change
> `fix-webui-approval-restore-and-session-identity` 的 delta scenario（2026-09-19
> review/e2e 阶段补齐）；同步主 spec 时随该 change 归档一并落锚。其中
> add-scope-reason 与 icons-local 两旅程当前被两个既有实现缺陷阻塞（Add project
> 手填路径 @input 绑定失效；`/icons/{*path}` 路由漏拼 `icons/` 前缀），旅程本身
> 即缺陷的回归钉，修复后应转绿。
>
> 旅程加固波（2026-09-11，交付终验前）：新增 4 例——未读徽标跨层旅程
> （`unread-badge.spec.ts` 新 spec）、History 倒序浏览器旅程（`session-mgmt.spec.ts`）、
> 设置分区导航 IA 旅程（`settings.spec.ts`）、编辑器抓取→保存→创建对话框接缝旅程
> （`models.spec.ts`）。树形账本 53 行 = 全套件 `it` 总数 65（主 config 58 + auth 3 +
> detached 4；较上期注记 +4，均入主 config）。
>
> 鉴权三形态补全（2026-09-16，webui auth e2e）：新增 5 例——免登录直达的专属
> 守卫（`auth-off.spec.ts`，原由全部主 config 用例隐式承担）与首启 root 引导
> 四例（`auth-setup.spec.ts`，第三种沙箱形态：auth 开、零用户、不预建户，
> tasks.py `TESTSUITE_AUTH_SETUP=1` 端口 9896）。树形账本 58 行 = 全套件
> `it` 总数 70（主 config 59 + auth 3 + auth-setup 4 + detached 4）。
>
> ⁷ round3/round5 QA 缺陷收尾补测（2026-09-20）：新增 9 例——聚焦到达不点灯与
> 重复聚焦（`unread-badge.spec.ts`）、深链项目标题（`rail-expand.spec.ts`）、
> composer 模式下拉单行（`mode.spec.ts`）、waiting 相位对账退避 + 挂载期拉取
> 去重（`approval-reconcile.spec.ts` 新 spec）、队列管理面复合转发（
> `pending-stack.spec.ts`）、API 写 label 行名即时翻新（`session-label.spec.ts`）、
> 行菜单 a11y（`row-menu-a11y.spec.ts` 新 spec）、error toast 8s 寿命
> （`error-toast.spec.ts` 新 spec）。锚点指向尚未归档的 change
> `fix-webui-qa-defects-round3` / `fix-webui-qa-defects-round5` 的 delta scenario
> 与任务级验收；同步主 spec 时随归档一并落锚。
> 同源测试纪律（waitListStatus）：detail 读取（GET /api/sessions/{key}）即设
> 服务端焦点（api.rs「Reading the detail focuses this session」）——聚焦敏感旅程
> 在点击聚焦后轮询详情会把焦点偷回被观测会话（unread-badge 原用例曾因此假红，
> 测试自己的探针流量成了焦点小偷）；聚焦敏感段一律走列表轮询。
> 同批上报实现缺口（round3 7.1 review 补口，已修复落地）：detail 投影
> （GET /api/sessions/{key}）缺 `with_parked_approvals` 合并（api.rs，列表/相位帧/
> summary 在 review 3c 补过）——泊车期间 detail 说 working，dashboard 据此喂
> review-card 的 sessionPhase，round3 7.1 的 waiting 退避在默认形态下被 detail
> 遮死。修复（detail 补同款 remote 二选一合并）落地前 approval-reconcile 7.1 以
> fulfill detail 的舞台绕行，落地后已拆舞台直跑真实投影。

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

requirement 级残留：**0 条**（五簇复核 2026-09-11 收口）。本期复核发现并当场收口
两处（收口前均为全测试层无覆盖）：

0a. router 订阅 core provider 数据闭环（router-admin-api「Configuration source」
    「card-edited provider reaches router」/「External change hot reload」
    「card edit hot-applies」/provider-management Core owns「router reads a core
    change without a restart」共同指向）— 已收口：本期补 E
    `core_owned_provider_reaches_router_without_restart`（watchdog 形态 router
    子进程订阅通道，webui BFF 写库 → 不重启可路由、不写 provider 文件）。
0b. Project-level default agent（agent-workbench，workbench-agent-wire-fix 引入）—
    已收口：本期补 `session_endpoints_test::project_default_agent_follows_last_use`
    （创建即记录/跟随最近一次/项目间互不串/无记录如实空）+ 前端 composer
    `projectDefaultAgent` 预选单测。

历史缺口去向：

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
2. **（unify-router-process-shape 后已消解）~~`run --router` 忽略 `SEBAS_ROUTER_LISTEN`~~**：内嵌形态删除，router 只以独立进程运行（`[router] listen` 是唯一地址来源，进程级可预注入 `SEBAS_AGENT_ROUTER_URL`）。native 仍走 `SEBAS_AGENT_PROVIDER_BASE_URL` 直连路径（本套件已覆盖）。
3. **会话状态落盘仅在优雅退出**：硬杀（TerminateProcess）不产生状态转储；Windows 无便携优雅信号，故重启恢复段 unix 门控。
4. **路由状态已入 SQLite**：`[router] state_file`（sessions.json）不再是重启恢复的活性来源，state store DB（sebas.db）承担持久化——矩阵断言已按此更新。
5. **restore 不复活会话**（二期浏览器旅程发现）：`archive` 先 close（mapping + transcript 丢弃），`restore` 只删归档条目；恢复后详情页如实 404（`sessions.spec.ts` 2.2 已按此诚实语义断言）。与 project-session-actions「History 点击恢复可写」条文不一致，产品语义变更另立项。
6. **无模型会话 set_model 是终态杀伤**（二期浏览器旅程发现）：webui 只投递（200 ok），Claude 驱动以终态 Error 应答 SetModel，会话被拆除（`models.spec.ts` 3.2 已按终态诚实断言）。正向模型切换需驱动模型面（configOptions 透出 + ModelChanged），待另立项。
7. **sebas-agent 内联测试环境依赖**（fail-fast-on-startup-errors 实施期发现）：`cargo test -p sebas-agent` 在 Windows Git Bash 环境下 17 项失败（tools::bash / tools::search / loop_ 等，依赖 POSIX shell 语义）；清理树同样失败（71 passed / 17 failed 前后一致），与 change 无关，属环境缺口。同批发现 `card_stream_e2e_test` 在并行负载下偶发 5s deadline 超时（隔离运行稳定通过）。
8. **rail 行名跟随最新 prompt，与「named by the first prompt」字面不符**（旅程加固波 2026-09-11 发现，只报告不修）：rail-declutter-unread delta 要求会话行以「首条用户消息」预览命名，但后端 `emit_turn_card`（sebas-dispatch `engine/mod.rs`）每轮开轮 `card_states.drop` + `seed_card`（新 prompt），`SessionInfo.user_prompt` → `SessionRow.prompt_preview` 投影因此携带**最新**一轮 prompt——首次跟发消息后行名即从首条漂移到最新。前端单测 mock 行数据、既有浏览器用例均为单消息会话，都探不到；`unread-badge.spec.ts` 跨层旅程首次踩中（已按「不依赖行名稳定」改写断言）。倾向视为产品语义决策（最新 prompt 作行名有其 UX 理由），若确认按 spec 字面收口需后端在首条消息后冻结 prompt_preview，另立项。

## 变更账本（core-set 增删与能力面变更说明）

| 日期 | change | commit | 说明 |
|---|---|---|---|
| 2026-09-17 | fix-pending-queue-liveness（工作树实施） | 未提交 | 待执行队列前进性：引擎停滞看门狗（`[dispatch] turn_stall_timeout` 默认 600s、0=关；WORKING/SEED + 无泊车审批 + 超阈值无事件 → 非终端 Error 同款收尾 DONE + drain + `SessionEvent::TurnStalled` warn 事实）；快照加 `turn_engaged`（只在 true 上 wire，旧 core 缺省回退 slug 判定）；WS 新增 `session.turn_stalled` 通知帧（同 socket 推送，非新端点）；前端 composer 消费 turn_engaged + 泊车「等待你的审批」指示（review-cards 待决计数上报）、pending-stack 阻塞原因 + 起等时刻标注、pending 操作两级反馈（竞态竞输静默 / 确定性拒绝 warn 通知点名条目与原因，网络失败归后者）。证据：引擎 `turn_stall_test` 6 例 + `engine/stall.rs` 单测、`session_endpoints_test::session_payloads_carry_turn_engaged_fact`、前端 composer/pending-stack/dashboard 扩展单测、E `stalled_turn_force_settles_and_the_queue_self_heals` + `parked_permission_does_not_trip_the_stall_guard`、browser `pending-stack.spec.ts` 拒绝反馈旅程。核心集 ⑤ +1 requirement（117 → 118，见簇⑤新行）。 |
| 2026-09-16 | add-webui-tiered-notices（工作树实施，spec-go 过渡提交） | 未提交 | 分级通知层：四级通知（info/warn/error/fatal，top-center，wa-toast 栈 + 自研驻留横幅）+ fatal 锁定语义（core 不可达时 `wa-split-panel.frame` inert 全锁 + 遮罩卡，恢复即解锁 + 「核心已恢复」info toast）；`client.ts` 统一拦截未豁免 API 失败自动判级（豁免名单集中一处）；旧 ws-banner/core-banner 退役收编（WS 断线 = 持续 warn 驻留横幅）；`ReachabilityInfo` 接 `kind` 三档分文案。证据：notify store 单测（栈上限/去重/退订）、notice-layer/banner 组件单测（四级变体/时长/手关/焦点）、app-shell 单测（get 初始化/翻转 fatal+解锁+toast/重连收敛/轮询退役守卫）、client 拦截器单测；浏览器级 `tiered-notices.spec.ts`（detached 停核→fatal+inert、恢复→解锁+toast）+ deployment 旅程同步新语义。**语义碰撞裁决留痕**：fatal 全锁使「停核期间经 UI 加项目看降级提示」不可达（rail 连同入口 inert），deployment 旅程移除该步并注明，降级注册契约由 API 级 `projects_add_degraded_when_core_unreachable`（201 + degraded.cause）承载，两 spec 文本均无需改动。回归：cargo 587 全绿、vitest 454 全绿、detached 套件 4 passed。 |
| 2026-09-16 | add-core-reachability-ws-push（工作树实施，spec-go 过渡提交） | 未提交 | 前端两处 5s `/api/summary` 轮询（app-shell 横幅 + composer 提交门）退役，core 通道可达性改 WS 推送：trait 新增 `reachability_updates()` 广播通道（channel 后端 `set_status()` 收口发布、真翻转才发；in-process/fake 走默认立即关闭接收端）；协议方法 `core.reachability.get`（当前态）+ `core.reachability` 翻转 Notification（payload 与 `/api/summary.reachability` 同形）；订阅权上收 app-shell（`sebas:ws-state` connected 时 get 初始态 + 订阅翻转），composer 经 property 链消费下传状态，提交门判定单一出处。证据：`core_channel/tests.rs` 3 例（翻转序列/零重复帧/startup 富化）、`ws_test.rs` FlipBackend 集成（get 当前态/翻转推帧/恢复推 ok:true）、前端 shell+composer 单测（get 初始化/翻转更新/重连收敛/无 `setInterval`+`api.summary` 残留守卫）；浏览器套件 69-70/72（1-2 例串行多播 flaky 隔离全绿，失败点跨 run 漂移为既有遗留类）。「停 core→横幅/禁用翻转」旅程由 Rust FlipBackend 集成测试承载（浏览器级无停 core 旅程，留待呈现层 change 需要时补）。 |
| 2026-09-16 | add-ws-rpc-protocol（工作树实施，spec-go 过渡提交） | 未提交 | `/ws` 协议层落地：三帧封套（Request/Response/Notification）+ 可换 codec 缝（Rust `WsCodec` trait / TS `WsFrameCodec`，JSON 首实现）；7 种既有事件帧迁入 Notification（method=原 type、params 字段名逐字保真），裸帧退役；服务端 `ping` 自证 + unknown_method 拒单不断连；客户端 `request()` id 关联/超时/断线批量拒付。证据：`sebas-webui/tests/ws_test.rs`（集成：id 回显、unknown_method、畸形帧容忍）+ `ws_rpc_contract_test.rs`（codec 可换性两侧、7 事件 params 保真、视图字段抽查）+ 前端 `ws.test.ts` 18 例（关联/超时/拒付/乱序/换 codec 字节级差异）；进程级 e2e `turn_appends_stream_over_ws` 解封改断言后单跑绿（真实二进制 turn.append 以 Notification 到达）。回归：cargo 584+ 全绿、vitest 414 全绿。已知串行套件遗留：session-roundtrip 全量下受前序旅程在途回合事件多播影响（操作者在途 spec 注释自述，隔离跑 1.9s 绿，与本 change 无关）。 |
| 2026-09-12 | 套件卫生加固轮（工作树实施） | fix/testsuite-hardening | 四件套：① Sandbox 每个顶层子进程自成进程组、SandboxDir 析构 killpg 整树（堵 sebas-gc7 孙进程泄漏，e2e 全程 0 残留）；② tasks.py `_sweep_orphan_test_processes` 孤儿清扫（Rust 套件路径直接杀、sbtestsuite 仅目录已消失时杀）+ detached teardown TERM→宽限→KILL 升级（收口 sebas-oo2）+ `.tests-failed` 保留现场豁免（此前被同次 finally 清理秒删）；③ submit-control 两条流式用例把 working 轮询提前到 goto 之前（冷窗口 flake 根修）；④ deployment 旅程自足化——起点自建会话深链聚焦（原先依赖字母序在前的 approval-detached 留下的服务端焦点指针，单跑 `--case deployment` 必挂）、降级注册改用每次尝试唯一的子目录（重试 attempt 撞 409）、恢复后新建会话重建聚焦（重启后深链会话暂离 summary、convergence 清 deepLinkKey 不回头——产品面登记 sebas-5mj/sebas-an1）。**附带发现并修复沙箱隔离违例**：archive.json 落在 `SEBAS_HOME`（~/.sebas）而沙箱 env 未覆盖，Rust 套件与 webui 沙箱长期读写操作员真实归档（93 条测试污染，已备份为 `archive.json.polluted-backup-20260912` 后清空）；两个 harness 现均注入 `SEBAS_HOME`+`SEBAS_ARCHIVE_PATH`，全量套件后真实归档保持 0 条。回归：e2e 19/19、acceptance 9/9、webui 58+3+4 全绿（deployment 单跑与全量均过）。 |
| 2026-09-11 | 旅程加固波（交付终验前，工作树实施） | 未提交（feat/webui 工作树） | 对照三个已实现 change（rail-declutter-unread、workbench-interaction-polish、revamp-settings-nav-and-models-editor）盘点 Scenario→层覆盖缺口，补 4 条浏览器级旅程：未读徽标跨层旅程（`unread-badge.spec.ts` 新 spec：真实回复→API msg_count→徽标免刷新亮起→聚焦清零+锚落 localStorage，覆盖 session-unread-badge 三个此前仅组件单测的 scenario）；History 倒序浏览器旅程（D7，此前仅前端单测）；设置分区导航 IA 旅程（分区顺序/分隔线/About 压底/缺省聚焦/记忆/旧值回退，revamp 后仅沙箱目检）；编辑器抓取→保存→创建对话框免重启接缝旅程（设置↔工作台接缝，此前两半各测各的）。发现并记录实施期问题 #8（rail 行名跟随最新 prompt）。testsuite-webui 61→65（58+3+4）全绿。 |
| 2026-09-11 | rail-declutter-unread（工作树实施） | 未提交（feat/webui 工作树） | rail 收敛 + 未读徽标：Inbox 分组移除（无项目会话不再进 rail，API 仍可达）；移除项目遇非归档会话改 typed rejection 409（废除「迁移 Inbox」承诺，`session_endpoints_test` 两例 + browser 旅程）；项目/会话行操作收敛 `…` 菜单（wa-dropdown）；会话行新增未读徽标（服务端 `msg_count` = 可见回复段口径，dispatch 引擎 `count_chat_messages` 派生 + native 后端 flush 处累计；浏览器 localStorage 游标 `unread-cursor.ts` 与 seam 同锚）；会话名改 prompt_preview（40 码点截断）；History 倒序；composer 创建强制选项目。浏览器用例改写 6 处（projects 1.2 分支呈现按 D8 改写）+ 新增 1 例（主 config 42→43）；全套件 3 配置 50 例全绿（46+3+4）。 |

| 2026-09-11 | 五 change 归档复核（make-core-own-provider-data、add-fetch-models、redesign-provider-models-settings、workbench-turn-queue、workbench-conversation-view） | specs 同步在工作树（归档目录 `openspec/changes/archive/2026-09-11-*`） | 五簇 requirement 级全量重数：117/117 = 100%（豁免 1 条不计分母）。分母 100 → 117：五 change 新增 +7（① Pending submissions observable/manageable；② Core owns provider and model data、Model entries carry capability tags；③ Pending submissions stack、conversation render、single surface；⑤ Model list fetch over the channel）；上期基数后归档补入账 +10（③ agent-workbench +6：审计 `982b4cc` rail/placeholder 三条 + wire-fix `bad057e` 三条；④ +4：project-session-actions rail 三条 + state-store Runtime state boundaries）；router-admin-api REMOVED 4 条退役出账（非核心面）。证据行全面改写（router CRUD/探测/默认值证据迁到 webui BFF + core store + 通道契约面；turn-queue/conversation-view 新 scenario 落行）。本期两处真缺口当场补测收口：E `core_owned_provider_reaches_router_without_restart`（router 订阅 core provider 数据的进程级闭环，此前零覆盖）+ `project_default_agent_follows_last_use` 与前端预选单测。树形账本 41 → 49 行（conversation/pending-stack 新 spec）。实施清单见各归档 change `tasks.md`。 |
| 2026-09-08 | raise-core-coverage-to-90 | `feat/raise-core-coverage-to-90`（基数：fail-fast `5eb1d85`、harden `52847f1`、cover-channel `f14767e`） | 核心集门槛 80%→90% 且扩为五簇（新增 ⑤ 通道与监督 = core-session-channel + watchdog，界定写死进主 spec「核心功能集界定」）；五簇 requirement 级全量重数入账（100/100 = 100%，豁免 1 条不计分母）；三期变更新用例证据归行（fail-fast → watchdog/webui/testsuite 行；harden → ⑤/webui/④/testsuite 行；cover-channel → ⑤/webui 行）；缺口清单收口（detached 审批、projects_branch、watchdog 监督旅程均已有交付命中；spawn-fail 进程级注入转豁免）；复核 grep 清单落 tasks 1.1。实施清单为本期 `tasks.md`。 |
| 2026-09-09 | harden-core-channel-deployment | `feat/harden-core-channel-deployment`（694c2a5 起） | 核心通道部署加固：core 无条件自动装配（生成密钥写 `<config dir>/core.secret` 0600，env 优先）、客户端每次连接 env→文件发现（轮换自愈）、ready 后移至 bind 成功后（bind 失败→75）、im/router/webui 订阅侧共享 resolver、webui 全局核心不可达横幅 + 项目注册降级标记/提示。证据：E `no_secret_assembly_end_to_end` / `secret_rotation_self_heal_across_core_restart` / `watchdog_supervised_core_recovery`（`testsuite_e2e_test`，3 连绿 11 passed ×3）；browser `deployment.spec.ts` 部署韧性旅程（detached 双进程，3 连绿）；`testsuite-webui/tests/helpers/detached.ts` 双进程 fixture（交付物，cover-B 复用）。实施清单为本期 `tasks.md`。 |
| 2026-09-07 | fail-fast-on-startup-errors | `feat/fail-fast-startup-errors`（2e4d952 起，5.3 收尾 commit 为该分支末端） | 启动失败 fail-fast 规约：生命周期子命令启动失败统一退出码 75 + stderr 末行 `startup-failure:` 摘要 + `SEBAS_STARTUP_ERROR_FILE` 覆盖写；watchdog `[watchdog] max_spawn_failures`（默认 3）→ 服务终态 `failed-startup` + watchdog 整体退出 75；rollback 失败不再 silently continue；`sebas ctl status` 新增 `startup_failure` 摘要、`/api/summary` 的 `reachability.cause` 富化；webui web_spawn 失败 inline 进 transcript（spawn-failed 状态 + error 事件 + 相邻同类合并计数）。证据：E `startup_failure_core/run`（`testsuite_e2e_test`）、browser「spawn 失败内显」（`errors.spec.ts`）、supervisor 终态策略/`SpawnFailurePolicy` 单测。实施清单为本期 `tasks.md`。 |
