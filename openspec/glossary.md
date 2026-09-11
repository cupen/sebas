# sebas 名词表(Glossary)

> 术语的单一事实来源。spec 与 planning artifacts 中的术语以本文为准;
> 定义取自现有 specs 与 `docs/architecture.md`,语义变化时先改这里。
> 引用方式:仓库根相对路径 `openspec/glossary.md`。

## 进程角色(主控是单一二进制,执行节点是第二个可分发产物)

> **范围**:主控二进制 `sebas` 是**单一二进制、子命令决定人格**;执行节点
> `sebas-node` 是**第二个可分发产物**——**独立二进制,不含任何主控角色**
> (add-remote-execution-node D0)。下表前五项是主控二进制内的子命令/进程内
> 领域层,末项是独立二进制;「单一二进制」只描述主控,不描述整个系统。

- **core(core 进程)**:`sebas core` 的长驻服务本体。会话状态的**单一权威**、
  唯一 spawn ACP 子进程的进程;持有会话映射、core session channel socket。
  core 是纯会话核心——IM 适配器宿主是独立的 `sebas im` 服务,core 不注册任何
  IM 适配器(extract-im-service)。(architecture.md §1)
- **run(watchdog 守护)**:唯一拉起其他进程的角色,入口命令 `sebas run`;
  按配置监督 core / webui /
  router / im 子进程(重启/退避/升级)。(architecture.md §2)
- **webui(WebUI)**:dashboard 进程,自身不持有会话状态;经 core session
  channel 观察与驱动会话,或在 core 进程内运行(进程内后端)。
- **router(模型路由)**:provider 透传代理进程,入口命令 `sebas router`,
  对外提供 OpenAI/Anthropic 兼容 API。
- **dispatch(sebas-dispatch crate,会话分发)**:core 进程内的领域层——会话映射、
  入站事件 dispatch、slash 命令解析、权限处理、出站 Out 指令编排(会话执行向;
  IM 呈现/reaction 由 sebas-im 前端负责)。不是独立进程。
  (原名 sebas-router;rename-cli-surface 改名)
- **sebas-node(执行节点,execution node)**:**独立二进制**(第二个可分发产物,
  add-remote-execution-node D0),在主控以外的机器上运行 agent 会话。**不含主控
  角色**——core / webui / router / im 的实现与可运行入口都不在节点产物里(不是
  「带过去但不运行」,是根本不带),节点机上也不需要安装主控。节点**出站**拨号主控
  (反向连接:主控不必能反访节点,节点无需任何入站端口);持自己会话的**执行事实**
  (子进程寿命、有序 turn 日志、审批悬空状态、节点本地 provider 凭据),会话身份与
  期望态来自主控;权限审批**永不自裁**(无本地放行入口),主控不可达时无限期 park。
  节点配置只有一个 `[node]` 段,不复用主控配置 schema。(execution-node)

### 三义消解(重要)

「router」一词在仓库里有三种含义,默认指第一种:
1. **CLI `sebas router` / `sebas-router` crate** = 模型路由(Anthropic/OpenAI
   双协议 provider 代理);
2. **`sebas-dispatch` crate**(原名 sebas-router)= core 进程内的会话分发领域层;
3. **前端 `router.ts`** = SPA 的 URL 路由,与以上两者无关。

「escalate」二义:
1. **审批 escalate** = native 内核 gated-call 的"带理由的一次性放行"决策
   (`ApprovalAnswer::Escalate { reason }`);ACP 路径无等价物,降级为
   `allow_once`(见 unify-permission-approval-vocabulary);
2. **挂起检测的 kill ladder**(acp-driver)= `interrupt()×3 → disconnect
   (≈SIGTERM) → drop (≈SIGKILL)` 的阶梯,与审批决策无关。

「ACP」二义:
1. **历史 `claude-acp-bridge`**:Claude 私有转码桥,ADR-1(08-06)弃用并删除,
   **不再存在**;
2. **现行 `agent-client-protocol` 标准**(crate v2,`sebas-acp` 依赖):驱动原生
   ACP 第三方 agent(gemini/copilot/opencode 等)。
   ADR-1 弃的是 (1) 而非 ACP 标准本身——两件事共用同一缩写,读史时才像反转。

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
- **pending submission(待生效提交)**:core 已接受、但尚未开始执行的文本
  提交。两种处置(disposition):
  - **staging(并入首条消息)**:会话尚不存在(spawn 窗口)期间接受的提交,
    激活时与同批兄弟提交合并为**一条**首条 prompt;
  - **queued turn(按序执行的待执行回合)**:某个 turn 在飞期间接受的提交,
    将作为独立回合按投递序逐条执行。
  每个 pending submission 携带 core 分配的 per-session 单调稳定 id、文本、
  位置与优先标记(/btw);开轮(或激活合并)即离开待执行栈、落 transcript,
  此后对它的移除/重排请求被类型化拒绝(AlreadyStarted)。
  (workbench-turn-queue)
- **项目(project)**:host 上的一个目录路径,通常是 git 仓库根;工作台的
  组织单元。每个 agent 会话至多归属一个项目分组。(agent-workbench)
- **回合(turn,显示单位)**:对话视图中的一个**显示单位**(workbench-
  conversation-view):一次操作者提交是一个回合(「你」气泡),其后到下一条
  提交之前的全部 agent 产出(流式正文、thinking、工具调用)合并为**一个**
  agent 回合(单个气泡;thinking 折叠、工具收进「used N tools」可展开组)。
  回合是纯前端分组概念——core 的 transcript 仍是 chunk 级条目
  (`kind`=prompt|content,`element_type`=markdown|thinking|tool|error),
  客户端按 `kind == "prompt"` 切回合;未读 seam 也按回合计数,永不落在
  回合内部。(agent-workbench;webui;core-session-channel)
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
| sebas(主控)vs sebas-node(执行节点) | 前者是主控二进制(`core`/`webui`/`router`/`im` 等子命令);后者是**第二个可分发产物**、独立二进制,不含主控角色,只作为执行位出站连回主控 |
| 项目 vs 工作台 | 项目是目录(组织单元);工作台是 webui 里呈现它的页面 |
| 产品定位"工作台" vs 页面级"工作台" | 前者指 sebas 整体(README 定位用法:"自托管的 agent 工作台");后者专指 webui 的 `/agent` 页。上下文无法区分时优先按页面级理解 |
| 会话 vs turn | 会话是持久载体;turn 是其中一次问答执行 |
| turn(显示回合)vs transcript 条目 | 前者是 webui 对话视图的**显示单位**(一个提交或一整个 agent 回合气泡);后者是 core transcript 的 chunk 级**存储条目**(每条带 kind/element_type/position)。一个显示回合通常对应多条 transcript 条目 |
| pending submission vs `SessionStatus::Queued` | 前者是 core 已接受、尚未开始执行的**提交**(staging/queued turn 两种处置,见上);后者是 webui 的**会话行状态词**(`models.rs` 的 `SessionStatus::Queued`)——active 会话子进程已存在但尚未产出任何内容。两者毫无关系,spec 行文说「排队中的提交」时永远指前者 |

- **im 服务**：独立 IM 服务进程（`sebas im`，`sebas-im` crate）——IM 适配器
  宿主与交互面（卡片/命令/表单/reactions/媒体），经核心会话通道观察并驱动
  会话；core 是纯会话核心，不注册任何 IM 适配器（extract-im-service）。
