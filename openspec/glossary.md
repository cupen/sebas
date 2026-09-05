# sebas 名词表(Glossary)

> 术语的单一事实来源。spec 与 planning artifacts 中的术语以本文为准;
> 定义取自现有 specs 与 `docs/architecture.md`,语义变化时先改这里。
> 引用方式:仓库根相对路径 `openspec/glossary.md`。

## 进程角色(单一二进制,子命令决定人格)

- **core(core 进程)**:`sebas core` 的长驻服务本体。会话状态的**单一权威**、
  唯一 spawn ACP 子进程的进程;持有会话映射、core session channel socket,
  按配置承载通道适配器(飞书 WS 等)。(architecture.md §1)
- **run(watchdog 守护)**:唯一拉起其他进程的角色,入口命令 `sebas run`
  (旧名 `watchdog`,现为隐藏别名);按配置监督 core / webui /
  router 子进程(重启/退避/升级)。(architecture.md §2)
- **webui(WebUI)**:dashboard 进程,自身不持有会话状态;经 core session
  channel 观察与驱动会话,或在 core 进程内运行(进程内后端)。
- **router(模型路由)**:provider 透传代理进程,入口命令 `sebas router`
  (旧名 `gateway`,现为隐藏别名),对外提供 OpenAI 兼容 API。
- **dispatch(sebas-dispatch crate,会话分发)**:core 进程内的领域层——会话映射、
  入站事件 dispatch、slash 命令解析、权限处理、出站呈现编排。不是独立进程。
  (原名 sebas-router;rename-cli-surface 改名)

### 三义消解(重要)

「router」一词在仓库里有三种含义,默认指第一种:
1. **CLI `sebas router` / `sebas-router` crate** = 模型路由(Anthropic/OpenAI
   双协议 provider 代理);
2. **`sebas-dispatch` crate**(原名 sebas-router)= core 进程内的会话分发领域层;
3. **前端 `router.ts`** = SPA 的 URL 路由,与以上两者无关。

## 领域概念

- **agent(执行 agent)**:实际执行任务的智能体统称。具体形态见"执行体"。
  消歧:不要与 *agent 会话*(一次会话实例)、*sebas-agent*(原生内核 crate)、
  *ACP agent*(经 ACP 驱动的外部 agent,如 Claude Code)混用。
- **会话(session)**:一次 agent 执行的载体,有唯一会话标识、状态机
  (Spawning/Active/Dormant/…)、会话历史与执行体。由通道消息按需懒创建
  (session-lifecycle)。
- **会话标识(session key)**:会话的地址。历史形状为飞书
  `(chat_id, thread_id)` 二元组 + webui 合成 key;`decouple-feishu-channel`
  之后为中立的 **`ChannelKey`**(见下)。(session-lifecycle;channels)
- **执行体(execution body,又称内核/kernel)**:会话背后的执行内核,两种:
  - **ACP 桥(ACP bridge)**:经 Agent Client Protocol 驱动外部 agent
    (Claude Code 等),是默认执行体。
  - **原生内核(native kernel,sebas-agent crate)**:自研 agent 内核
    (turn loop、工具集、policy engine、权限审批)。(feishu-bridge;agent-core)
- **项目(project)**:host 上的一个目录路径,通常是 git 仓库根;工作台的
  组织单元。每个 agent 会话至多归属一个项目分组。(agent-workbench)
- **工作台(workbench)**:webui 中的项目导向 agent 工作区(`/agent` 页):
  项目列表、会话侧栏(按项目目录或聊天来源分组)、时间线与输入区、
  inbox(操作者离开期间到达的 turn 流)。(agent-workbench;webui/projects)
- **卡片(card)**:对用户的流式富文本呈现,含思考/工具面板、交互元素
  (按钮/表单)、预算与轮转。本 change 后 = **中立呈现模型**由通道适配器
  渲染成各自渠道的形态(飞书 = card schema 2.0 JSON)。(feishu-cards;channels)
- **主控(webui 主控形态)**:部署形态——watchdog 默认只启动 webui,
  core/飞书按需启用。(feishu-option)

## 通道抽象(decouple-feishu-channel 引入)

- **通道(channel)**:会话的来源与去向。现状:`feishu`、`web`;未来任意
  IM / agent 客户端。核心只依赖抽象,不特判任何渠道。(channels)
- **适配器(channel adapter)**:一个通道对中立抽象的实现——把渠道入站
  事件翻译为中立事件、把中立呈现渲染为渠道出站。经**适配器注册表**接入,
  由配置决定是否注册。(channels)
- **`ChannelKey`(中立会话标识)**:`通道名 + 通道内不透明引用`。核心不解析
  引用内部结构;`web-*`/`oc_*` 等前缀特判废弃。(channels)
- **`ChannelEvent`(中立入站事件)**:text / media / button callback /
  form callback 四种,携带来源 `ChannelKey`。(channels)
- **`ChannelCard`(中立呈现模型)**:出站呈现的渠道无关累积模型——标题/
  正文/思考/工具/用法/交互元素与冻结·更新·轮转生命周期。(channels)
- **core session channel(core.sock)**:core 与进程外客户端(独立 webui)
  之间的 Unix socket 协议(观察/驱动会话)。core 是唯一写者,客户端只是
  缓存。(core-session-channel)

## 易混对照

| 易混 | 区分 |
|---|---|
| core vs dispatch | core 是进程角色;dispatch(原 sebas-router)是该进程内的会话分发领域层 crate |
| router(模型路由)vs dispatch(会话分发) | 前者是独立的 provider 代理进程(`sebas router`);后者是 core 进程内的领域层 crate(原 sebas-router) |
| webui(进程)vs `web`(通道) | 前者是 dashboard 进程;后者是它在通道抽象里的注册名 |
| sebas-agent vs ACP 桥 | 两种执行体:自研内核 vs 经 ACP 驱动的外部 agent |
| 项目 vs 工作台 | 项目是目录(组织单元);工作台是 webui 里呈现它的页面 |
| 产品定位"工作台" vs 页面级"工作台" | 前者指 sebas 整体(README 定位用法:"自托管的 agent 工作台");后者专指 webui 的 `/agent` 页。上下文无法区分时优先按页面级理解 |
| 会话 vs turn | 会话是持久载体;turn 是其中一次问答执行 |
