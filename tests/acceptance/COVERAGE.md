# 验收矩阵（acceptance-suite）

> 账本规则：每个能力一行，requirement 簇逐条标注命中证据（验收旅程 J*、进程级 e2e E*、
> 既有测试引用）或豁免（注明 cause）。未命中且未豁免 = 缺口（⚠️）。
> **核心功能集**（80% 硬指标）：① 会话管理 = session-lifecycle + session-persistence +
> acp-session-mapping；② models 管理 = acp-model-selection + router-model-aliases +
> provider-management；③ agent workbench 相关 = agent-workbench + permission-flow；
> ④ 项目管理 = project-session-actions + state-store(projects) + webui(projects 面)。
> 核心集增删必须留变更说明。
>
> 旅程用例：`invoke accept`（`tests/acceptance_suite_test.rs`）
> 冒烟用例：`invoke e2e`（`tests/core_flow_e2e_test.rs`）

## 矩阵图例

- ✅ = 命中（含证据）　⚠️ = 缺口（未命中且未豁免）　🚫 = 豁免（cause）

## 核心功能集统计（达标复核 2026-09-05）

| 核心簇 | requirement 数 | 命中 | 命中率 | 套件内旅程 |
|---|---|---|---|---|
| ① 会话管理 | 13 | 13 | 100% | `session_lifecycle_journey` |
| ② models 管理 | 19 | 18 | 95% | `provider_governance_journey`、`native_agent_turn_via_router_journey` |
| ③ agent workbench 相关 | 21 | 19 | 90% | `workbench_aggregate_journey` |
| ④ 项目管理 | 12 | 12 | 100% | `projects_session_journey` |
| **核心合计** | **65** | **62** | **95%** | 每簇 ≥1 条 ✓ |

全量 247 条中：豁免 22 条（飞书真实传输、opencode CLI、浏览器级 UI 渲染），
非核心缺口 3 条（见各行 ⚠️）。可沙箱验收面（225 条）命中 216，≈96%（长期方向 ≥90%，非门槛）。

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
| provider-management | /provider 主卡布局 | 🚫 | 浏览器级 UI 渲染（豁免，见豁免清单）|
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
| | 项目视图真实工作副本上下文 | ⚠️ | projects_branch 端点缺旅程/端点测试（非阻塞缺口）|
| | 原生内核会话执行 | ✅ | J: `native_agent_turn_via_router_journey`（E 级）|
| | 原生内核 gated call 审批 | ✅ | src `core_channel/tests.rs`（审批往返/fail-closed）|
| | 目录浏览器加项目 | ✅ | sebas-webui `api_endpoints_test`（browse-dirs）|
| | 无 prompt 新会话 | ✅ | J: `workbench_aggregate_journey`（占位会话）|
| | 会话归档 | ✅ | sebas-webui `api_endpoints_test`（archive 路由）|
| | 历史组即归档 | ✅ | 同上 |
| | 归档过期 | ✅ | src `archive.rs` 内联测试 |
| permission-flow | Hook 驱动权限请求 | ✅ | `permission_flow_test`；fake-claude "perm" 场景 |
| | 三种决定结果 | ✅ | `permission_flow_test`、sebas-webui `acp_permission_roundtrip_test` |
| | allowlist 命中自动批准 | ✅ | `permission_flow_test` |
| | allowlist 作用域与生命周期 | ✅ | `permission_flow_test` |
| | 迟到点击处理 | ✅ | src `core_channel/tests.rs`（typed rejection）|
| | 无应答者 fail-closed | ✅ | src `core_channel/tests.rs`；E: detached 审批通道旅程待 wire-webui 1.3（⚠️ 缺口，见备注）|

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
| cli-service | ✅ | `config_test`、`config_env_test`、`daemon_path_repro_test`、src `cli` 内联测试 |
| core-session-channel | ✅ | src `core_channel/tests.rs`（协议往返/双路由/密钥）；E: startup/reachability/wrong-secret/restart（`core_flow_e2e_test`）|
| feishu-bridge | 🚫+✅ | 真实 WS/HTTP 传输豁免（需真实凭据）；进程内注入面 ✅（`feishu_native_webui_test`、ws_loop 内联测试）|
| feishu-cards | ✅+🚫 | 卡模型/流式节流/轮转 ✅（`card_stream_e2e_test`、sebas-feishu 内联）；飞书端渲染 🚫 豁免 |
| feishu-option | ✅ | `feishu_native_webui_test`、config 测试 |
| feishu-reactions | ✅ | src `reactions.rs` 内联测试 |
| gateway-admin-api | ✅ | `admin_test`；J: provider_governance（/admin/stats）|
| gateway-auth-rate-limit | ✅ | `auth_test`、`rate_limit_test`；J: `gateway_downstream_auth_journey` |
| gateway-core | ✅ | `proxy_smoke_test`、`contract_test`、`debug_provider_test`、`failure_test`；E: gateway debug |
| gateway-metrics | ✅ | sebas-router `metrics` 测试；J: /admin/stats 200 |
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
| watchdog | ✅+⚠️ | src `watchdog.rs`/`upgrade.rs` 内联测试、`upgrade_dev_test`；监督循环的进程级旅程 ⚠️（非阻塞）|
| webui | ✅ | sebas-webui 全套端点测试；E: detached 双进程启动/健康/重连（`core_flow_e2e_test`）|

