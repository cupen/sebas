# make-pi-the-builtin-agent — Design

## Context

见 proposal.md「Why」。关键现状（已核实到文件:行号）：

- `DualSessionBackend { acp, native }`（`src/agent_backend.rs:1406`）：native = 进程内内核（`NativeAgentBackend`，包 `sebas_agent::SessionManager` + 直连 LLM client）；pi = ACP 侧 driver（`sebas-acp/src/pi/driver.rs`，spawn `pi --mode rpc`）。
- native 可达性判定在 `build_native_manager`（agent_backend.rs:327-358）：`SEBAS_AGENT_PROVIDER_API_KEY`/`SEBAS_AGENT_ROUTER_URL` 二选一在场，否则 cause「native backend needs …」。`/api/summary` execution_bodies 与 catalog native 行同源（agent_backend.rs:1912-1935、sebas-webui/src/api.rs:814-826，display 硬编码 `"Native Kernel"`）。
- native 槽位路由：`spawn_with`/`create_placeholder` 对 `agent == "native"` 走 `self.native`（agent_backend.rs:1741-1754、1783-1794）；native key = `feishu/agent-{8hex}`。
- **pi 的 set_model 已全链路可用**：webui `POST /api/sessions/{key}/model` → `Out::SendAcp(SetModel)` → PiDriver（`provider/id` 词表，codec.rs:314-323）；模型面来自握手 `get_available_models`（driver.rs:389-413）；裸 id 兜底 `model_index` **握手期不装载**（driver.rs:506，仅 set_model 成功后插入）。
- **pi 凭据零注入是 spec 明文**：`spawn_overrides` 对非 claude 短路（`src/session_boot.rs:80-95`）；pi-agent spec「sebas SHALL NOT 翻译或代写 pi 的凭据与 provider 资产」；add-pi-driver design D5 把「翻译 ProviderResolution → env」列为预留备选。
- provider 解析机制现成：`src/spawn_env.rs` 的 `resolve_spawn_overrides` → `compute_provider_resolution`（读 settings.db provider 行，api_key 明文 > api_key_env）→ claude 的 `ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN` + cover env。
- sebas-router 透传 9 家 provider（anthropic/openai/deepseek/kimi/glm/minimax/ark/dashscope/gemini）——内核 LLM 端点可指向 router，provider 广度不依赖 pi。
- pi 无内置权限系统（pi-agent spec:75-87）：无审批卡、`SetMode` 不支持、工具以子进程权限直跑。
- 未关闭的内核任务：`sebas-w184`（工具进度接线）、`sebas-f9es`（native 审批卡恒空）、`sebas-san`（native mode 生效）等 7 项。

## Goals / Non-Goals

**Goals:**

- 「启动 sebas → 进后台 → 直接用内置 PI 会话」端到端成立：装了 pi 即可选、默认即指向它。
- 操作员在 sebas 侧配置的 provider 凭据对 pi 子进程生效（spawn 期注入标准 env）。
- composer 的模型选择对 native(pi) 会话真实生效（pi 自广告词表 + `set_model` 往返）。
- **可回滚**：不删内核代码与 `agent-core` 规范。

**Non-Goals（设计层补充）:**

- 不给 pi 加审批/沙箱（pi 无权限系统是上游事实；安全隔离按 pi 官方口径走容器化，文档明示）。
- 不做 env 热更新、不写 `models.json`。
- 不清理 105 处主 specs 提及中与行为契约无关的部分。

## Decisions

### D1 形态：native id 保留，底层改指 pi（可逆优先）

`native` 作为保留 id 与内置槽位不变（保住 wire 兼容、「恒在不可删」性质、飞书/归档消费方），但其执行路径改为转发 ACP 侧 pi driver：`spawn_with`/`create_placeholder` 的 `agent == "native"` 分支改解析 pi launch 配置并 spawn pi。display 改 `PI`。

- 否决「删掉 native 槽位、统一到 `pi` 配置行」（B 形态）：会破坏 `native` 恒在语义（agent-settings「内置内核恒在不可删」）、`agent=native` wire 词汇与归档会话路由，破坏面大且不可逆。
- **内核处置：保留代码、摘除接线**（legacy resume 除外，见 D5）——回滚 = 还原路由一处。

### D2 可达性改按 pi 口径；置灰保留、指引改安装面（修正 Q2.2 字面答案）

native 条目的可达性 = **pi 二进制在场**（复用 `discover_agent`/`resolved_binary` 探测），`pi auth check` 非 ready 作为 advisory cause（不阻塞可选）。pi 未安装 → 置灰，文案改为指引「Settings → Agent 一键安装 pi」。

- **修正记录**：用户曾选「未配凭据也可选（放开置灰）」。该前提在 native=pi 后失效——pi 不存在「sebas 凭据」维度，置灰理由变为「pi 未安装」；放开置灰只会让操作员选中一个必然 spawn 失败的选项，违背本仓「诚实拒绝」契约（`agent-workbench`「不可用执行体 SHALL NOT 可选」）。故保留置灰、更换门与指引。
- 好处：凭据维度从内置槽位消失——「装了就能选、没登录也能跑（env key）」，正是用户要的「启动即可用」。

