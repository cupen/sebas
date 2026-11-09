# 浏览器旅程迁移矩阵

- 基线：`40d8ce3`（2026-11-08）。共 **71 个 spec / 197 条 `test()`**；执行入口 `invoke testsuite-webui` 串联 9 个 Playwright config（原 6 个 + React 入口的 `react` / `react-auth` / `react-setup` 三套装配）。
- 权威锚点：`openspec/specs/testsuite-webui-browser/spec.md`（21 requirement / 55 scenario）与 `openspec/specs/agent-workbench/spec.md`（56 requirement）；`tests/acceptance/COVERAGE.md` 与 `tests/testsuite-webui/README.md` 是账本视图，不是语义来源。
- 本矩阵的目的：让每一条既有旅程都有明确归宿，禁止以「框架替换」为由静默删除。

## 1. 配置矩阵

| config | 端口 | 形态 | spec |
|---|---|---|---|
| `playwright.config.ts` | 9899 | 单进程（auth off） | 其余全部 |
| `playwright.auth.config.ts` | 9898 + Vite 5286 | auth on，预置 admin；React 入口 | `auth`、`users-admin`、`agents-gate`、`qa-round7-ws-auth`、`provider-gate`、`qa-round14-rbac`（+ React `react-auth`） |
| `playwright.auth-setup.config.ts` | 9896 + Vite 5287 | auth on，零用户；React 入口 | `auth-setup` |
| `playwright.detached.config.ts` | 9897 + Vite 5291 | core + 独立 webui 双进程；React 入口 | `deployment`、`approval-detached`、`tiered-notices`、`core-link-badge` |
| `playwright.deployment.config.ts` | 9897 + Vite 5292 | detached（单 spec 装配）；React 入口 | `deployment` |
| `playwright.dead-core.config.ts` | 9895 + Vite 5293 | 单进程 core 被 SIGKILL；React 入口 | `singleprocess-dead-core` |
| `playwright.native.config.ts` | 9894 | `TESTSUITE_NATIVE=1` | `test-model-scenarios`、`qa-round8-native`、`tool-call-fold` |
| `playwright.react.config.ts` | 9893 + Vite 5290 | 生产入口 `/`（端口独立），auth off | `react-workbench`、`react-responsive-a11y` |
| `playwright.react-auth.config.ts` | 9892 + Vite 5289 | 生产入口 `/`，auth on（admin/admin） | `react-auth` |
| `playwright.react-setup.config.ts` | 9891 + Vite 5288 | 生产入口 `/`，auth on、零用户 | `react-setup` |

## 2. 处置图例

| 标记 | 含义 | 迁移要求 |
|---|---|---|
| **A 等价迁移** | 用户可观察行为不变，只改选择器/DOM 实现 | 断言逐条换成 `getByRole`/可访问名/稳定 `data-testid`；不改语义 |
| **B 评审调整** | 交互随蓝图有意改变（导航、设置工作区、待办区、转录布局） | 必须在 1.6 评审通过后按蓝图改写断言；改动逐条写入本表，禁止顺手弱化 |
| **C 待蓝图决定** | 依赖蓝图对布局/可访问性的最终选择 | 1.6 前不得删除或改写；评审后回填 A 或 B |
| **D 删除/合并** | 产品语义消失或重复 | 本 change 当前 **0 条**；若将来出现必须写明依据与替代证据 |

## 3. 全量 spec 映射（71/71）

「旅程数」= 该 spec 的 `test()` 数（含 retry 前的用例定义）。「账本」列标出 `COVERAGE.md` 是否已有该 spec 的行。