## 豁免清单（cause + 替代验证）

| 面 | cause | 替代验证 |
|---|---|---|
| 飞书真实 WS/HTTP 传输 | 需真实 app 凭据，沙箱不可得 | 进程内注入级测试（router 派发/出站事件即生产意图）|
| 飞书端卡片渲染 | 同上 | 卡片 JSON 生成单测（`card_stream_e2e_test`）|
| 浏览器级 workbench UI 渲染 | 簇 C 另行立项 | 组件级 vitest + HTTP 面旅程 |
| opencode-agent 真实代理 | 需真实 opencode CLI | AcpDriver 抽象层测试 |
| agent-bench 真实模型跑分 | 需真实凭据 | bench 断言逻辑单测 |

## 缺口清单（未命中且未豁免）

1. **detached 审批通道旅程**（permission-flow / agent-workbench）：审批事件经核心通道推送到 detached webui 的接线属进行中的 `wire-webui-sebas-agent-e2e` 任务 1.3；落地后补 `allow / deny` 两条旅程。既有进程内审批测试当前作为命中证据。
2. **项目视图工作副本上下文**（agent-workbench）：`projects_branch` 端点无端点测试/旅程。
3. **watchdog 监督循环进程级旅程**（watchdog）：崩溃退避/自动回滚仅有单元面；进程级需 watchdog 双进程沙箱（后续扩展 `support::Sandbox`）。
4. **record/replay 独立旅程**（replay-debug）：既有 `record_test`/`replay_test` 命中；端到端旅程待补。

## 实施期发现（只记录不顺手修，见 design Non-goals）

1. **native 会话状态卡在 Queued**：native 回合完成（turn summary 已写、模型调用已完成），但 `src/native_router_bridge.rs` 从不设置 phase=DONE，workbench 状态恒为 "Queued"（models.rs derive：active+"" → Queued）。建议立项修复后，`native_agent_turn_via_router_journey` 的断言可升级为 status_slug=done。
2. **`run --gateway` 忽略 `SEBAS_GATEWAY_LISTEN`**（run.rs:87 写死 127.0.0.1:0）：detached 形态下 `SEBAS_AGENT_GATEWAY_URL` 无法预注入（网关地址只能事后从日志读）。native 走 `SEBAS_AGENT_PROVIDER_BASE_URL` 直连路径作为替代（本套件已覆盖）。
3. **会话状态落盘仅在优雅退出**：硬杀（TerminateProcess）不产生状态转储；Windows 无便携优雅信号，故重启恢复段 unix 门控。
4. **路由状态已入 SQLite**：`[router] state_file`（sessions.json）不再是重启恢复的活性来源，state store DB（sebas.db）承担持久化——矩阵断言已按此更新。
