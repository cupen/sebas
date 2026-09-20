# sebas webui browser-e2e（Playwright）

浏览器级旅程套件：真实 chromium 驱动 webui，被测后端是 `invoke testsuite-webui-server`
（tasks.py）装配的一次性沙箱（`sebas core --webui` + 独立 `sebas router --config …
--debug` 两进程调试形态 + `tests/bin` 的 fake-claude 桩）。绝不触碰真实
`~/.sebas` 与端口 9797。

## 一键运行

```bash
invoke testsuite-webui                 # 构建（dist 自动重建）→ 全量旅程 → 清理
invoke testsuite-webui --case auth     # 仅鉴权旅程（auth-on 形态，端口 9898）
invoke testsuite-webui --case auth-setup  # 仅首启建户旅程（auth 开、零用户，端口 9896）
invoke testsuite-webui --case first-paint  # 仅单个旅程 spec
```

直接跑（仓库根）：

```bash
cargo build --bin sebas --bin fake-claude
pnpm install --dir tests/testsuite-webui
pnpm --dir tests/testsuite-webui exec playwright install chromium
pnpm --dir tests/testsuite-webui exec playwright test
```

## 旅程与账本

树形账本，与 `tests/acceptance/COVERAGE.md` 的 `testsuite-webui-browser` 一节同源：
大功能 = spec 顶层 `test.describe`（对应 requirement），子功能 =
二层 `test.describe`，锚点格式 `<requirement>「<scenario>」`，对应
`openspec/specs/testsuite-webui-browser/spec.md` 的 scenario 名。

