# QA-B findings —— 会话核心链路簇（round13）

- 测试人：QA-B subagent（纯 GUI 黑盒，真实浏览器 = Playwright Chromium 1440x900，持久 profile 逐脚本驱动，全程 console/pageerror/HTTP 监听）
- 被测：http://127.0.0.1:9877/ ，admin/admin（root）
- ACP = fake-claude 桩；native = debug router `test/*` 场景模型
- 开始时间：2026-10-04
- 截图目录：openspec/changes/fix-webui-qa-round13/verification/shots/（b01–b64）
- 驱动脚本与 console 台账：verification/driver-b/（console-b.jsonl）

## 环境变更记录（主 agent 指示保留时间线）

1. **实际使用端口：127.0.0.1:9877（主沙箱），全程未换。** 中途两次服务中断（见环境-2），恢复后继续在同一实例测试。
2. 事故责任更正（留痕）：主 agent 通报称清场来自 QA-B 运行的 `invoke testsuite-webui-sandbox`——**QA-B 全程未运行任何 invoke/cargo 命令**（Bash 台账：curl 探活、node+Playwright GUI 驱动脚本、tasklist/netstat 只读排查）；`Temp/sbtestsuite.*`（9894）进程系 QA-B 排查宕机时**发现已存在**（第一次 .7342ql4c，创建 06:30:57 恰在事故窗口；第二次 .c0lnx5yd，在第二次中断窗口出现），非 QA-B 启动。采纳「根因=主沙箱缺 native env + sbtestsuite 清场波及」的结论，但启动者非 QA-B 这一点以本记录为准。
3. 第一次恢复后环境形态变化：settings.db 被重置（自建 provider fake 消失，出现内置 minimax/MiniMax-M3 已配 key）；历史页残留 2 条 7 天前旧 hello 会话（Dormant）——恰为在册项 N-1 的对照组：API 建的会话跨重启存活，GUI 工作台建的会话蒸发（本轮 GUI 建的 hello 会话重启后消失，另见 PASS-E.15 的 N-1 形态实录）。
4. 恢复后 /api/summary 仍报 `native ok:false`（SEBAS_AGENT_ROUTER_URL 未设），新建会话对话框 Native Kernel 选项仍 disabled；Agent 下拉新增场景 agent：**empty / error / slash / slow / thinking**（+claude）。BRIEF 中 native `test/*` 场景项改由这些场景 agent 在 ACP 通道覆盖（thinking/empty/error/slow ↔ test/thinking、test/empty、test/error、test/long 形态对应），B.4/B.6/B.7/B.8 据此执行。
5. 22:30 前的 native 失败类观察已全部按环境问题处理（见环境-1），未计入产品缺陷。

## PASS 项（20 项）