| # | spec | 旅程数 | config | 主要 requirement / 场景 | 账本 | 处置 | 迁移要点 |
|---|---|---|---|---|---|---|---|
| 1 | `add-scope-reason` | 1 | main | workspace-root「手填越界路径给出禁用原因」 | ✅ | B | 项目目录选择器改为 AntD 弹窗/工作区；越界原因与提交禁用必须保留 |
| 2 | `agents-gate` | 8 | auth | webui-user-management「RBAC 角色与权限执法」 | ❌ | B | 设置工作区 Agents 分区；viewer/member 写入口裁剪与服务端 403 同时保留 |
| 3 | `agents-slots` | 3 | main | agent-settings「claude 档位映射」 | ❌ | B | 设置工作区表单；档位映射与 new-session 联动不变 |
| 4 | `agents` | 3 | main | agent-settings「Agents 目录 webui 全链路」 | ❌ | B | 表单增删改、免重启进入创建对话框；失败内联 |
| 5 | `approval-detached` | 3 | detached | webui「approval_answer end-to-end」 | ✅ | B | 审批进入会话内待办区；双进程链路与 404 类型化拒绝不变 |
| 6 | `approval-reconcile` | 2 | main | fix-webui-qa-round3 7.1/7.2（task 级） | ✅ | B | 读模型拉取去重/退避保留；呈现位置改待办区 |
| 7 | `approval-restore` | 1 | main | permission-flow「刷新或重连后仍可取得」 | ✅ | B | 同一 `request_id` 幂等重建；不得因待办区重组而丢卡 |
| 8 | `archive-identity` | 1 | main | project-session-actions「恢复保留 agent 身份与模型面」 | ❌ | B | rail 行操作与恢复确认；身份四件原样带回 |
| 9 | `archive-restore` | 1 | main | project-session-actions「restore archived session」 | ✅ | B | 归档/恢复入口在 rail；恢复确认文案与只读语义保留 |
| 10 | `auth` | 4 | auth | 鉴权与访问旅程「登录闭环」 | ✅ | A | 登录/登出/深链重定向语义不变；表单与错误呈现换 AntD |
| 11 | `auth-off` | 1 | main | 鉴权与访问旅程「免登录直达」 | ✅ | A | 单机免登录直达工作台 |
| 12 | `auth-setup` | 4 | auth-setup | 鉴权与访问旅程「首启 root 引导」 | ✅ | A | 首启建户、409、校验、重载保持 |
| 13 | `conversation` | 5 | main | agent-workbench「单一对话面」「模型目录」 | ❌ | B | 主区结构重排；两侧交替、工具合并块、就地聚焦、模型两级选择语义保留 |
| 14 | `conversation-streaming` | 1 | main | agent 对话覆盖「流式分批渲染」+ live-turn-stream | ✅ | B | live-tail → 结算 Markdown 切换；`retries: 0`、零固定 sleep 纪律保留 |
| 15 | `core-freeze-pending-ack` | 2 | main | 提交确认有界（SIGSTOP） | ❌ | A | 等待回执即时可见、解冻免刷新落账；交互位置可随待办区调整，语义不变 |
| 16 | `core-link-badge` | 2 | detached | agent-workbench「core 连接状态常驻指示」 | ✅ | B | 连接徽标移入新导航/状态区；ok/断连/恢复三态 + cause 保留 |
| 17 | `deployment` | 1 | detached | webui「全局核心可达性横幅」 | ✅ | B | fatal 横幅 + 锁定 + composer 门禁 + 降级提示；呈现位置随新壳调整 |
| 18 | `dialog` | 3 | main | agent 对话覆盖「多轮连续」「输入守卫」 | ✅ | B | composer 重排；多轮顺序、空输入守卫、特殊字符不回退 |
| 19 | `empty-turn-notice` | 4 | main | close-acceptance-blind-spots「零输出回合 notice」 | ❌ | B | 主题断言从 `wa-dark` 类改为新主题标记；notice 条目语义不变 |
| 20 | `error-toast` | 1 | main | webui「error 默认自动消失」 | ✅ | A | 8s 自动消失 + 挤占；AntD message 需保留同等寿命语义 |
| 21 | `errors` | 3 | main | 会话核心旅程「错误呈现」 | ✅ | A | refuse/crash/spawn-failed 诚实呈现；错误分类与只读保留 |
| 22 | `first-paint` | 2 | main | 工作台、项目与会话管理面「首屏与 reachability」 | ✅ | B | 新导航轨/单工作区首屏；创建对话框 agent 选择与 native 诚实标记 |
| 23 | `folder-picker-mkdir` | 3 | main | webui/projects「目录选择器可新建子目录」 | ✅ | A | 选择器换 AntD Tree，API `fs/mkdir` 与类型化拒绝不变 |
| 24 | `icons-local` | 1 | main | fix-webui-approval-restore 5.4（无 delta） | ✅ | A | 零 CDN、自托管图标；新栈断言改为无外链资源 |
| 25 | `layout` | 4 | main | 工作台布局分割（task 级） | ❌ | C | 拖拽 rail/composer 分割与浮岛视觉是旧交互；蓝图决定是否保留可拖拽边界、是否保留浮岛 |
| 26 | `mode` | 5 | main | agent mode 选择 / fix-webui-qa-round3 6.2 | ✅ | B | 创建与 composer 的模式控件合并进新工具栏；四词、allow/auto 等价、单行不折行保留 |
| 27 | `models` | 8 | main | acp-model-selection、webui provider 管理 | ✅ | B | 设置工作区 Models/Providers；抓取→保存→创建对话框免刷新链路保留 |
| 28 | `parallel-approval-routing` | 2 | main | permission-flow「决策精确路由」 | ❌ | B | 待办区多卡；乱序决策与刷新后路由精确性必须保留 |
| 29 | `parallel-permissions` | 1 | main | permission-flow「Parallel tool calls each get their own request id」 | ✅ | B | **ACP 桩侧载体**；独立 request_id、逐一决策 |
| 30 | `pending-stack` | 3 | main | agent-workbench「Pending submissions stack above the composer」 | ✅ | B | 会话内待办区；上移/移除/丢弃/拒绝两级反馈保留 |
| 31 | `permission` | 4 | main | 审批卡片旅程（拒绝/单次/会话级/多轮） | ✅ | B | 待办区统一审批；三种决策语义与 rail waiting 投影保留 |
| 32 | `project-focus-reconcile` | 3 | main | agent-workbench「Focused session drives project context」 | ❌ | B | 项目头与 rail 同步；移除项目/切换项目后的空态与恢复 |
| 33 | `projects` | 6 | main | 项目管理覆盖（增删/异常/排序/选择器） | ✅ | B | rail 项目行 + AntD 对话框；排序、分支隐藏、移除阻断保留 |
| 34 | `provider-gate` | 3 | auth | provider-management「role-gated」 | ❌ | B | 设置工作区 Models/别名；写入口裁剪 + 服务端 403 |
| 35 | `qa-round10` | 3 | main | 转录直播/rail 历史徽章/composer 模型拒绝 | ❌ | B | 转录容器宽度、历史徽章、模型拒绝回退；布局重排后重新量测 |
| 36 | `qa-round14-rbac` | 4 | auth | webui-user-management「viewer 只读」 | ❌ | B | viewer 纯 GET、`/sessions` 只读、member 对照；新壳下重写选择器 |
| 37 | `qa-round14` | 6 | main | 创建失败/别名消费/历史展开/预命名 | ❌ | B | 创建弹窗、模型菜单、历史组、预命名；语义逐条保留 |
| 38 | `qa-round3` | 10 | main | fix-webui-qa-round3 多项（含 shadow 样式断言） | ✅ | B | 下拉连续点击、/sessions 1280、/usage 收纳、改名同步、History 标签、排序菜单、URL、长错误、thinking 折叠 |
| 39 | `qa-round6-row-name` | 1 | main | 占位会话行名 | ❌ | B | rail 行名回退规则不变 |
| 40 | `qa-round6-settings-interaction` | 2 | main | 设置交互稳定性 | ❌ | B | 设置工作区分区切换/对话框可重复打开；不得复现遮罩吞点击 |
| 41 | `qa-round7-transcript-layout` | 2 | main | agent-workbench「转录任意内容不破坏布局」 | ✅ | A | 超长无空格、GFM 表格横滚、pre 语义、CJK 折行；布局不变量跨框架保留 |
| 42 | `qa-round7-ws-auth` | 2 | auth | webui-ws-rpc「未认证客户端不反复撞升级端点」 | ✅ | A | 登录页 WS 静默与登录后实时链路 |
| 43 | `qa-round8-native` | 2 | native | fix-webui-qa-round8 native 转录/影子队列 | ❌ | B | **native 侧载体**；用户气泡、模型留痕、待执行栈 |
| 44 | `qa-round8-transcript-scroll` | 2 | main | 转录滚动跟随与跳到最新 | ✅ | C | 未读缝/浮标是新转录布局的一部分；蓝图定稿后改写滚动锚定断言 |
| 45 | `qa-round8` | 4 | main | 降级留痕/模型留痕/历史入口/别名管理 | ❌ | B | 四个产品面均随设置工作区与转录重排；留痕条目内容不变 |
| 46 | `rail-expand` | 3 | main | agent-workbench「Rail expansion」「Focused session drives project context」 | ✅ | B | 新导航轨展开态、焦点联动、深链落地项目名 |
| 47 | `rail-focus` | 1 | main | agent-workbench「rail selection renders immediately」 | ✅ | B | 点击会话即时渲染；节流窗口语义保留 |
| 48 | `row-menu-a11y` | 1 | main | project-session-actions「renaming from the rail」 | ✅ | A | 关闭态菜单项不暴露给 a11y 树；换 AntD Dropdown 后仍须成立 |
| 49 | `services` | 6 | main | webui「Services 分区与 router 状态归属」 | ❌ | B | 设置工作区 Services；受管行/合成行、按钮互斥、core 只读、force stop |
| 50 | `session-label` | 2 | main | project-session-actions「operator label takes precedence」 | ✅ | B | rail 行重命名与 API 写后免刷新翻名 |
| 51 | `session-mgmt` | 5 | main | 会话管理覆盖（close/archive/深链/模型面） | ✅ | B | 归档确认、History 倒序、退役路径、模型芯片 |
| 52 | `session-roundtrip` | 1 | main | agent 对话覆盖「首回合往返」 | ✅ | A | 提交→回复→done→reload 恢复 |
| 53 | `session-usage` | 2 | main | usage-statistics「会话级 token 用量可见」 | ✅ | A | 非零累计与「未上报」诚实态；芯片位置可随新头部调整 |
| 54 | `sessions` | 2 | main | 会话管理覆盖「多会话切换」「archive 写保护」 | ✅ | B | 双会话不串线、归档 400 与 restore 诚实语义 |
| 55 | `settings` | 8 | main | 设置面只读/分区 IA/写降级 | ✅ | B | 设置工作区：分区顺序、About 压底、Env 占位、裸 core 退化、CRUD 持久 |
| 56 | `singleprocess-dead-core` | 2 | dead-core | SPA 核心死亡诚实降级 | ❌ | B | ws-down 横幅、rail 不可达、草稿保留；新壳下重新定位 |
| 57 | `skills` | 4 | main | agent-skills「Skills 仓」 | ❌ | B | 设置工作区 Skills；预览、删除确认、同步投影 |
| 58 | `slash-commands` | 2 | main | session-slash-commands「命令面板」 | ✅ | B | composer 命令面板/补全/不支持拦截 |
| 59 | `stop-reachability` | 1 | main | agent-workbench「停止控件随回合结算消失」 | ✅ | C | 泊车卡与 composer 的几何遮挡关系由新待办区/输入区布局决定 |
| 60 | `stop-settle` | 2 | main | agent-workbench「停止后 transcript 有停止条目」 | ✅ | B | 停止条目、控件复位、释放泊车审批 fail-closed |
| 61 | `streaming` | 1 | main | agent 对话覆盖「流式分批渲染」聚合半边 | ✅ | A | 多 chunk 合并为一气泡并收敛 done；API oracle 不变 |
| 62 | `submit-ack` | 1 | main | 提交反馈时限（5s 内可见） | ❌ | B | 本地排队指示即时可见；新控件状态机重写 |
| 63 | `submit-control` | 3 | main | 提交控件状态机 | ❌ | B | idle/streaming/queued 三态；停止与排队语义保留 |
| 64 | `test-model-scenarios` | 2 | native | test 场景驱动的浏览器呈现 | ✅ | B | **native 侧载体**；并行审批、零输出 notice |
| 65 | `thinking-process-fold` | 1 | main | agent-workbench「process folds interleave」 | ❌ | B | **ACP 桩侧载体**；thinking 折叠与正文按序 |
| 66 | `tiered-notices` | 3 | detached | 分级通知层 / fatal 锁定 / 回合终点通知 | ✅ | B | fatal 锁与恢复、历史页驻留、未打开会话完成通知 |
| 67 | `tool-call-fold` | 2 | native | fold-tool-calls-into-process-tree（native） | ❌ | B | **native 侧载体**；工具 call id 精确配对、跨 run 重聚 |
| 68 | `unread-badge` | 2 | main | session-unread-badge | ✅ | C | 徽标与锚水位语义必须保留；呈现位置随导航轨定稿 |
| 69 | `unread-seam` | 1 | main | 未读分界线 | ❌ | C | 已读缝视觉/定位由新转录蓝图决定 |
| 70 | `usage` | 5 | main | usage-statistics「/usage」 | ✅ | A | 来源/粒度/维度、router 不可达、全零空态；图表可换实现，语义不变 |
| 71 | `users-admin` | 8 | auth | webui-user-management Users 闭环 | ❌ | B | 设置工作区 Users；409/弱密码/角色即时/最后 root 保护 |

