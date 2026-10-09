# add-agent-auto-install — Design

## Context

见 proposal.md「Why」。与设计相关的现状事实（均已核实到文件行号）：

- Agents 分区（`sebas-webui/frontend/src/views/settings-modal.ts`）已有完整 CRUD 与可达性徽标；`GET /api/agents` 由 `sebas-webui/src/agent_kinds.rs` 的 `discover_agent` 服务端探测（which 语义 + `--version`，`resolved_binary` 处理 Windows PATHEXT），fix-webui-qa-round10 后绝对路径直查。**全仓无任何「安装 agent」代码。**
- agent 目录权威是 agents store；`POST /api/agents` 经 `state_mutate("agents", {"op":"put","id",…,"agent":{…}})` 提交（`sebas-webui/src/routes.rs:914`），内嵌与 core-channel 两种后端模式载荷同形。`validate_agent_definition`（`sebas-dispatch/src/state_store.rs:950`）允许 sessions_dir/work_dir 留空（None 走默认）、driver 封闭 `claude|acp`、acp 要求 path 非空。
- 无通用长任务机制；`POST /api/skills/sync` 是同步阻塞返回全量 outcome 的先例。前端 `doFetch` 无超时（`api/client.ts:939`），axum 0.8 无全局超时中间件——同步长请求在 127.0.0.1 直连（无反代）下安全。
- sebas home 可经 `sebas_domain::state_paths::sebas_home()` 取得，webui 已依赖 sebas-domain。
- 外部事实：claude 的 npm 包 `@anthropic-ai/claude-code`（bin `claude`）；opencode 的 npm 包 `opencode-ai`（bin `opencode`）；两者官方支持纯 npm 安装。npm `install -g --prefix <dir>` 的 bin shim 落点：unix `<dir>/bin/`，Windows `<dir>/` 根（.cmd/.ps1 shim）。

## Goals / Non-Goals

**Goals:**

- 从 Agents 分区一键完成「探测缺失 → npm 安装 → 目录行就绪 → 会话可开」全链路，零特权、零手工步骤。
- 安装产物完全落在 sebas 自治范围内（`<SEBAS_HOME>/agent-tools/<recipe>/`），卸载 = 删目录。
- 复用既有机制到最大程度：探测复用 `discover_agent` 口径、建行复用 `state_mutate("agents")` 路径、权限复用 SettingsManage、错误复用既有错误信封。

**Non-Goals（设计层补充，范围层见 proposal）:**

- 不做安装进度流式呈现（按钮 busy + 完成通知即可）。
- 不引入 registry/镜像配置面——用操作员 npm 自身的 registry 配置。
- 不做 Windows 硬验收（尽力支持，见 D2 平台差异）。

## Decisions

> 本 change 在自主会话中立项，用户未逐项当面确认；以下决策按推荐方案敲定，评审时可推翻（影响面集中在 D2/D3/D4）。

### D1 执行者：WebUI 后端进程 spawn npm

安装请求由 webui 进程直接 spawn `npm` 完成（新模块 `sebas-webui/src/agent_install.rs`）。

- 否决「前端展示命令让用户复制」：不是自动安装，违背需求本意。
- 否决「CLI 子命令为主、UI 调它」：agent 目录管理的面在 webui（CLI 只有 `agent-kinds list` 只读），装一个 CLI 动词超出本期范围（已列 Non-goal）。webui 进程 spawn 子进程有 `--version` 探测先例，同构扩展。

### D2 落点：npm `--prefix` 全局模式装进 sebas 私有前缀

安装命令形如 `npm install --global --prefix <SEBAS_HOME>/agent-tools/<recipe> <package>`，bin 落 `<prefix>/bin/`（unix）/`<prefix>/`（Windows）。agent 行的 path 登记私有 bin 的**绝对路径**，与「绝对路径直查」探测口径直接衔接。

- 否决 `npm install -g` 裸全局：系统 node prefix 通常需要 root/sudo，webui 进程无特权；且污染操作员全局环境，违背沙箱纪律。
- 否决下载官方 native installer：非 npm 渠道（Non-goal），且引入非 npm 的下载/校验逻辑。
- 平台差异收进配方表的 bin 路径推导（unix `<prefix>/bin/<bin>`、windows `<prefix>/<bin>.cmd`），Windows 由 `resolved_binary`/PATHEXT 既有逻辑兜底，不做硬验收。

