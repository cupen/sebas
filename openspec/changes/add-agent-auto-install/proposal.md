# add-agent-auto-install

## Why

Settings → Agents 分区已经能增删改 agent 并诚实探测可达性，但当 agent 二进制本身缺失时（新机器、新环境），操作员只能看到「command not found」徽标，然后离开 UI 去查文档、手敲 npm 命令。已知 claude（`@anthropic-ai/claude-code`）与 opencode（`opencode-ai`）都支持纯 npm 安装，缺的只是一个从目录直达的安装动作。

## What Changes

- Settings → Agents 分区新增「可安装 agent」区：内置配方（claude、opencode）逐行显示安装状态，未装时提供一键「安装」按钮。
- webui 后端新增 `POST /api/agents/install`：以操作员自己的 npm 把配方包安装进 sebas 私有前缀 `<SEBAS_HOME>/agent-tools/<recipe>`（npm `--prefix` 全局模式），零 sudo、不污染操作员的全局环境；同请求内重探测并刷新可达性。
- 安装成功且该 id 在 store 与 config 均无对应行时，自动创建一条标准 agents store 行（claude 用 claude 驱动、opencode 用 acp 驱动 + `acp` 参数，path 指向私有 bin 的绝对路径）；已有行一律不动。
- npm 不在场（或安装失败）时诚实报错并给最小指引，对齐 `sebas skills add` 探测外部工具的既有口径；重复安装 = 升级到 latest，无独立升级面。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-settings`: 新增两条需求——已知 agent 可从 Settings 一键 npm 安装（封闭配方表、私有前缀落点、npm 缺失诚实报错）；安装成功自动补目录行（仅缺失时创建，已有行不动，目录即时可用）。
- `webui`: HTTP route surface 的 agent management cluster 增加 `POST /api/agents/install`（body `{recipe}`，挂既有 SettingsManage 权限与 mutation origin 守卫）。

## Impact

- `sebas-webui`（后端）：新增安装模块（配方表、npm 探测、spawn + 超时 + in-flight 锁）、路由与错误形状；复用 `discover_agent` 探测与 `state_mutate("agents", …)` 建行路径。
- `sebas-webui/frontend`：Agents 分区新增可安装区 UI、api client 新增 `agentInstall`。
- 落盘新增 `<SEBAS_HOME>/agent-tools/<recipe>/` 目录树（npm prefix），删除该目录即完全卸载。
- 无 breaking 变更：不动既有 agents CRUD、config 种子语义、spawn 解析与探测协议。

## Non-goals

- 不做 agent 市集、发现或版本选择（沿用 add-agent-settings 的历史排除）。
- 不安装 node/npm 自身；npm 不在场只报错指引。
- 不做卸载/升级专用 UI（重复安装即升级 latest；卸载 = 删 `agent-tools/<recipe>` 目录）。
- 不加 `sebas agent install` CLI 子命令。
- 不接受任意 npm 包名输入——配方清单硬编码封闭，防任意远程代码安装面。
- 不接 npm 以外的安装渠道（brew、官方 native installer 等）。