| # | 项 | 结论与证据 |
|---|---|---|
| 1 | 登录（前置） | admin/admin 登录直达工作台无报错。b01/b02/b03 |
| 2 | A.1 会话创建 | 项目行 + → 对话框（Agent/Provider/模型/权限模式）→ toast「已在「work」创建新会话。」；5 个场景会话批量创建成功（work 计数 5）。b08/b23/b44 |
| 3 | A.1 自动命名 | 首条消息后「未命名会话」自动改名（hello / line1 line2 / drip / 流式测试请慢速回答 / 给我一个空回合）。b25/b26/b46/b59 |
| 4 | A.2 项目注册 | 「添加项目」弹目录树（钉在 workspace root）+手填路径+执行节点；注册成功 toast + 树节点 work(local)。b04/b05/b06/b42 |
| 5 | B.3 纯正文回合（acp） | hello → 「hello world」，秒级终态，头部 token 出现（in 100·out 10）。b25/b26 |
| 6 | B.4 thinking 呈现 | thinking agent：过程 chip「thinking 1」折叠态；点击展开为虚线面板（💭 thinking + 内容 hmm），块序如实（thinking→text→thinking→text）；正文「thought out loud / and the answer」。b46/b47/b48 |
| 7 | B.5 并行审批卡 | `parallel`（Ask 档）：权限审批区出现**两组独立**的 仅允许一次/本会话内允许/拒绝（Bash echo first claude:tc-par-1-2 + Read /tmp/fake-parallel.txt），侧栏会话行亮「等待」徽章。批准动作被第二次服务中断打断（见环境-2#2）。b63 |
| 8 | B.9 多行输入 | Shift+Enter 换行（textarea value="line1\nline2"），发送后气泡按两行渲染。b45/b46 |
| 9 | C.10 ask 档批准 | 审批卡（Bash rm -rf /）+状态条「会话在等你的权限批复」+停止回复按钮；批准→✓Bash 已执行→perm done→环后正文 perm turn finished。b28/b29 |
| 10 | C.10 ask 档拒绝 | 拒绝→红色「Bash ✗ 已拒绝」+denied by fake→perm turn finished；会话可继续。b30/b31 |
| 11 | C.10 edit 档 | Edit·自动接受编辑下 Bash 仍出卡（acceptEdits 语义正确）；批准→✓已执行→perm done→perm turn finished。b33/b34/b35 |
| 12 | C.10 auto 档（间接） | allow 档实测走了与 auto 相同的直接执行分支（见缺陷 B-1 的另一面：bypass 分支形态=✓已执行+无收尾正文，与 round10/11 核实的 auto 桩设计一致）；auto 档本身的独立复测被第二次中断阻断 |
| 13 | C.11 切换回执 | 切 Edit/Allow 均落系统卡「权限模式已切换：自动接受编辑/放行」，头部徽章与 composer combobox 同步。b33/b36/b37 |
| 14 | B.6/D.12 流式与停止 | slow 会话回合中「停止回复」按钮出现（红色方块，b50）；T+1s 点击→按钮变「停止中…」→系统卡「提示：回合已取消（操作者停止了本次回复）。」落转录→跟发消息正常回复。b58/b59 |
| 15 | B.7 空回合 | empty agent：零输出回合落提示卡「回合已结束且无输出：本回合未产生任何可见输出（正文、thinking、工具、错误皆无）。」不白屏、状态正常、token 记账 in 100·out 10。b60 |
| 16 | B.8 错误回合 | error agent：红卡「! 错误 upstream error (fake): provider returned 500」；错误后跟发消息仍正常回复（hello world）。b61/b62 |
| 17 | E.15 历史页 | /sessions 列表、统计徽章（0 活跃 2 休眠 2 总计）、每行 Dormant 状态+聚焦/关闭按钮、新建会话表单（任务描述+agent 下拉）。b41 |
| 18 | E.18 token 累计 | 多回合单调续增：hello(100/10)→perm×3(600/60)→edit 批准(1000/100)→allow(1500/150)；slow 会话两回合 200/20。无回退（crash 案例未能执行，见阻断项）。b29/b31/b35/b37/b59 |
| 19 | F.19 console 横切 | 全程监听：**pageerror=0，console.error=0，4xx/5xx=0**（console-b.jsonl 21 条均为驱动脚本自身定位错误与 2 次断连记录，非应用产生） |
| 20 | 深链形态（部分） | 会话头部深链可见可复制（/sessions/web%00web-…）。b23/b26（点击直达与死会话 404 有界性被中断阻断） |

## 缺陷

### B-1（P3）：Allow·放行 档的 `perm` 不出审批卡，直接执行且无环后正文——与验收预期「ask/edit/allow 档应出审批卡」不符
- 现象：composer 权限模式切到「Allow · 放行」（系统回执「权限模式已切换：放行」正常落卡）后发 `perm`：未出现审批卡，过程 chip 直接「✓ Bash ✓ 已执行 → perm done」，且其后没有「perm turn finished」收尾正文——形态与 Auto 档的 bypassPermissions 直接执行分支完全一致。BRIEF 预期 allow 档应出审批卡（批准后才执行并有环后正文）。
- 复现步骤：admin 登录 → work 项目会话（fake-claude）→ composer 权限模式选 Allow·放行 → 发送 `perm`。
- 证据：shots/b36_switched_allow.png（放行回执）、b37_allow_perm.png（无卡直接 ✓已执行、无环后正文）；对照 ask 档 b28/b29、edit 档 b34/b35。
- 初判：会话权限模式到 ACP agent 的映射把 放行 映射成了 bypassPermissions（桩因此走直接执行分支），或 BRIEF 的验收口径与现行设计不一致（放行=不问）。前端呈现无渲染问题；需产品/桩口径裁决，故 P3。

### B-2（P3，单采样观察）：thinking 过程展开面板内部留白偏大
- 现象：展开「过程 thinking 1」后，虚线面板高度较大，「💭 thinking」标签与内容「hmm」之间有明显空带（b48），视觉上像 min-height 撑高而非内容高度。
- 复现步骤：thinking agent 会话发任意消息 → 点击「过程 thinking 1」chip。
- 证据：shots/b48_thinking_expanded.png（对照折叠态 b47）。
- 初判：前端样式（展开面板最小高度/间距），打磨项；单采样未复试。