统计（逐行数）：A = 15，B = 51，C = 5，D = 0，合计 71。C 类为 `layout`、`qa-round8-transcript-scroll`、`stop-reachability`、`unread-badge`、`unread-seam`。

## 4. 与 `COVERAGE.md` / README 对账

基线账本落后于 spec 树，属于**已知登记缺口**，不是迁移导致的缩面：

- `tests/acceptance/COVERAGE.md` 提及 51/71 个 spec；未登记 20 个：`agents-gate`、`agents-slots`、`core-freeze-pending-ack`、`empty-turn-notice`、`parallel-approval-routing`、`project-focus-reconcile`、`qa-round14-rbac`、`qa-round14`、`qa-round6-row-name`、`qa-round8-native`、`qa-round8-transcript-scroll`、`qa-round8`、`services`、`singleprocess-dead-core`、`skills`、`slash-commands`、`tool-call-fold`、`unread-seam`、`usage`、`users-admin`。
- `tests/testsuite-webui/README.md` 提及 41/71；缺口更大（含旧表未收录的辅助旅程）。
- 处理：5.1 迁移选择器时同步回填树形账本；本阶段只记录，不改账本。**不得**把未登记当作可删除。
- `COVERAGE.md` 豁免清单中的「浏览器级 workbench UI 渲染」已由本套件承接，迁移后仍须保留该豁免条的替代证据指向。

