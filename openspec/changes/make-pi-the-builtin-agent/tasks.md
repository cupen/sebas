# make-pi-the-builtin-agent — Tasks

> 执行备注（本阶段）：1.1–1.3 / 2.1–2.3 / 3.1–3.2 / 4.1–4.2 / 5.1 / 5.4 / 6.3 已完成。
> **5.2（Playwright first-paint spec）、5.3（acceptance pi 旅程）、6.1（进程级 e2e）、
> 6.2（浏览器 GUI 手验）留后续阶段**（保持未勾选）。3.2 已按允许**降级**（见下）。

## 1. 内置槽位重指向（src/agent_backend.rs）

- [x] 1.1 `DualSessionBackend::spawn_with` / `create_placeholder` 的 `agent == "native"` 分支改为转发 ACP 侧 pi：解析 pi launch 配置（`[acp.agents.pi]` seed / agents-store 行，缺省回退 pi driver 默认）并 spawn，session key 用 web 形态；`is_native`（feishu/agent-* 前缀）仅保留给 legacy 会话路由。验证：单测断言 agent=native 创建的会话落在 pi driver、key 为 web 形态；legacy key 仍路由内核。
  - 实现：`spawn_with`/`create_placeholder` native 分支转发 `self.acp.spawn_with(..., "pi", ...)`（web key）；`agent_store::resolve_spawn_config` 对 `pi` 恒可解析（config∪store 优先，缺则合成缺省 `{driver:"pi"}`＝`AcpPiConfig::default()`，绝不 unknown-agent）；`is_native` 保留 feishu/agent-* 前缀路由。测试：`dual_routes_on_backend_hint_and_prefix`（改）、`legacy_native_key_still_routes_to_kernel`（新）、`agent_store::builtin_pi_config_*`（新 3 条）。
- [x] 1.2 `execution_bodies` 的 native 条目改读 pi 可达性（复用 `discover_agent` 对 pi 的探测 + `pi_auth_cause`），native.reachability 不再读 `NativeAgentBackend::reachability`。验证：单测——pi 二进制在场（PATH 垫 fake pi）时 native 条目 ok=true；不在场时 ok=false 且 cause 指向安装面。
  - 实现：`DualSessionBackend::native_probe`（`AgentKindSource`，run.rs 经 `agent_store::builtin_pi_probe_source` 注入）→ `native_body()` 用 `agent_kinds::discover_agent` 探测；在场 ok=true（未登录时 `pi auth check` 状态作 advisory cause），不在场 ok=false + `pi_install_hint()`（安装面文案，不含 env 标识）。测试：`native_body_follows_pi_binary_presence`（新）。
- [x] 1.3 飞书 native 桥与 `native_manager()` 消费方收口：新会话不再进内核，legacy resume 路径保留。验证：既有 feishu native 用例改写后通过；归档 restore 用例通过。
  - 实现：`DispatchNativeBridge::is_native` 只认「已登记会话 ∨ feishu/agent-* 前缀」；`[feishu] native_default` 键保留兼容但不再影响路由；`native_manager()` 保留给 legacy。测试：`tests/feishu_native_webui_test.rs` 改写为 legacy 旅程 + 新建不再进内核两条。

## 2. pi 凭据注入（src/session_boot.rs / src/spawn_env.rs）

- [x] 2.1 `spawn_overrides` 放开 pi：复用 `resolve_spawn_overrides`/`compute_provider_resolution`，把 provider 解析翻译为标准 env（Anthropic 协议 → `ANTHROPIC_API_KEY`+`ANTHROPIC_BASE_URL`；OpenAI chat 协议 → `OPENAI_API_KEY`+`OPENAI_BASE_URL`）；无法映射的协议跳过并记录。验证：单测覆盖 anthropic/openai 两形状、api_key 明文与 api_key_env 两来源、未知协议跳过。
  - 实现：`spawn_env::compute_pi_provider_env`（复用 `direct_resolution_from_overlay` 解析）+ `resolve_pi_spawn_env`；`session_boot::spawn_overrides` 对 `kind=="pi"` 注入标准 env、无 argv 追加。测试：`pi_anthropic_provider_emits_standard_env`、`pi_openai_provider_emits_standard_env_from_api_key_env`、`pi_unmappable_provider_is_skipped_with_diagnostic`、`pi_router_mode_emits_router_anthropic_env`、`pi_off_mode_emits_no_env`、`pi_kind_injects_standard_env_without_args`。
- [x] 2.2 同名 env 冲突规则：默认 provider（provider-defaults）优先，其余同协议 provider 不注入，跳过记录进 spawn 诊断。验证：单测断言双 provider 冲突时默认者胜、诊断含跳过事实。
  - 实现：枚举顺序 default_provider 优先 + `occupied` 家族集合；默认 provider 取 `state.mode.Direct.provider` 或 `default_selection.provider`（provider-defaults 的存储真源）。测试：`pi_default_provider_wins_standard_env_collision`。
- [x] 2.3 确认 sebas 不写 pi 的 `models.json`/auth 存储（现状即不写，钉住回归）。验证：单测断言 spawn 前后 pi home 内无 sebas 写入的文件。
  - 测试：`tests/pi_driver_integration_test.rs::pi_spawn_never_writes_into_pi_home`（pinned `$HOME/.pi`，spawn+回合后目录不存在）。

## 3. 模型同步收尾（sebas-acp）