| 大功能 | 子功能 | 用例名 | spec 文件 | 锚点 |
|---|---|---|---|---|
| 工作台首屏 | 首屏结构与 reachability | renders rail, composer and honest reachability | `first-paint.spec.ts` | 工作台、项目与会话管理面旅程「首屏与 reachability」 |
| 鉴权闭环 | 深链重定向 | deep link under auth redirects to the login page | `auth.spec.ts` | 鉴权与访问旅程「登录闭环」 |
| 鉴权闭环 | 登录与登出 | session cookie survives reload until logout, then the gate returns | `auth.spec.ts` | 鉴权与访问旅程「登录闭环」 |
| 鉴权闭环 | 登录与登出 | wrong credentials rejected, admin/admin enters, logout returns | `auth.spec.ts` | 鉴权与访问旅程「登录闭环」 |
| 鉴权闭环 | 免登录直达 | the workbench is the homepage and no gate ever appears | `auth-off.spec.ts` | 鉴权与访问旅程「免登录直达」 |
| 首启 root 引导 | 首启门禁 | homepage shows the setup card and never flips to the login gate | `auth-setup.spec.ts` | 鉴权与访问旅程「首启 root 引导」 |
| 首启 root 引导 | 校验与建户 | in-place validation rejects short password and mismatch without submitting | `auth-setup.spec.ts` | 鉴权与访问旅程「首启 root 引导」 |
| 首启 root 引导 | 校验与建户 | creating root enters the workbench and the session survives reload | `auth-setup.spec.ts` | 鉴权与访问旅程「首启 root 引导」 |
| 首启 root 引导 | 校验与建户 | a second setup POST is refused after root exists (409) | `auth-setup.spec.ts` | 鉴权与访问旅程「首启 root 引导」 |
| agent 对话覆盖 | 首回合往返与重载恢复 | composer submit → reply → done → reload restores | `session-roundtrip.spec.ts` | agent 对话覆盖「首回合往返」、会话核心旅程「重载恢复」 |
| agent 对话覆盖 | 流式分批 | chunks arrive in batches and the turn converges to done | `streaming.spec.ts` | agent 对话覆盖「流式分批渲染」 |
| agent 对话覆盖 | 多轮连续 | 4.1 two consecutive rounds append in order and survive reload | `dialog.spec.ts` | agent 对话覆盖「同会话多轮连续」 |
| agent 对话覆盖 | 输入守卫 | 4.2 empty and blank input creates no turn, session stays usable | `dialog.spec.ts` | agent 对话覆盖「composer 输入守卫」 |
| agent 对话覆盖 | 输入守卫 | 4.2 special-char long text round-trips without loss or console errors | `dialog.spec.ts` | agent 对话覆盖「composer 输入守卫」 |
| agent 对话覆盖 | 错误诚实呈现 | refuse — non-terminal: session survives, next message works | `errors.spec.ts` | 会话核心旅程「错误呈现」 |
| agent 对话覆盖 | 错误诚实呈现 | crash — honest death: row retires to dormant, transcript retained, no fabricated success | `errors.spec.ts` | 会话核心旅程「错误呈现」（round4 1.2 retire-to-record 语义） |
| agent 对话覆盖 | spawn 失败内显 | spawn failure inline: error event in transcript, session stays as spawn-failed | `errors.spec.ts` | webui「web_spawn 失败的立即内显」（fail-fast-on-startup-errors） |
| 审批卡片旅程 | 拒绝路径 | deny path — refusal semantics, turn completes | `permission.spec.ts` | 审批卡片旅程「拒绝路径」 |
| 审批卡片旅程 | 单次允许路径 | allow-once path — allowed semantics, turn completes | `permission.spec.ts` | 审批卡片旅程「单次允许路径」 |
| 审批卡片旅程 | 会话级允许 | allow-session path — session switches to auto mode, follow-up is no longer gated | `permission.spec.ts` | 审批卡片旅程「会话级允许」 |
| 项目管理覆盖 | 增删 | add via project dialog appears in rail; removed project disappears | `projects.spec.ts` | 项目管理覆盖「项目增删」 |
| 项目管理覆盖 | 异常拒绝 | 1.1 illegal path 400, duplicate 409, removal persists across reload | `projects.spec.ts` | 项目管理覆盖「项目异常拒绝」 |
| 项目管理覆盖 | 排序与持久化 | 1.2 reorder persists; git branch shows, plain dir shows none | `projects.spec.ts` | 项目管理覆盖「项目排序与分支呈现」 |
| 项目管理覆盖 | 选择器交互 | P1 tree expand, click-select fills path, submit lands in rail | `projects.spec.ts` | 项目管理覆盖「folder-picker 树展开点选回填」 |
| 项目管理覆盖 | 选择器交互 | P2 empty path disables submit; missing path errors inline, dialog stays | `projects.spec.ts` | 项目管理覆盖「选择器空路径与非法路径」 |
| 会话管理 | close 与 archive | close removes the session from the active list | `session-mgmt.spec.ts` | 会话管理覆盖「close 与 archive」 |
| 会话管理 | close 与 archive | archive hides from list, shows in History | `session-mgmt.spec.ts` | 会话管理覆盖「close 与 archive」 |
| 会话管理 | 深链与退役路径 | deep link renders the session via SPA fallback; /settings redirects to / | `session-mgmt.spec.ts` | 会话管理覆盖「深链与退役路径」 |
| 会话管理 | 模型面诚实缺省 | model honest absence — no selector, switch attempt keeps model absent | `session-mgmt.spec.ts` | 工作台、项目与会话管理面旅程「模型面诚实缺省」 |
| 会话管理 | 多会话切换 | 2.1 dual-session switch (rail + deep-link) does not crosstalk | `sessions.spec.ts` | 会话管理覆盖「多会话切换」 |
| 会话管理 | archive 写保护 | 2.2 archive → 400 write-protection → restore unhides honestly | `sessions.spec.ts` | 会话管理覆盖「archive 写保护与 restore 诚实语义」（restore 语义随 fix-webui-qa-defects 翻转为重建）² |
| 会话管理 | 归档恢复重建 | archive → restore: rail row back, transcript intact, History cleared, writable again | `archive-restore.spec.ts` | project-session-actions「restore archived session」+「restore preserves the transcript」² |
| 会话管理 | rail 切换即时聚焦 | 4.2 focusing A and clicking B in the rail renders B within the throttle window, with no other events | `rail-focus.spec.ts` | agent-workbench「rail selection renders the conversation immediately」² |
| 审批卡片旅程 | 审批读模型恢复 | reload rebuilds the review card from the read model under the same request_id, and deciding clears it for good | `approval-restore.spec.ts` | permission-flow「刷新或重连后仍可取得」+ agent-workbench「刷新后审批面从读模型重建」³ |
| 停止收尾 | 回合被停止条目 | stopping a streaming turn appends the stop entry, resets the control, and stays settled across reloads | `stop-settle.spec.ts` | agent-workbench「停止后 transcript 有停止条目」+「刷新后不复活在飞状态」³ |
| 停止收尾 | 停止释放泊车审批 | stopping a parked turn releases the approval fail-closed: read model drains, late decision is rejected, stop entry lands | `stop-settle.spec.ts` | permission-flow「停止回复清空未决审批」+「释放的请求不可再批复」³ |
| 停止收尾 | 泊车审批态下停止控件可点 | parked: stop control is unobstructed by the composer input and a plain click lands | `stop-reachability.spec.ts` | agent-workbench「停止控件随回合结算消失」+ permission-flow「停止回复清空未决审批」（round6 固化：textarea 盒体不得遮挡停止控件） |
| 会话管理 | 聚焦联动与展开持久 | rail switch and creation landing drive the project title; project-row clicks stay independent | `rail-expand.spec.ts` | agent-workbench「rail 切换会话后项目标题跟随」+「项目行点击仍独立生效」³ |
| 会话管理 | 聚焦联动与展开持久 | expansion persists across reloads; the focused project defaults to expanded with no record | `rail-expand.spec.ts` | agent-workbench「展开状态跨刷新保持」+「聚焦会话所在项目缺省展开」³ |
| 会话管理 | 聚焦联动与展开持久 | deep link lands the focused session project in the main title, never 「未选择项目」 | `rail-expand.spec.ts` | agent-workbench「rail 切换会话后项目标题跟随」深链/刷新直达半边（fix-webui-qa-defects-round3 tasks 4.2/6.3 补齐）⁴ |
| 会话管理 | 行重命名 | label takes precedence over the first-prompt preview, survives reloads, and clearing falls back | `session-label.spec.ts` | project-session-actions「operator label takes precedence」+「renaming from the rail」³ |
| 会话管理 | 行重命名 | a label write through the API flips the rail row live, without a reload (round5 C) | `session-label.spec.ts` | project-session-actions「label writes through any path update the row live」（fix-webui-qa-defects-round5 delta⁴） |
| 会话管理 | 行菜单可达性 | closed row menus expose no menu items to the a11y tree; open ones do | `row-menu-a11y.spec.ts` | project-session-actions「renaming from the rail」（fix-webui-qa-defects-round5 tasks 4.1：关闭态菜单项对辅助技术隐藏）⁴ |
| 未读徽标 | 聚焦到达与重复聚焦 | focused arrivals never badge; repeated same-session clicks keep it cleared | `unread-badge.spec.ts` | session-unread-badge（fix-webui-qa-defects-round3 delta⁴）「Streaming arrival into the focused session does not badge」+「Repeated focus keeps the badge cleared」 |
| agent mode 选择 | composer 模式下拉形态 | composer mode dropdown options stay single-line (round3 6.2) | `mode.spec.ts` | fix-webui-qa-defects-round3 tasks 4.1/6.2（无 delta scenario，task 级验收：选项单行不折行；add-agent-mode-selection 词汇同源）⁴ |
| 审批卡片旅程 | 相位对账与挂载去重 | waiting-phase empty pull backs off and retries until the card lands without any phase change (round3 7.1) | `approval-reconcile.spec.ts` | fix-webui-qa-defects-round3 tasks 7.1（无 delta scenario，task 级验收：waiting 扑空退避重试补卡；permission-flow「刷新或重连后仍可取得」的对账纵深）⁴ |
| 审批卡片旅程 | 相位对账与挂载去重 | mount and session switch pull the approvals read model exactly once (round3 7.2) | `approval-reconcile.spec.ts` | fix-webui-qa-defects-round3 tasks 7.2（无 delta scenario，task 级验收：挂载期拉取去重 + pullSeq 防陈旧不回归）⁴ |
| 待执行堆叠区 | 队列管理面 | move up and remove ride the composite to the hosting backend; queue and API agree (round5 A) | `pending-stack.spec.ts` | core-session-channel（fix-webui-qa-defects-round5 delta⁴）「reorder from the web UI in an embedded deployment」+「removing a pending submission over the channel」 |
| 分级通知层 | error toast 寿命 | an error toast auto-dismisses after ~8s without manual closing | `error-toast.spec.ts` | webui（fix-webui-qa-defects-round5 delta⁴）「error 默认自动消失且参与挤占」的寿命半边（挤占半边由 notify/notice-layer 单测承载） |
| 会话管理 | 归档恢复身份 | archived→restored keeps agent_kind, mode and the model catalog; legacy entries fall back honestly | `archive-identity.spec.ts` | project-session-actions「恢复保留 agent 身份与模型面」+「旧归档条目如实回退」³ |
| 项目管理覆盖 | 越界禁用原因 | out-of-root and missing paths state their reason with submit disabled; a valid path enables it | `add-scope-reason.spec.ts` | workspace-root「手填越界路径给出禁用原因」³ |
| 工作台首屏 | 图标本地化 | blocking the icon CDN changes nothing: zero CDN requests, local /icons assets, icons still render | `icons-local.spec.ts` | fix-webui-approval-restore-and-session-identity tasks 5.4（无 delta scenario，task 级验收：阻断 CDN 零请求 + 本地 icons 资源） |
| 模型管理覆盖 | 无模型诚实缺省 | 3.2 set_model on a model-less session fails terminally and honestly | `models.spec.ts` | 模型管理覆盖「无模型会话 set_model 诚实拒绝」 |
| 模型管理覆盖 | settings provider 只读 | 3.2 settings provider list matches API, zero probe traffic | `models.spec.ts` | 模型管理覆盖「settings provider 只读」 |
| 设置面 ¹ | 只读呈现 | S1 services rows match /api/admin/services truth (response-driven) | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S2 about table matches /api/about truth | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S3 env table renders placeholder semantics under Env Vars | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S6 bare core degrades: no-adapter banner, no rows, restart disabled | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 写降级 | S4 defaults read parity; sandbox write fails honestly | `settings.spec.ts` | 设置面写操作诚实降级覆盖「写降级失败外显且状态不变」¹ |
| 设置面 ¹ | 写降级 | S5a create/edit mutations: client validation + honest 503 | `settings.spec.ts` | 模型管理覆盖「settings provider 只读」 |
| 设置面 ¹ | 写降级 | S5b delete/probe mutations fail honestly, list unchanged | `settings.spec.ts` | 模型管理覆盖「settings provider 只读」 |

