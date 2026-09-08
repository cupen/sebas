# 验收矩阵（testsuite-acceptance）

> 账本规则：每个能力一行，requirement 簇逐条标注命中证据（验收旅程 J*、进程级 e2e E*、
> 既有测试引用）或豁免（注明 cause）。未命中且未豁免 = 缺口（⚠️）。
> **核心功能集**（80% 硬指标）：① 会话管理 = session-lifecycle + session-persistence +
> acp-session-mapping；② models 管理 = acp-model-selection + router-model-aliases +
> provider-management；③ agent workbench 相关 = agent-workbench + permission-flow；
> ④ 项目管理 = project-session-actions + state-store(projects) + webui(projects 面)。
> 核心集增删必须留变更说明。
>
> 旅程用例：`invoke testsuite-acceptance`（`tests/testsuite_acceptance_test.rs`）
> 冒烟用例：`invoke testsuite-e2e`（`tests/testsuite_e2e_test.rs`）
> 浏览器级旅程：`invoke testsuite-webui`（`tests/testsuite-webui/`，Playwright + chromium，
> 旅程账本见 `tests/testsuite-webui/README.md` 与 `openspec/changes/add-webui-playwright-tests`）

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

全量 247 条中：豁免 22 条（飞书真实传输、opencode CLI；原"浏览器级 UI 渲染"
豁免已由 `testsuite-webui-browser` 旅程套件接替），
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
| cli-service | ✅ | `config_test`、`config_env_test`、`daemon_path_repro_test`、src `cli` 内联测试；E: startup_failure（fail-fast-on-startup-errors：garbage config → 退出码 75 + stderr 末行 `startup-failure:` 摘要 + `SEBAS_STARTUP_ERROR_FILE`，core 与 run 双形态，`testsuite_e2e_test`）|
| core-session-channel | ✅ | src `core_channel/tests.rs`（协议往返/双路由/密钥；harden-core-channel-deployment：自动装配时序/密钥文件 0600/轮换自愈/bind 失败拒绝重绑）；E: startup/reachability/wrong-secret/restart + startup_failure 75 契约 + no_secret_assembly（无 env 自动装配 + 0600 密钥文件 + 会话往返）/ secret_rotation_self_heal（kill→同 config 新钥重启→不重启的 webui 自愈）/ watchdog_supervised_core_recovery（监督形态杀 core 子进程→自动重启→webui 恢复）（`testsuite_e2e_test`）|
| feishu-bridge | 🚫+✅ | 真实 WS/HTTP 传输豁免（需真实凭据）；进程内注入面 ✅（`feishu_native_webui_test`、ws_loop 内联测试）|
| feishu-cards | ✅+🚫 | 卡模型/流式节流/轮转 ✅（`card_stream_e2e_test`、sebas-feishu 内联）；飞书端渲染 🚫 豁免 |
| feishu-option | ✅ | `feishu_native_webui_test`、config 测试 |
| feishu-reactions | ✅ | src `reactions.rs` 内联测试 |
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
| watchdog | ✅+⚠️ | src `watchdog.rs`/`upgrade.rs` 内联测试、`upgrade_dev_test`；fail-fast-on-startup-errors：spawn 失败终态策略单测（N=1/3/10 边界、ready 清零、early-fatal 计入、post-ready 崩溃不计入、rollback 失败终态化）、E: startup_failure 75 契约；harden-core-channel-deployment 5.3 补进程级监督恢复旅程（SIGKILL core 子进程 → supervisor 自动重启 → webui 恢复 reachable）；watchdog「连续 3 次 spawn fail → 整体退出 75」进程级注入仍 ⚠️（非阻塞，真实二进制下 current_exe spawn 无法注入失败——见缺口清单 3）|
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

> Settings 语义修正（fix-settings-menu-and-services-semantics，spec 改动 `1e4a807`）：
> S1 改写为 `/api/admin/services` 响应驱动（响应即真源，不枚举具体服务）、S6 新增裸
> core 退化覆盖（横幅 + 零行 + 重启 disabled）；实施清单见
> `openspec/changes/fix-settings-menu-and-services-semantics/tasks.md` §4；本表 35 行 =
> 全套件 `it` 总数 35（含新增 S6）。

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
| 飞书端卡片渲染 | 同上 | 卡片 JSON 生成单测（`card_stream_e2e_test`）|
| 浏览器级 workbench UI 渲染 | ~~簇 C 另行立项~~ 已由 testsuite-webui-browser 落地（`invoke testsuite-webui`）| `tests/testsuite-webui/` 旅程套件（Playwright + fake-claude 沙箱）|
| opencode-agent 真实代理 | 需真实 opencode CLI | AcpDriver 抽象层测试 |
| agent-bench 真实模型跑分 | 需真实凭据 | bench 断言逻辑单测 |