- [x] 3.1 PiDriver 握手期把 `get_available_models` 应答装载进 `model_index`（driver.rs:506 现为空表），使裸 id 提交可解析。验证：既有桩级单测扩展——握手后裸 id set_model 成功。
  - 实现：`Spawned.models`（握手 `(provider,id)` 清单）→ `run_loop` 入参 → `model_index` 反查表（**id→provider**）。测试：`bare_model_id_resolves_after_handshake`（driver 桩级 + pi_driver_integration_test 各一条）。
- [x] 3.2 sebas 目录 → pi 词表的字面匹配桥：创建请求携带裸 model 且 provider 名与 pi provider id 一致时拼 `provider/id`；不一致不拼（pi 缺省起步，composer chip 兜底）。验证：单测覆盖一致/不一致两分支。
  - **降级（design 允许）**：创建对话框不做桥接。原因——创建 wire 只携带裸 model（provider 仅前端 last-used 记忆），且 pi 的 `provider/id` 词表在子进程握手前不存在；spawn 期拼 `provider/id` 需要在建会话后、首 prompt 前重发 set_model，而「字面一致」判定所需的 pi 词表只有 spawn 后才可得，任何预拼都是猜测（违反诚实原则）。既有 **composer chip**（`get_available_models` 的 `provider/id` 面 + `set_model` 往返）完整覆盖会话内模型同步需求；3.1 使裸 id 在握手后也可解析，进一步兜住创建面。前端测试：`workbench-composer.test.ts` 的 native 段（4.2）。

## 4. 前端（sebas-webui/frontend）

- [x] 4.1 catalog 内置行 display 改 `PI`（后端 api.rs:821）；新会话对话框 native 不可用文案改「安装 pi」指引（`agentUnavailableLabel` native 分支）；Settings → Agents 内置行徽标改「内置 · PI」、尾注同步。验证：Vitest 组件测试断言新文案与可达/不可达两态。
  - 实现：api.rs display → `PI`；`new-session-dialog.ts` native 分支改「未安装 pi——到「设置 → Agent」一键安装 pi」；`settings-modal.ts` 徽标「内置 · PI」+ 尾注「内置 PI agent（不可删除）」+ 段描述。测试：`new-session-dialog.test.ts`（改 2 条）、`agent_kinds_test.rs`（display 断言）。
- [x] 4.2 composer 对 native(pi) 会话的模型 chip 确认走 `available_models`（既有），无假「已选」态。验证：Vitest 断言未选模型时无 selected 标记。
  - 测试：`workbench-composer.test.ts` 新增 `native (PI) session model chip (4.2)` 两条（未选不落 selected；pi 自广告 `provider/id` 词表呈现）。

## 5. 测试同步与回归

- [x] 5.1 `sebas-webui/tests/agent_kinds_test.rs`、`gateway_bff_test.rs`（native 400 族）、`api_endpoints_test.rs` 的 native 断言改写为新语义。验证：`rtk cargo test -p sebas-webui` 绿。
  - 改动：`agent_kinds_test.rs`（display `PI`、native 可用性/ cause 改 pi 安装面）；`src/agent_backend.rs` 单测（native 重指向、可达性）；`feishu_native_webui_test.rs`。`gateway_bff_test.rs`/`api_endpoints_test.rs` 的 native 断言是「保留 id 的目录/角色门禁」，语义未变（native 仍是保留 id），无需改。
- [ ] 5.2 `tests/testsuite-webui/tests/first-paint.spec.ts` 的 native 置灰/env 文案断言改写为「pi 未装 → 置灰 + 安装指引」。验证：浏览器套件该 spec 绿。**（后续阶段）**
- [ ] 5.3 `tests/testsuite_acceptance_test.rs` 的 native 内核旅程改写为 pi 旅程（fake pi 桩经 PATH 垫），legacy resume 旅程保留内核路径。验证：`invoke testsuite-acceptance` 绿。**（后续阶段）**
- [x] 5.4 全量回归：`rtk cargo test`、`cd sebas-webui/frontend && rtk pnpm test`、clippy/fmt 无新告警。验证：退出码 0。
  - 结果：`rtk cargo test` = 832 passed / 104 ignored；`rtk cargo test -p sebas-webui` = 422 passed；`pnpm test` = 1012 passed。clippy/fmt 与改动前计数一致（仅既有告警/漂移，新代码零新增）。

## 6. 收口

- [ ] 6.1 进程级验收：沙箱（SEBAS_HOME 钉定 + PATH 垫 fake pi + fake node/npm）旅程 = 启动 → 新建会话选 native(PI) → composer 出现 pi 自广告模型面 → 切模型 set_model 生效 → provider env 注入断言（fake pi 记录 env）。验证：`invoke testsuite-e2e --case <name>` 通过。**（后续阶段）**
- [ ] 6.2 浏览器 GUI 验收：`invoke testsuite-webui-sandbox` 手验「启动 → 内置 PI 可选 → 建会话 → 切模型」，截图与清单落 `verification/`。验证：沙箱会话记录。**（后续阶段）**
- [x] 6.3 `openspec validate make-pi-the-builtin-agent --strict` 通过；specs 逐场景对账（尤其「未装 pi 指引安装」「未登录仍可选」「凭据 spawn 期语义」「legacy resume 保留」）。
  - 结果：`openspec validate make-pi-the-builtin-agent --strict` → valid。逐场景：未装 pi 指引安装（1.2/4.1）、未登录仍可选（1.2 advisory cause）、凭据 spawn 期语义（2.1 env 注入 + 无热更新）、legacy resume 保留（1.1/1.3）均落地。