### D3 请求模型：同步 POST + per-recipe in-flight 锁

`POST /api/agents/install` 同步执行：handler 内 `tokio::time::timeout`（300s）包 npm 子进程，per-recipe 的 `Mutex`/`HashSet` in-flight 锁防重入（并发同 recipe → 409），前端按钮 busy 态。响应 `{installed, path, version, agent_created}`。

- 否决「后台 job + 轮询」：引入仓库尚不存在的通用长任务机制，范围膨胀一个量级；前端 fetch 与 axum 均无超时、127.0.0.1 直连无反代，npm 安装两个 CLI 包通常 <2min，同步在 300s 上限内安全（超时即如实失败，重试幂等，见 D6）。

### D4 建行策略：按 spawn 解析口径判缺失，缺失才建

安装成功后以「该 id 在 agents store ∪ config 注册表是否已有定义」为判据（与 spawn 解析、目录 union 同构）：无定义 → 经 `state_mutate("agents", {"op":"put",…})` 建标准行（claude → driver `claude`；opencode → driver `acp` + args `["acp"]`，与前端表单 opencode 形态同口径；path 指私有 bin 绝对路径；sessions_dir/work_dir 留空走默认）；有定义 → 不动，响应 `agent_created=false`。

- 否决「总是 upsert 覆盖 path」：会覆盖操作员手工维护的行（自定义 path/参数），安装动作不该有破坏性。
- 否决「装完不建行」：装完还要手建表单，违背一键目标。

### D5 配方清单封闭、硬编码

静态表 `[{recipe: "claude", package: "@anthropic-ai/claude-code", bin: "claude"}, {recipe: "opencode", package: "opencode-ai", bin: "opencode"}]`，含各自的 agent 定义模板。请求只认 recipe 名，**不接受任意 npm 包名输入**。

- 否决「开放包名/命令输入」：等于把「任意远程代码安装执行面」挂到 webui 端点上；配方封闭把暴露面限定为两个官方包。

### D6 重装语义：幂等升级 latest

重复安装同一配方 = 再次 `npm install`（npm 天然升级到 latest），无「已装拒绝」状态机。

- 否决「已装返回 409」：修复损坏安装、升级旧版是真实场景，拒绝反而逼用户手动删目录。

### D7 权限、守卫与错误形状

- 权限：`Permission::SettingsManage`（与 agents CRUD 同款，root/admin）。
- Origin 守卫：与 provider/agents mutation 同款 loopback 校验。
- npm 缺失：typed 4xx（探测 `npm --version` 不可达即拒），错误信息含「安装 Node.js 后重试」指引，对齐 `sebas skills add` 探测外部工具的口径；安装失败：携带 npm stderr 尾部摘要。
- 失败原子性：npm 非零退出 → 不建行、不报成功（npm 自身可能残留部分文件于 prefix，不承诺清理，重装自愈）。

### 测试策略：fake npm，绝不真拨 registry

单测/集成/浏览器验收统一用 PATH 垫 fake `npm` 脚本（记录 argv + 按剧本落假 bin 文件），进程级 e2e 在沙箱内走完整旅程（安装 → 目录行 → 可达 → 建会话入口可见）。真 npm 安装不在自动化验收面内。

## Risks / Trade-offs

- [慢网下 npm 超过 300s 超时] → 如实失败 + UI 提示重试；重试幂等（D6）。
- [同步长请求占住浏览器 fetch] → 单按钮 busy 态隔离影响面；仅 root/admin 可触发。
- [Windows npm shim 形态（.cmd/无扩展名文件）] → `resolved_binary` PATHEXT 解析已有先例；配方表 bin 推导带平台分支；Windows 不做硬验收。
- [用户手删 agent-tools 目录] → 探测自然回落「command not found」，行为诚实，重装即修复。
- [私有 bin 不在 PATH，终端里直接敲 `claude` 不可用] → 目录行与安装响应都携带完整绝对路径；「让终端也能用」不是本期目标。
- [npm 供应链风险] → 只装两个官方包名（D5），用操作员 npm 与 registry 配置，不加 `--force` 类危险旗标。

## Migration Plan

纯新增能力，无数据迁移。回滚 = 还原代码；`agent-tools/` 目录与可能已建的 store 行均为普通文件/行，可独立手工清理，不阻塞回滚。
