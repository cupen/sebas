# add-pi-install-recipe

## Why

add-pi-driver 把 pi 接成了与 claude / acp 并列的一等 driver，并把产品默认 agent 切到 pi——但 Settings → Agents 的一键安装配方表里没有 pi。结果是「默认 agent 装不了」：操作员拿到一个默认选 pi 的产品，却要离开 UI 手敲 `npm i -g @earendil-works/pi-coding-agent`。add-pi-driver 已明文把「pi 安装配方」挂靠到 add-agent-auto-install 落地后加行，该机制现已合并，补这一行是收尾。

## What Changes

- 安装配方表新增 `pi` 配方：npm 包 `@earendil-works/pi-coding-agent`、bin `pi`、driver `pi`（args 空，pi 的 `--mode rpc` 由驱动层拼装，不在安装配方里）。
- 配方结构支持**可选的 Node 版本前置**（非 pi 硬编码分支）：pi 声明 `node >= 22.19.0`，安装前探测 node 在场与版本，缺失或版本不足时以 typed 拒绝应答并给升级指引；claude / opencode 无前置、行为不变。
- 修 `source_from_store_item` 的既有缺口：`driver=pi` 且 store 行无 path 时回退内置 `"pi"`（与 claude 的 `"claude"` 回退同构），否则 pi 行探测会误报 `empty command`。
- 顺带修正过期文案与断言：未知 recipe 文案（现写「只支持 claude / opencode」）、`RECIPES` 长度单测、driver 封闭集相关过期注释。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-settings`: 安装配方清单扩为三配方（新增 pi），并引入「配方可声明 Node 版本前置」的安装前置契约；pi 配方的目录行就绪语义与 claude/opencode 同构（driver `pi`、path 指私有前缀 bin）。

## Impact

- `sebas-webui`：`agent_install.rs`（配方表加 pi 行、`Recipe` 增可选 Node 前置字段与探测、`recipe_agent_definition` 对 pi 落 driver `pi`）、`routes.rs`（未知 recipe 文案）、`agent_kinds.rs`（pi 的 path 回退）。
- 前端 `settings-modal.ts`：`INSTALL_RECIPES` 加 pi 行（状态徽标与安装按钮沿用既有机制，无新交互）。
- 测试：配方表单测、Node 前置单测（fake node 版本剧本）、pi 回退单测、安装端点集成与进程级旅程扩到 pi。
- 无 breaking 变更：claude / opencode 两配方与其安装路径逐字不变。

## Non-goals

- 不做 pi 的凭据/登录自动化（design D5：sebas 不代写 pi 的凭据，`/login` 与 env key 由操作员自理）。
- 不检查/安装 Node 自身——只在 pi 配方上做版本前置探测与拒绝，不做 Node 升级动作。
- 不改未登录状态的呈现（目录行「可达」+ tooltip 的既有 pi 语义不动）。
- 不做 pi 权限审批桥、sebas-node 远端 pi（add-pi-driver 已列 Non-goal，仍不做）。
- 不动 claude / acp 两个既有配方的安装行为与探测口径。