## 缺口清单（未命中且未豁免）

1. **detached 审批通道旅程**（permission-flow / agent-workbench）：审批事件经核心通道推送到 detached webui 的接线属进行中的 `wire-webui-sebas-agent-e2e` 任务 1.3；落地后补 `allow / deny` 两条旅程（harden-core-channel-deployment 5.4 已交付可复用双进程沙箱 fixture：`tests/testsuite-webui/tests/helpers/detached.ts` + `TESTSUITE_MODE=detached` harness，B1 直接复用不重写 harness）。既有进程内审批测试当前作为命中证据。
2. **项目视图工作副本上下文**（agent-workbench）：`projects_branch` 端点无端点测试/旅程。
3. **watchdog 监督循环进程级旅程**（watchdog）：崩溃退避/自动回滚仅有单元面；进程级需 watchdog 双进程沙箱。harden-core-channel-deployment 5.3 已收窄：`testsuite_e2e_test::watchdog_supervised_core_recovery`（`sebas run` 监督形态拉起 core+webui、SIGKILL core 子进程 → supervisor 自动重启新 pid → webui 恢复 reachable）落地，崩溃→自动重启→恢复的进程级循环有据；fail-fast-on-startup-errors 补齐的 spawn 失败终态策略（N 边界/清零/early-fatal 计入，supervisor 单测）与启动失败 75 契约（`startup_failure_*` e2e）不变；「受管服务连续 3 次 **spawn fail** → watchdog 整体退出 75」仍缺进程级注入手段——spawner 用 `current_exe()` 派生子进程，真实二进制下 spawn 系统调用无法失败，早期退出又全部收敛为退出码 75（Degraded 语义）或 crash 退避（计数清零），故该子路径保留单元面覆盖（fake spawner 直驱监督循环 + 终态事件通道）。
4. **record/replay 独立旅程**（replay-debug）：既有 `record_test`/`replay_test` 命中；端到端旅程待补。

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
| 2026-09-09 | harden-core-channel-deployment | `feat/harden-core-channel-deployment`（694c2a5 起） | 核心通道部署加固：core 无条件自动装配（生成密钥写 `<config dir>/core.secret` 0600，env 优先）、客户端每次连接 env→文件发现（轮换自愈）、ready 后移至 bind 成功后（bind 失败→75）、im/router/webui 订阅侧共享 resolver、webui 全局核心不可达横幅 + 项目注册降级标记/提示。证据：E `no_secret_assembly_end_to_end` / `secret_rotation_self_heal_across_core_restart` / `watchdog_supervised_core_recovery`（`testsuite_e2e_test`，3 连绿 11 passed ×3）；browser `deployment.spec.ts` 部署韧性旅程（detached 双进程，3 连绿）；`testsuite-webui/tests/helpers/detached.ts` 双进程 fixture（交付物，cover-B 复用）。实施清单为本期 `tasks.md`。 |
| 2026-09-07 | fail-fast-on-startup-errors | `feat/fail-fast-startup-errors`（2e4d952 起，5.3 收尾 commit 为该分支末端） | 启动失败 fail-fast 规约：生命周期子命令启动失败统一退出码 75 + stderr 末行 `startup-failure:` 摘要 + `SEBAS_STARTUP_ERROR_FILE` 覆盖写；watchdog `[watchdog] max_spawn_failures`（默认 3）→ 服务终态 `failed-startup` + watchdog 整体退出 75；rollback 失败不再 silently continue；`sebas ctl status` 新增 `startup_failure` 摘要、`/api/summary` 的 `reachability.cause` 富化；webui web_spawn 失败 inline 进 transcript（spawn-failed 状态 + error 事件 + 相邻同类合并计数）。证据：E `startup_failure_core/run`（`testsuite_e2e_test`）、browser「spawn 失败内显」（`errors.spec.ts`）、supervisor 终态策略/`SpawnFailurePolicy` 单测。实施清单为本期 `tasks.md`。 |