### D3 凭据注入：反转 pi-agent env 条款，复用 claude 的解析机制

`spawn_overrides` 放开 pi：对 pi 会话执行与 claude 同源的 provider 解析（`compute_provider_resolution` 读 settings.db），把结果翻译成 **pi 认识的标准 env 名**——Anthropic 协议 provider → `ANTHROPIC_API_KEY` + `ANTHROPIC_BASE_URL`；OpenAI chat 协议 → `OPENAI_API_KEY` + `OPENAI_BASE_URL`；无法映射的协议跳过（诚实，不硬造）。

- **同名 env 冲突规则**：操作员默认 provider（provider-defaults）优先占用标准 env 名；其余同协议 provider 不注入（limitation 如实记入文档，不静默混淆）。
- **spawn 期语义**：env 只在子进程启动时生效——spawn 后修改 provider 配置不影响在跑会话（spec 显式声明，避免「同步」被误读为热更新）；切换**模型**走 `set_model` 不受此限。
- **models.json / auth 存储仍 pi 自管**：sebas 不写；操作员自定义 provider（pi 侧 models.json 定义）照旧 pi 侧配。
- 否决「只注入会话所选 provider」：创建 wire 只携带裸 model（provider 仅前端记忆），spawn 期无法可靠还原所选 provider；按注册表注入 + 默认优先是确定性的。

### D4 模型同步：pi 自广告面为准，catalog 按名词表桥接

- composer 模型 chip 对 native(pi) 会话呈现 `get_available_models` 的 `provider/id` 列表（既有行为，零改动）；选择经 `set_model` 往返（既有）。
- 创建对话框的 Settings catalog（裸 id + 操作员 provider 名）与 pi 词表**不硬造映射**：仅当 sebas provider 名与 pi provider id **字面一致**时，目录条目翻译为 `provider/id` 提交；否则会话以 pi 缺省模型启动并在会话内切换（composer chip 里 pi 自广告面总是可用）。
- 补**握手期装载 `model_index`**（driver.rs:506）：把 `get_available_models` 应答灌进裸 id 索引，使裸 id 提交可解析——这是让「目录裸 id」在词表一致时真正可达的小改动。
- 否决「把 sebas 目录强翻译成 pi 模型」：两套命名空间（操作员自命名 vs pi models.json 定义），硬映射必然产生静默错配，违背诚实原则。

### D5 legacy native 会话：保留内核 resume 路径

已存在的 native 会话（`feishu/agent-*` key）与其归档仍按 `is_native` 路由到内核（agent_backend.rs:1573-1575）——内核保留在树并**仅为 legacy resume/归档读路径接线**。新建会话一律走 pi。这样历史会话不丢、内核代码不用删，7 个在办内核任务转挂起（不作废，若日后彻底废弃再清理）。

### D6 命名与重复行

- catalog 内置行 display：`Native Kernel` → `PI`（api.rs:821 一处 + 前端 Settings 徽标「内置 · PI」与尾注文案）。
- `native(PI)` 内置行与 `[acp.agents.pi]` 配置行**并存**（后者是操作员可自定义的一般 pi 行，前者是恒在便利槽位）；内置槽位的 launch 配置**复用 pi 的解析结果**（seed/存储行优先，缺省回退），保证操作员对 [acp.agents.pi] 的自定义（args/sessions_dir）对内置槽位同样生效。默认配置示例的去重（删 [acp.agents.pi] 段）不在本 change。

## Risks / Trade-offs

- **安全回退（最大项）**：native 槽位失去 approval-first 审批与 Landlock 沙箱（pi 无权限系统，工具以子进程权限直跑）。→ 文档与 spec 明示；pi 官方隔离建议是容器化；内核保留，安全敏感部署可回切。
- **pi 上游协议变更随升级流入** → 既有协议 fixture 测试钉住 RPC 形状，升级破协议时测试先红；`add-pi-install-recipe` 装 latest 不钉版本（其 Non-goal），必要时后续加版本前置。
- **legacy 会话依赖保留的内核接线** → 内核成为「仅 resume」的冻结路径；其 open 任务（w184/f9es/san）转挂起。
- **同名 env 冲突只注入默认 provider** → 多账号同协议场景受限，limitation 文档化。
- **catalog 词表桥按字面匹配** → sebas provider 名 ≠ pi provider id 时目录条目对 pi 会话不可用（诚实降级，composer 的 pi 自广告面兜底）。

## Migration Plan

纯行为切换，无数据迁移。上线顺序：本 change 不依赖 `add-pi-install-recipe` 归档，但体验闭环（未装 → 一键装）等它落地。回滚 = 还原 native 路由与注入两处；内核与 legacy 会话路径未动。

## Open Questions

（无阻塞项。D2 的置灰修正是对用户字面选择的显式变更，已在对话中预告；若用户坚持「未装也可选」，改动集中在 availability 需求一条。）