## 5. ACP / native 双通路保留核对

| 通路 | 驱动/装配 | 承载 spec | 迁移后要求 |
|---|---|---|---|
| ACP 桩（fake-claude） | `claude`、`claude-stream`、`claude-thinking`、`fakeacp` | `permission`、`parallel-permissions`、`thinking-process-fold`、`conversation-streaming`、`models`（fakeacp）、`stop-*`、`approval-*` | 两条通路不得互相替代；ACP 侧独立跑审批/thinking/流式结算 |
| native（test 场景模型） | `TESTSUITE_NATIVE=1` + `test/*` 场景 | `test-model-scenarios`、`qa-round8-native`、`tool-call-fold` | native 侧独立跑并行审批、零输出 notice、工具折叠、影子队列 |
| 双进程（detached） | core + 独立 `sebas webui` | `deployment`、`approval-detached`、`tiered-notices`、`core-link-badge` | 断连/恢复/审批跨进程链路保留 |
| 单进程死亡 | SIGKILL core --webui | `singleprocess-dead-core` | SPA 诚实降级保留 |

**明确保留的双载体**：并行审批（`parallel-permissions` ACP vs `test-model-scenarios` native）、thinking/过程折叠（ACP `thinking-process-fold` vs native `tool-call-fold`）、流式（ACP `conversation-streaming` vs native 场景）、零输出 notice（ACP `empty-turn-notice` vs native `test-model-scenarios`）。三处均不得因「已有另一条通路」而删除。

