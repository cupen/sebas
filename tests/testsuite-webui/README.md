# sebas webui browser-e2e（Playwright）

浏览器级旅程套件：真实 chromium 驱动 webui，被测后端是 `invoke testsuite-webui-server`
（tasks.py）装配的一次性沙箱（`sebas core --webui` + 独立 `sebas router --config …
--debug` 两进程调试形态 + `tests/bin` 的 fake-claude 桩）。绝不触碰真实
`~/.sebas` 与端口 9797。

## 一键运行

```bash
invoke testsuite-webui                 # 构建（dist 自动重建）→ 全量旅程 → 清理
invoke testsuite-webui --case auth     # 仅鉴权旅程（auth-on 形态，端口 9898）
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

树形账本，与 `tests/acceptance/COVERAGE.md` 的 `testsuite-webui-browser` 一节同源
（同为 35 行）：大功能 = spec 顶层 `test.describe`（对应 requirement），子功能 =
二层 `test.describe`，锚点格式 `<requirement>「<scenario>」`，对应
`openspec/specs/testsuite-webui-browser/spec.md` 的 scenario 名。

| 大功能 | 子功能 | 用例名 | spec 文件 | 锚点 |
|---|---|---|---|---|
| 工作台首屏 | 首屏结构与 reachability | renders rail, composer and honest reachability | `first-paint.spec.ts` | 工作台、项目与会话管理面旅程「首屏与 reachability」 |
| 鉴权闭环 | 深链重定向 | deep link under auth redirects to the login page | `auth.spec.ts` | 鉴权与访问旅程「登录闭环」 |
| 鉴权闭环 | 登录与登出 | session cookie survives reload until logout, then the gate returns | `auth.spec.ts` | 鉴权与访问旅程「登录闭环」 |
| 鉴权闭环 | 登录与登出 | wrong credentials rejected, admin/admin enters, logout returns | `auth.spec.ts` | 鉴权与访问旅程「登录闭环」 |
| agent 对话覆盖 | 首回合往返与重载恢复 | composer submit → reply → done → reload restores | `session-roundtrip.spec.ts` | agent 对话覆盖「首回合往返」、会话核心旅程「重载恢复」 |
| agent 对话覆盖 | 流式分批 | chunks arrive in batches and the turn converges to done | `streaming.spec.ts` | agent 对话覆盖「流式分批渲染」 |
| agent 对话覆盖 | 多轮连续 | 4.1 two consecutive rounds append in order and survive reload | `dialog.spec.ts` | agent 对话覆盖「同会话多轮连续」 |
| agent 对话覆盖 | 输入守卫 | 4.2 empty and blank input creates no turn, session stays usable | `dialog.spec.ts` | agent 对话覆盖「composer 输入守卫」 |
| agent 对话覆盖 | 输入守卫 | 4.2 special-char long text round-trips without loss or console errors | `dialog.spec.ts` | agent 对话覆盖「composer 输入守卫」 |
| agent 对话覆盖 | 错误诚实呈现 | refuse — non-terminal: session survives, next message works | `errors.spec.ts` | 会话核心旅程「错误呈现」 |
| agent 对话覆盖 | 错误诚实呈现 | crash — honest death: not-found presentation, no fake success | `errors.spec.ts` | 会话核心旅程「错误呈现」 |
| agent 对话覆盖 | spawn 失败内显 | spawn failure inline: error event in transcript, session stays as spawn-failed | `errors.spec.ts` | webui「web_spawn 失败的立即内显」（fail-fast-on-startup-errors） |
| 审批卡片旅程 | 拒绝路径 | deny path — refusal semantics, turn completes | `permission.spec.ts` | 审批卡片旅程「拒绝路径」 |
| 审批卡片旅程 | 单次允许路径 | allow-once path — allowed semantics, turn completes | `permission.spec.ts` | 审批卡片旅程「单次允许路径」 |
| 审批卡片旅程 | 会话级允许 | allow-session path — observed product gap: the follow-up call is gated again | `permission.spec.ts` | 审批卡片旅程「会话级允许」 |
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
| 会话管理 | archive 写保护 | 2.2 archive → 400 write-protection → restore unhides honestly | `sessions.spec.ts` | 会话管理覆盖「archive 写保护与 restore 诚实语义」 |
| 模型管理覆盖 | 无模型诚实缺省 | 3.2 set_model on a model-less session fails terminally and honestly | `models.spec.ts` | 模型管理覆盖「无模型会话 set_model 诚实拒绝」 |
| 模型管理覆盖 | settings provider 只读 | 3.2 settings provider list matches API, zero probe traffic | `models.spec.ts` | 模型管理覆盖「settings provider 只读」 |
| 设置面 ¹ | 只读呈现 | S1 services rows match /api/admin/services truth (response-driven) | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S2 about table matches /api/about truth | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S3 env table renders placeholder semantics | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 只读呈现 | S6 bare core degrades: no-adapter banner, no rows, restart disabled | `settings.spec.ts` | 设置面只读呈现覆盖「只读分区与 API 对账」¹ |
| 设置面 ¹ | 写降级 | S4 defaults read parity; sandbox write fails honestly | `settings.spec.ts` | 设置面写操作诚实降级覆盖「写降级失败外显且状态不变」¹ |
| 设置面 ¹ | 写降级 | S5a create/edit mutations: client validation + honest 503 | `settings.spec.ts` | 模型管理覆盖「settings provider 只读」 |
| 设置面 ¹ | 写降级 | S5b delete/probe mutations fail honestly, list unchanged | `settings.spec.ts` | 模型管理覆盖「settings provider 只读」 |

> ¹ 设置面（S1–S4, S6）的锚点指向尚未归档的 change `expand-webui-e2e-settings` 的 delta
> scenario（尚未同步进主 spec）；S5a/S5b 锚到主 spec `模型管理覆盖`「settings provider
> 只读」（provider 写 503 语义已由该 scenario 吸收）。harness 级 scenario（沙箱装配、
> 一键入口）与鉴权「免登录直达」（全部主 config 用例隐式承担）不设单用例行。
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
| `TESTSUITE_PORT=<port>` | 覆盖沙箱端口（默认 9899；`TESTSUITE_AUTH=1` 时 9898）|

任一用例失败时，keep-on-fail reporter 会把沙箱目录保留下来并在输出里打印路径
（含后端日志 `core.log`），供复现。

## 平台适配

Linux（含 headless CI/云端）为主；Windows（Git Bash/msys）尽力而为：
脚本内 `cygpath -m` 转换 config 路径、`.exe` 后缀探测、短沙箱目录名
（named pipe 256 字符上限）、`.gitattributes` 强制 `*.sh eol=lf`。