> ¹ 设置面（S1–S4, S6）的锚点指向尚未归档的 change `expand-webui-e2e-settings` 的 delta
> scenario（尚未同步进主 spec）；S5a/S5b 锚到主 spec `模型管理覆盖`「settings provider
> 只读」（provider 写 503 语义已由该 scenario 吸收）。harness 级 scenario（沙箱装配、
> 一键入口）不设单用例行。
>
> ² 归档恢复与 rail 聚焦聚焦行的锚点指向尚未归档的 change `fix-webui-qa-defects` 的
> delta scenario；同步时注意主 spec `testsuite-webui-browser`「archive 写保护与
> restore 诚实语义」的旧文案（「restore 不复活会话，详情页如实 404」）需随 D1 语义
> （restore = 重建会话行 + 保留转写）一并改写。
>
> ³ 审批读模型恢复 / 停止收尾 / 聚焦联动与展开持久 / 行重命名 / 归档恢复身份 /
> 越界禁用原因各行的锚点指向尚未归档的 change
> `fix-webui-approval-restore-and-session-identity` 的 delta scenario；同步主 spec
> 时随该 change 归档一并落锚。
>
> ⁴ 未读徽标聚焦到达 / 深链项目标题 / 模式下拉单行 / 审批对账与挂载去重 /
> 队列管理面 / API 写 label 即时行名 / 行菜单 a11y / error toast 寿命各行的
> 锚点指向尚未归档的 change `fix-webui-qa-defects-round3`（聚焦到达 delta、
> tasks 4.1/6.2/4.2/6.3/7.1/7.2）与 `fix-webui-qa-defects-round5`（core-session-channel
> 与 project-session-actions 的 label delta、webui 的 error toast delta、
> tasks 4.1）；同步主 spec 时随归档一并落锚。
>
> ⁵ 焦点敏感旅程的等待纪律：detail 读取（GET /api/sessions/{key}）即设服务端
> 焦点（api.rs「Reading the detail focuses this session」）——点击聚焦之后再轮
> 询被观测会话的详情会把焦点偷回去，被测语义失真（unread-badge 原用例曾因此
> 假红）。聚焦敏感段一律走 `waitListStatus`（列表轮询无焦点副作用）。
>
> 首启 root 引导四例（`auth-setup.spec.ts`）跑在第三种沙箱形态上：auth 开关打开、
> 零用户、不预建户（tasks.py `TESTSUITE_AUTH_SETUP=1`，端口 9896）——首启设置页
> 姿态本身是旅程被测前提，main/auth 两种形态都构造不出来。
>
> Settings 语义修正（fix-settings-menu-and-services-semantics，spec 改动 `1e4a807`）：
> S1 改写为 `/api/admin/services` 响应驱动、S6 新增裸 core 退化覆盖；实施清单见
> `openspec/changes/fix-settings-menu-and-services-semantics/tasks.md` §4。