## 6. 5.1 / 5.2 已交付的新增旅程

React 入口装配（Vite 代理到一次性沙箱；5.3 起页面即生产入口 `/`）：

- `playwright.react.config.ts`：沙箱 9893（auth off）+ Vite 5290；页面导航（含
  `/sessions/{key}` 深链）就是生产 SPA 文档，无专用预览文档、无 `SEBAS_REACT_ENTRY`。
- `playwright.react-auth.config.ts`：沙箱 9892（auth on）+ Vite 5289，同一入口。
- `playwright.react-setup.config.ts`：沙箱 9891（auth on、零用户）+ Vite 5288，
  同一入口的首启建户姿态。
- 选择器层 `tests/helpers/pages/react.ts`：只用角色、可访问名称与稳定
  `data-testid`；AntD Select 的 0×0 `role=option` 镜像与两字中文按钮的
  可访问名空格在 helper 内集中处理，未写入任何 WA/Lit 内部结构。
- 已加入 `invoke testsuite-webui` 链（以及 `--case react` / `--case react-auth`
  / `--case react-setup`）。

新增 30 条用例（`tests/testsuite-webui/tests/`）：

| spec | 条数 | 覆盖 |
|---|---|---|
| `react-workbench.spec.ts` | 11 | 首屏/rail/nav、项目上下文建房、深链切换不串话、归档/历史/只读快照/恢复、待执行提交归档确认、审批单次允许、Markdown 长内容（表格块级横滚）、设置工作区无 URL/草稿保留/退休路由、关于真值、手填越界原因、总览与用量 |
| `react-responsive-a11y.spec.ts` | 10 | 375 抽屉导航与草稿、375 输入与审批、375 设置、深/浅主题 token 与持久化及对比度、键盘打开设置与 2px 焦点环、设置关闭焦点归还、reduced-motion、rail 键盘调宽跨刷新、768 图标轨浮层不回流、375/768/1024/1440 横向溢出矩阵 |
| `react-auth.spec.ts` | 6 | 深链门禁、错误凭据内显、登录续跳深链、cookie 跨刷新/登出、viewer 入口裁剪（建房入口、用户分区、服务/别名写控件）、viewer 服务分区只读说明 |
| `react-setup.spec.ts` | 3 | 首启建户门禁与客户端校验、创建 root 进入工作台并跨刷新、root 已存在时再次 setup 409 |

