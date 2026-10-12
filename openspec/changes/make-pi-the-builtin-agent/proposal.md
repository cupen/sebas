# make-pi-the-builtin-agent

## Why

pi 已是产品默认 agent（add-pi-driver），但内置 `native` 槽位仍指向自研内核（`sebas-agent`），按「是否配置内核凭据」判可达——默认部署下**恒置灰不可选**。操作员要的体验是：启动 sebas 进后台，直接用内置的 pi 会话，自己在 Settings → Models 配的 provider/model 对它生效。当前断在三处：内置槽位指向内核、pi 拿不到 settings.db 的 provider 凭据（spec 锁死 pi-managed）、pi 一键安装未落地。

## What Changes

- 内置 `native` 槽位（wire id 保留，display 改 `PI`）改由 **pi driver** 承担：会话创建/回合/取消走 `pi --mode rpc` 子进程，launch 配置复用 `[acp.agents.pi]` 种子或 pi 缺省；可达性 = pi 二进制在场（`pi auth check` 作 advisory cause）。
- **规范反转（凭据）**：`pi-agent` 的「sebas SHALL NOT 翻译凭据」改为——sebas 在 spawn 时把 provider 注册表解析成**标准 provider env**（`ANTHROPIC_API_KEY`/`ANTHROPIC_BASE_URL` 等，与 claude 通路同一解析机制）注入 pi 子进程；`models.json` 与 auth 存储仍 pi 自管，sebas 不写。
- **规范修正（可达性）**：内置槽位不再按内核凭据判可达；pi 未安装时置灰并指引「Settings → Agent 一键安装 pi」，而不是允许选中后失败。
- 模型同步：native(pi) 会话的模型面来自 pi 自广告（`get_available_models`），选择经 `set_model` 往返生效（既有管线，补握手期 model_index 装载使裸 id 可解析）。
- sebas-agent 内核**保留在树、不接线**（legacy native 会话的 resume 路径除外）——可回滚，不删代码。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `pi-agent`: 凭据条款反转——sebas 注入标准 provider env（spawn 时解析），`models.json`/auth 存储仍 pi 自管；env 为 spawn 期语义，启动后凭据变更不影响在跑会话。
- `agent-workbench`: 内置槽位执行体从自研内核改为 pi driver；执行体可用性对内置槽位按 pi 二进制在场判定，修复指引指向安装面；模型选择需求对齐 pi 的 `provider/id` 往返语义。

## Impact

- `src/agent_backend.rs`（native 槽位路由改指 acp 侧 pi；execution_bodies native 条目改 pi 可达性）、`src/session_boot.rs`/`src/spawn_env.rs`（pi 的 provider env 注入）、`sebas-acp/src/pi/driver.rs`（握手期装载 model_index）。
- 前端：新会话对话框与 composer 的 native 文案、Settings → Agents 内置行徽标（`内置 · PI`）。
- 测试：`agent_kinds_test.rs`、`tests/testsuite-webui/tests/first-paint.spec.ts`、`gateway_bff_test.rs` native 400 族、acceptance 的 native 旅程改写为 pi 旅程；legacy native 会话 resume 保留内核路径的回归。
- 依赖前置：`add-pi-install-recipe`（未实现）提供「未装 pi → 一键装」的指引落点。

## Non-goals

- 不删除 `sebas-agent` crate 与 `agent-core` 规范（保留可回滚；本 change 只把内置槽位从内核改指 pi）。
- 不给 pi 增加权限系统或沙箱（pi 会话无审批卡是 pi-agent spec 既有契约，保持如实呈现）。
- 不做 provider 凭据的**运行中热更新**（env 是 spawn 期语义；切换模型走 `set_model`，切换 provider 需新会话）。
- 不代写 pi 的 `models.json`/`auth.json`（操作员自定义 provider 仍需在 pi 侧配置）。
- 不动 `add-pi-install-recipe`（独立 change，本 change 的前置）。
- 不重写主 specs 里全部 105 处 native 提及——只改行为契约所在的 `pi-agent` 与 `agent-workbench`，其余（testsuite/feishu 等）随任务同步。
