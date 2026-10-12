# add-pi-install-recipe — Design

## Context

见 proposal.md「Why」。与设计相关的现状事实（均已核实到文件行号）：

- 一键安装机制（add-agent-auto-install，已合并）落在 `sebas-webui/src/agent_install.rs`：`Recipe { name, package, bin, driver, args }`（29-41 行）+ `RECIPES` 表（44-60 行，仅 claude/opencode）+ `recipe_agent_definition`（97-106 行，只写 driver/path，args 非空才写）+ `probe_recipe_bin`（110-124 行，复用 `discover_agent`，driver 取配方值——pi 配方会自动走 `pi_auth_cause`）。路由 handler 在 `routes.rs:1038`，六步顺序：配方校验 → in-flight 锁 → npm 在场探测 → 同步安装 → 复核探测 → 建行判定。未知 recipe 文案硬编码「只支持内置配方 claude / opencode」（1056 行）。
- `pi` 已是合法 driver：`is_valid_driver`（sebas-models/src/agent.rs:133-135）接受 `claude|acp|pi`；`validate_agent_definition`（sebas-dispatch/src/state_store.rs:963）因此放行；`config_of_definition`（src/agent_store.rs:96-110）为 pi 构造 `AcpPiConfig`，path 缺省回退 `DEFAULT_PI_PATH = "pi"`。
- pi 的探测回退缺口：`source_from_store_item`（sebas-webui/src/agent_kinds.rs:271-311）path 缺省**只在 driver=="claude" 时**回退 `"claude"`，pi 落到空串 → command 为空 → 探测误报 `empty command`。
- pi 走 stdio JSONL（非 ACP）：argv 由驱动层拼 `<exe> [args…] --mode rpc [--session-dir …]`（sebas-acp/src/pi/driver.rs:130-150），所以安装配方的 args 应为空。
- 外部事实：npm 包 `@earendil-works/pi-coding-agent`（v1.1.0），`bin = { "pi": "dist/bundle/cli.js" }`，`engines.node >= 22.19.0`，无 postinstall 脚本。
- 凭据语义（已核实）：pi 不必须登录——凭据优先级 `--api-key` > `auth.json` > `models.json` 的 `apiKey` > env（`ANTHROPIC_API_KEY` 等）；sebas 对未登录无硬拒绝，pi 子进程全量继承 sebas env（design D5）。故未登录呈现不在本 change 内改动。

## Goals / Non-Goals

**Goals:**

- pi 与 claude/opencode 在一键安装面上完全同构：同机制、同错误形状、同目录行就绪语义。
- 用**可声明的配方前置**承载 pi 的 Node 版本要求，而不是在 handler 里写 `if recipe == "pi"` 分支——前置是配方数据，claude/opencode 不声明即零影响。
- 修掉 pi 探测的 path 回退缺口（加 pi 配方会实际踩到它）。

**Non-Goals（设计层补充，范围层见 proposal）:**

- 不做 Node 自动安装/升级（只探测与拒绝）。
- 不解析 pi 的凭据、不代跑 `/login`。
- 不重构安装机制本身（六步顺序、锁、超时、建行判定逐字不动）。

## Decisions

> 本 change 在自主会话中立项，用户已当面敲定范围（最小）、Node 前置（安装前探测版本）、回退缺口（一并修）；「未登录呈现不动」按推荐采纳，评审可推翻。

### D1 配方表加行，机制零改动

`RECIPES` 追加 `Recipe { name: "pi", package: "@earendil-works/pi-coding-agent", bin: "pi", driver: "pi", args: &[] }`。`recipe_agent_definition` 现有逻辑（只写 driver/path，args 空则省）对 pi 天然正确——产出 `{driver:"pi", path:<私有 bin 绝对路径>}`，正是 `config_of_definition` 的 pi 分支所期望的形状（args 缺省=空，`--mode rpc` 由驱动层加）。

- 否决「给 pi 加 args `["--mode rpc"]`」：`--mode rpc` 是驱动层拼装细节，写进 agent 定义会与驱动 argv 组装重复、且 `config_of_definition` 会把 args 透传给 PiDriver（可能重复注入）。
- 否决「为 pi 另开安装端点/独立机制」：无任何必要，配方表就是为此设计的扩展点。