实测（一次性沙箱、`retries: 0`、无固定 sleep）：30 条全部按断言执行且
**30 条全绿**，无 `test.fail()` 标注残留。

上轮 3 条 `test.fail()` 已由实现修复，复测转绿后**移除标注，断言原样保留**：

1. 401：`AuthStore.authenticate` 现在提交 store 错误后重抛原始 `ApiError`，
   错误凭据在登录页就地显示「用户名或密码错误」。
2. reduced motion：`buildThemeConfig` 接收 `motion` 种子令牌，Drawer/mask
   过渡为 `0s`；全局 `tokens.css` 同时把动画/过渡压到 0s。
3. 设置焦点归还：`Shell` 记录打开前的 `document.activeElement`，关闭后恢复，
   Escape 落回「设置」入口。

复测第 2 轮发现的 viewer 服务只读说明不可达也已修复：`ServicesPanel` 在
`!canControlServices(role)` 时跳过 `/api/admin/services` 请求并无条件渲染
只读 Alert，服务端 403 仍是最终防线；对应 `test.fail()` 已转为通过并移除。

已复核收敛的偏差（不再列为问题）：

- 导航轨 240–360px 拖拽 + 方向键/Home/End 调宽，`sebas.rail-width` 跨刷新
  持久化；1024 收到 260px，768 为 56px 固定图标轨，hover/focus 浮层展开且
  主列保持 56px 沟槽（无回流、无页面横向滚动）；已无未引用的死 CSS。
- React 入口导入全局 `app.css`/`tokens.css`：统一 2px accent 焦点环、
  reduced-motion 全局抑制、600px 下 44px 触控预算。
- 「关于」分区落地，读取 `/api/about` 真值；项目支持展开/折叠并持久化；
  手填路径有独立校验与越界/不存在原因，提交按钮同步禁用。
- GFM 表格包进 `.sebas-table-scroll`，宽表在条目内横滚，页面不横滚。
- 5.3 切换后 `/` 即 React 文档；`/react-preview.html` 与 Lit 入口已移除。

### 6.1 既有 71 条 spec 的实际迁移状态（2026-11-09 收尾，非意向）

**静态迁移：71/71 + 4 条新增 React 旅程 = 75/75 个 spec 文件已改写为
React 公共语义（角色 / 可访问名 / 稳定 `data-testid`），仓库内已无
`wa-*`、`.service-*`、`.entry/.remove`、`[role="separator"].frame` 等
Lit/WA/Shadow DOM 选择器**（`services`、`tiered-notices`、`errors`、
`stop-settle`、`approval-restore`、`qa-round8-native` 在收尾批补齐）。
复核方式：`grep -rn '<wa-\|<sebas-\|shadowRoot\|>>>' tests/` 零命中；
仅剩注释文字（`mode.spec.ts:189` 的历史说明）与业务字符串
`.sebas-probe`，均非 DOM 选择器。既有 spec 无删除、无 `test.fail()`、
无新增 retry；仅保留平台性 `test.skip`（win32）与 detached 拓扑自卫
skip（main 唯一 skip 即此类既有自卫，非本轮新增）。

已实跑 config（**最终前端代码**，日志 `/tmp/sebas-react-antd-final-suite/`，
全部 `--retries=0`，未用 skip/fail/retry 掩盖失败）：