能力矩阵账本见 `tests/acceptance/COVERAGE.md` 的 `testsuite-webui-browser` 一节。

## 调试开关

| 开关 | 作用 |
|---|---|
| `TESTSUITE_KEEP=1 invoke testsuite-webui` | 通过后也保留沙箱现场（排障）|
| `TESTSUITE_REUSE=1` | 复用上一次保留的沙箱（配合 TESTSUITE_KEEP）|
| `TESTSUITE_PORT=<port>` | 覆盖沙箱端口（默认 9899；`TESTSUITE_AUTH=1` 时 9898；`TESTSUITE_AUTH_SETUP=1` 时 9896）|

任一用例失败时，keep-on-fail reporter 会把沙箱目录保留下来并在输出里打印路径
（含后端日志 `core.log`），供复现。

## WS 帧形态（add-ws-rpc-protocol 起）

`/ws` 上的事件以 RPC 封套投递：`Notification{method, params}`，`method` 即原
事件 type（`turn.append`、`session.updated` 等），原载荷整体在 `params` 下、
字段名逐字保真；裸 `{type, ...}` 帧已退役。断言实时事件的新用例请匹配
`v["method"]` 并从 `v["params"]` 取字段（进程级参照
`tests/testsuite_e2e_test.rs` 的 `turn_appends_stream_over_ws`，集成参照
`sebas-webui/tests/ws_test.rs`）。协议往返可用内置 `ping` 方法自证。

## core 可达性推送（add-core-reachability-ws-push 起）

前端不再 5s 轮询 `/api/summary` 取可达性：app-shell 在 WS 连接建立/重连时发
`core.reachability.get` 取当前态，此后随 `core.reachability` 翻转通知即时
更新（横幅与 composer 提交门同源）。涉及横幅/提交门断言的新用例请直接驱动
沙箱 core 进程起停（翻转即达，无需等轮询窗）；集成级参照
`sebas-webui/tests/ws_test.rs` 的 FlipBackend 用例。

## 平台适配

Linux（含 headless CI/云端）为主；Windows（Git Bash/msys）尽力而为：
脚本内 `cygpath -m` 转换 config 路径、`.exe` 后缀探测、短沙箱目录名
（named pipe 256 字符上限）、`.gitattributes` 强制 `*.sh eol=lf`。