### 环境-1（P2 → 已被主 agent 定性为环境问题，非产品缺陷）：native 渠道整段不可用
- 时间线：测试初期 Native Kernel 选项 disabled（提示「未配置模型凭据——到「设置 → 模型」配置」）；Settings→模型 自建 provider（anthropic 协议、指向 127.0.0.1:8791、key 已配、设为新建会话默认）后仍禁用；/api/summary execution_bodies native ok:false（cause=native backend needs SEBAS_AGENT_PROVIDER_API_KEY (or SEBAS_AGENT_ROUTER_URL)）。主 agent 通报为起 core 时漏设 env。**恢复后 native 仍 ok:false**（06:47Z 复查），Native Kernel 依旧 disabled；主 agent 另以场景 agent（thinking/empty/error/slow）补齐形态覆盖。
- 遗留引导文案问题：「到「设置 → 模型」配置」的提示与实际启用条件（core 进程 env）不一致——在设置页配全 provider 也不会点亮该选项，引导落空。建议文案或启用条件二选一收敛（此条独立于环境修复，仍值得跟）。
- 证据：b09/b18/b20/b21/b22/b43。

### 环境-2（P1，服务存活，外部因素）：主沙箱 core(9877)+router(8791) 两次无日志消失
- #1：22:30:10Z core.log 末条（edit 档批准回执）后进程消失，连接拒绝约 2 分钟；core.log 无 panic/关机记录（Windows 强杀形态）；同时窗出现新 sbtestsuite 装配（9894）。恢复由主 agent 完成（非 QA-B）。
- #2：22:49Z（06:49 CST）第二次连接拒绝，本 QA-B 剩余项被迫终止；存活 sebas.exe 再次易主为更新的 sbtestsuite 装配（.c0lnx5yd）。恢复后 core 的日志另有去向（原 core.log 未再增长）。
- 证据：b37（中断前最后状态）、console-b.jsonl 22:31:05Z 与 22:49:42Z 两条 ERR_CONNECTION_REFUSED。
- 按 QA 纪律未自行重启/修复。

## 在册项验证（round12 遗留）

- **R12-B-1（crash 后 token 累计回退）**：未能复测——`crash` 触发回合未及执行即遇第二次服务中断。中断前的旁证：多回合 token 单调续增无回退（见 PASS-18）。结论：**未验证**（标记：本轮无法给结论）。
- **R12-B-2（历史页驻留 + 他会话回合完成 → info 瞬时条）**：未能执行（curl API 建会话的步骤在第二次中断前未轮到）。结论：**未验证**。
- **N-1（工作台直建会话不进 session_map、重启蒸发）**：**形态实录相符**——GUI 建的 hello 会话在 core 重启后从列表消失，而历史页 2 条 7 天前（API 建）的旧 hello 会话跨重启存活。非新缺陷，佐证在册项成立。
- **O-2（crash 恢复后「模型已切换：default → fake」假回执）**：未复测（crash 未执行）。
- **O-5（死会话深链首载集中 404）**：未复测（深链直达被中断阻断）；本次全程网络监听 4xx/5xx=0，无自发 404 噪音。

## 阻断/未覆盖项（全部因第二次服务中断）

- C.10 auto 档独立复测、C.11 切换状态 F5 保持、B.5 双卡批准完成态、D.13 crash + 头部 token 续增（R12-B-1）、D.14 F5 历史保持、E.16 R12-B-2 info 瞬时条、E.17 深链直达/死会话 404 有界性、F.20 登出后登录页零 console 错误（round11 修复不回归）。
- B.6 增量渲染的逐帧目验：slow/drip/stream 三种桩均 <6s 完成，0.8s/1.5s 采样未能捕捉中间帧（b50 证明回合中停止按钮在位，b53 显示 3s 时 drip 已完整呈现「drip0 drip1 drip2」）；非缺陷，采样局限如实记录。

## console pageerror 台账（F.19）

- pageerror：**0**
- console.error：**0**
- HTTP ≥400：**0**（两次 ERR_CONNECTION_REFUSED 属服务中断，非页面请求失败；无 404/5xx 噪音）
- 明细文件：verification/driver-b/console-b.jsonl（21 条：19 条驱动脚本自身错误、2 条断连）