| config | 最终实际结果 | 说明 |
|---|---|---|
| `playwright.config.ts`（main） | **179 passed / 0 failed / 1 skipped**（8.4m，完整跑完） | 上轮 5 条实现缺陷全部由实现修复后转绿，断言未改；唯一 skip 为既有平台/拓扑自卫 |
| `react` | 21/21 | 独立沙箱 |
| `react-auth` | 6/6 | |
| `react-setup` | 3/3 | React 入口合计 30/30 |
| `auth` | 35/35 | 上轮 4 条（viewer composer、users-admin 确认/弱密码/自锁 title）已由实现修复 |
| `auth-setup` | 4/4 | |
| `detached` | 7 passed / 1 skipped | 恢复 toast 实现已修；skip 为拓扑自卫 |
| `deployment` | 1/1 | |
| `dead-core` | 1/1 | 断连 rail 文案实现已修 |
| `native` | **未跑** | 后端 native→pi 路由冲突仍在（基线冲突，非本轮回归） |

上轮记录、现已修复并复测转绿的实现缺陷（保留为验收轨迹，不再失败）：
`sessionLabel` 0-turn 占位回退「未命名会话」；`TranscriptView` 的
`escalate_downgrade` 单元；`writeRailExpanded`/`readRailExpanded` 表达
显式收起；`Composer` 斜杠补全后关闭弹层；`ConversationPane` 聚焦前
`armOpeningSeam`；Users 分区删除确认/弱密码客户端校验/自锁 title；
断连 rail 不可达文案；恢复后「核心已恢复」通知。

本轮及上轮测试侧修正（均为 AntD 6/React 语义适配，未放宽业务断言）：
AntD 选中标签断 `.ant-select-content`（`agents-slots`、`session-mgmt`、
`qa-round10`、`qa-round14`）；表格横滚断言移到 `.sebas-table-scroll`
包裹层（`qa-round7-transcript-layout`、`qa-round10`），保留「内容不撑破
页面」；`pre:has(code)` 区分 markdown 代码块与操作者明文气泡；弹窗遮罩
未退场前不点 rail 行（`archive-restore`）；AntD Modal 进入动画未稳前
不取换行几何（`qa-round3` D5）；`first-paint` 的 PI 可用态对照
`/api/agents` 真值而非固化本机 PATH；`project-focus-reconcile` 修掉
自身嵌错的 `workbench-home` 选择器；`rail-expand` 的 URL 断言改为有界
正向等待（创建仍在飞行中）。

**5.1 / 5.3 / 5.4 结论**：main 与全部非 native config 已真实全绿
（179 + 30 + 35 + 4 + 7 + 1 + 1）；5.3 生产入口已切换完成——`index.html`
即 React 文档（`src/react/main.tsx`），Vite 构建输入只剩 `index`，Lit/WA
源码、polyfill、workaround、旧预览与仅 Lit 测试全部移除，依赖树/源码/dist
与嵌入二进制扫描零旧栈运行依赖；5.4 文档（`docs/frontend-dev.md`、
testsuite README、本矩阵、COVERAGE）同步，命令在隔离环境实跑通过。但
`native` config 因后端 native→pi 路由冲突**未验证**，因此整个 change
仍不得宣称完成——该阻塞保留在最终报告与 tasks 中，native 基线须补跑；
5.2 独立视觉验收亦未完成，保持不勾。

## 7. 无静默缩面声明

- 71 个 spec 全部在本表有行；0 条 D（删除）。
- 旧框架专属的**浏览器断言**（Shadow DOM 内部结构、`wa-*` 标签、`wa-dark` 类）不是产品契约，迁移为语义角色/可访问名/稳定测试标识或新主题标记；对应产品旅程本身保留（标为 A/B）。
- 设置 Modal → 设置工作区、审批卡 → 会话内待办、rail → 导航轨是**已评审的交互调整**，全部标记 B；1.6 蓝图已通过（`review-decision.md`，2026-11-08），改写断言须按蓝图逐条留证。
- C 类 5 条在蓝图评审前保持冻结，不得以「实现困难」为由提前删除。
- 本阶段新增的 React 旅程不替代任何旧 spec；旧 Shadow DOM 断言的删除必须
  等对应 spec 在 React 入口等价迁移并留下新证据，且不得以「React 已有另一条
  用例」为由删减 ACP/native 双通路。
