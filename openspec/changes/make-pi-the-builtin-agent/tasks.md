# make-pi-the-builtin-agent — Tasks

## 1. 内置槽位重指向（src/agent_backend.rs）

- [ ] 1.1 `DualSessionBackend::spawn_with` / `create_placeholder` 的 `agent == "native"` 分支改为转发 ACP 侧 pi：解析 pi launch 配置（`[acp.agents.pi]` seed / agents-store 行，缺省回退 pi driver 默认）并 spawn，session key 用 web 形态；`is_native`（feishu/agent-* 前缀）仅保留给 legacy 会话路由。验证：单测断言 agent=native 创建的会话落在 pi driver、key 为 web 形态；legacy key 仍路由内核。
- [ ] 1.2 `execution_bodies` 的 native 条目改读 pi 可达性（复用 `discover_agent` 对 pi 的探测 + `pi_auth_cause`），native.reachability 不再读 `NativeAgentBackend::reachability`。验证：单测——pi 二进制在场（PATH 垫 fake pi）时 native 条目 ok=true；不在场时 ok=false 且 cause 指向安装面。
- [ ] 1.3 飞书 native 桥与 `native_manager()` 消费方收口：新会话不再进内核，legacy resume 路径保留。验证：既有 feishu native 用例改写后通过；归档 restore 用例通过。

## 2. pi 凭据注入（src/session_boot.rs / src/spawn_env.rs）

- [ ] 2.1 `spawn_overrides` 放开 pi：复用 `resolve_spawn_overrides`/`compute_provider_resolution`，把 provider 解析翻译为标准 env（Anthropic 协议 → `ANTHROPIC_API_KEY`+`ANTHROPIC_BASE_URL`；OpenAI chat 协议 → `OPENAI_API_KEY`+`OPENAI_BASE_URL`）；无法映射的协议跳过并记录。验证：单测覆盖 anthropic/openai 两形状、api_key 明文与 api_key_env 两来源、未知协议跳过。
- [ ] 2.2 同名 env 冲突规则：默认 provider（provider-defaults）优先，其余同协议 provider 不注入，跳过记录进 spawn 诊断。验证：单测断言双 provider 冲突时默认者胜、诊断含跳过事实。
- [ ] 2.3 确认 sebas 不写 pi 的 `models.json`/auth 存储（现状即不写，钉住回归）。验证：单测断言 spawn 前后 pi home 内无 sebas 写入的文件。

## 3. 模型同步收尾（sebas-acp）

- [ ] 3.1 PiDriver 握手期把 `get_available_models` 应答装载进 `model_index`（driver.rs:506 现为空表），使裸 id 提交可解析。验证：既有桩级单测扩展——握手后裸 id set_model 成功。
- [ ] 3.2 sebas 目录 → pi 词表的字面匹配桥：创建请求携带裸 model 且 provider 名与 pi provider id 一致时拼 `provider/id`；不一致不拼（pi 缺省起步，composer chip 兜底）。验证：单测覆盖一致/不一致两分支。

## 4. 前端（sebas-webui/frontend）

- [ ] 4.1 catalog 内置行 display 改 `PI`（后端 api.rs:821）；新会话对话框 native 不可用文案改「安装 pi」指引（`agentUnavailableLabel` native 分支）；Settings → Agents 内置行徽标改「内置 · PI」、尾注同步。验证：Vitest 组件测试断言新文案与可达/不可达两态。
- [ ] 4.2 composer 对 native(pi) 会话的模型 chip 确认走 `available_models`（既有），无假「已选」态。验证：Vitest 断言未选模型时无 selected 标记。

## 5. 测试同步与回归

- [ ] 5.1 `sebas-webui/tests/agent_kinds_test.rs`、`gateway_bff_test.rs`（native 400 族）、`api_endpoints_test.rs` 的 native 断言改写为新语义。验证：`rtk cargo test -p sebas-webui` 绿。
- [ ] 5.2 `tests/testsuite-webui/tests/first-paint.spec.ts` 的 native 置灰/env 文案断言改写为「pi 未装 → 置灰 + 安装指引」。验证：浏览器套件该 spec 绿。
- [ ] 5.3 `tests/testsuite_acceptance_test.rs` 的 native 内核旅程改写为 pi 旅程（fake pi 桩经 PATH 垫），legacy resume 旅程保留内核路径。验证：`invoke testsuite-acceptance` 绿。
- [ ] 5.4 全量回归：`rtk cargo test`、`cd sebas-webui/frontend && rtk pnpm test`、clippy/fmt 无新告警。验证：退出码 0。

## 6. 收口

- [ ] 6.1 进程级验收：沙箱（SEBAS_HOME 钉定 + PATH 垫 fake pi + fake node/npm）旅程 = 启动 → 新建会话选 native(PI) → composer 出现 pi 自广告模型面 → 切模型 set_model 生效 → provider env 注入断言（fake pi 记录 env）。验证：`invoke testsuite-e2e --case <name>` 通过。
- [ ] 6.2 浏览器 GUI 验收：`invoke testsuite-webui-sandbox` 手验「启动 → 内置 PI 可选 → 建会话 → 切模型」，截图与清单落 `verification/`。验证：沙箱会话记录。
- [ ] 6.3 `openspec validate make-pi-the-builtin-agent --strict` 通过；specs 逐场景对账（尤其「未装 pi 指引安装」「未登录仍可选」「凭据 spawn 期语义」「legacy resume 保留」）。