### D2 Node 前置做成配方数据（可选字段），不做 pi 硬编码分支

`Recipe` 增可选前置字段（设计上表达为 `Option<Prerequisite>`，`Prerequisite::NodeAtLeast { major, minor }` 这一种形态），pi 声明 `NodeAtLeast{22,19}`，claude/opencode 为 `None`。handler 在「npm 在场探测」之后、「同步安装」之前插入一步：有前置则解析，不满足即 typed 4xx（缺失 → 安装 Node.js 指引；版本低 → 点明当前版本与下限 22.19.0 的升级指引）。

- 否决「handler 里 `if recipe.name == "pi"` 检查 node」：把配方知识漏进流程代码，加第三个带前置的配方就要再改 handler。
- 否决「只查 node 在场不查版本」：pi 的 `engines.node >= 22.19.0` 是硬约束，只查在场会在旧 Node 上装出一个跑不起来的 pi，误导操作员；既然要做前置就做准。
- 版本解析：`node --version` 输出 `v22.19.0`（前导 `v`）——解析为 major/minor 与下限比较，解析失败（输出异常）按「不满足」拒绝（诚实优先，不猜）。

### D3 探测回退缺口按 claude 同构修

`source_from_store_item` 的 path 缺省回退从「仅 claude → `"claude"`」推广为「按 driver 取默认二进制名」：claude → `"claude"`，pi → `"pi"`，acp → 空（ACP 无通用默认，维持现状）。实现上可复用 sebas-models 的 `DEFAULT_CLAUDE_PATH`/`DEFAULT_PI_PATH` 常量，避免再写一份字面量。

- 这是实现层对齐，不新增可观察契约：spec 的「honest reachability」已规定探测口径，此改动只是让 pi 行在没有显式 path 时也走「探测默认二进制」而非误报 `empty command`。
- 加 pi 配方后此缺口会被真实踩到（安装建行会写 path，但操作员手工建/编辑 pi 行清空 path 时会触发），故一并修。

### D4 未登录呈现不动

pi 装完但未登录时：目录行照旧 `reachable=true` + 「可达」，未登录说明留在 tooltip（`pi_auth_cause` 既有语义，add-pi-driver 已定）。理由：未登录 ≠ 不可用（env 有 key 就能跑），把 `not_ready` 内联或当作安装失败都是不诚实的呈现。安装成功通知也不额外加凭据提示——操作员看 tooltip 即可，避免在安装面塞与「装没装上」无关的信息。

### D5 顺带修正过期文案与断言

未知 recipe 文案改为点明三配方（或改为不含枚举的「未知 recipe」措辞，避免每加一行就要改文案——取后者，更耐久）。`RECIPES.len() == 2` 的单测断言改为按名查存在性（不绑长度，加行不破测试）。`validate_agent_definition` 与 `Recipe` 的过期注释（写「claude|acp」）顺手更新为三 driver。

## Risks / Trade-offs

- [Node 版本探测的假阴性] → `node --version` 输出格式异常时按不满足拒绝（宁可拒绝并给指引，不放行装出跑不起来的 pi）；解析逻辑带单测覆盖 `v22.19.0`/`v22.18.9`/异常输出。
- [旧 Node 上 pi 已装但不可用] → 前置只在**安装动作**上拦截；已装好的 pi 行探测不受影响（探测是 `pi --version`，与 node 版本无关）。
- [pi 包名/engines 未来变更] → 配方是硬编码数据，包名与下限变更即改一行；无自动发现面（与既有封闭配方表口径一致）。
- [回退推广误伤 acp] → acp 维持空串回退（无通用默认二进制），单测钉住 acp 行为不变。

## Migration Plan

纯新增，无数据迁移。回滚 = 还原代码；已装的 `agent-tools/pi` 目录与可能的 store 行可独立手工清理。**归档顺序前提**：本 change 的 delta 用 MODIFIED 改「Known agents are installable from Settings」，该需求目前只存在于未归档的 add-agent-auto-install；两者都归档后主 specs 才自洽（`openspec validate` 对 MODIFIED 未落主 specs 的需求放行，不阻塞开发）。
