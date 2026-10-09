## ADDED Requirements

### Requirement: Known agents are installable from Settings

Settings 的 Agents 分区 SHALL 呈现一个封闭的内置安装配方清单（首期 `claude` 与 `opencode`，npm 包分别为 `@anthropic-ai/claude-code` 与 `opencode-ai`），每条配方 SHALL 复用目录的可达性探测口径呈现状态（探测对象含配方 bin 名与 sebas 私有安装前缀），未探测到对应可执行时 SHALL 提供安装动作。安装 SHALL 由 WebUI 后端以操作员环境的 npm 执行并落进 sebas home 下的私有前缀目录（`agent-tools/<recipe>`），SHALL NOT 要求特权、SHALL NOT 改写操作员的全局 npm 环境。npm 不在 PATH 时安装请求 SHALL 以携带最小指引的失败应答如实拒绝；安装进程失败时 SHALL 呈现失败原因摘要且 SHALL NOT 创建目录行。同一配方的重复安装 SHALL 等价于升级到最新版（幂等重装，无独立升级面）。安装请求携带清单之外的 recipe 时 SHALL 以 typed 拒绝应答，且 SHALL NOT 发起任何子进程。

#### Scenario: 一键安装到私有前缀

- **WHEN** `claude` 的 bin 不可达且操作员的 npm 在 PATH，操作员在 Agents 分区对 claude 配方点「安装」
- **THEN** 包被安装进 sebas home 下的 `agent-tools/claude` 前缀，应答携带最终 bin 的绝对路径与探测到的版本，目录刷新后 claude 行可达

#### Scenario: npm 缺失诚实报错

- **WHEN** npm 不在 PATH 时发起安装请求
- **THEN** 请求失败并说明 npm 未安装与最小指引（安装 Node.js/npm 后重试），agents 目录无任何变化

#### Scenario: 安装失败不落半成品

- **WHEN** npm 进程以非零退出（如网络不可达、registry 拒绝）
- **THEN** 应答携带 npm stderr 摘要，不创建 agents store 行、不报告安装成功

#### Scenario: 未知配方是 typed 拒绝

- **WHEN** 安装请求携带清单之外的 recipe 名
- **THEN** 以 typed 拒绝应答，不 spawn 任何子进程

#### Scenario: 重复安装即升级

- **WHEN** 对此前已安装成功的配方再次发起安装
- **THEN** 重装为最新版并成功，无冲突报错，目录刷新后版本为新探测值

### Requirement: Installing an agent seeds its catalog entry

安装成功后，若按 spawn 解析口径（agents store ∪ config 注册表）该配方 id 尚无定义，系统 SHALL 自动创建一条标准 agents store 行并经 agents 目录的既有单写者提交路径落库：`claude` 配方用 claude 驱动，`opencode` 配方用 acp 驱动且参数首词为 `acp`，path SHALL 指向私有前缀 bin 的绝对路径。该 id 已有定义（store 行或 config 条目）时 SHALL 保持原定义不动，且应答 SHALL 如实区分「已创建」与「已存在未改动」。

#### Scenario: 全新安装后目录行就绪

- **WHEN** opencode 安装成功且 store 与 config 均无 opencode 定义
- **THEN** agents 目录出现新的 opencode 行（acp 驱动、path 指向私有前缀 bin），可达，新建会话对话框无需重启任何进程即可选用

#### Scenario: 已有定义不被覆盖

- **WHEN** claude 安装成功但操作员已在 store 手工维护 claude 行（自定义 path）
- **THEN** 原 claude 行保持不变，应答标记该定义已存在未改动

#### Scenario: config 种子定义同受尊重

- **WHEN** config.toml 存在 `[acp.agents.opencode]` 条目而 store 无该行时安装成功
- **THEN** 不创建 store 行（该 id 已有定义），应答标记已存在未改动
