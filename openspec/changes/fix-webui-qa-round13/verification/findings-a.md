# QA-A findings —— 认证/RBAC/项目/设置/技能/agents/usage 簇 + QA-B 未覆盖补测（round13）

- 测试人：QA-A subagent（纯 GUI 黑盒，真实浏览器 = Playwright Chromium 1440x900 headless 持久 profile 逐脚本驱动，全程 console/pageerror/HTTP ≥400 监听）
- 被测：http://127.0.0.1:9877/ （auth 开，全新库）；admin/admin（root）、member/member、viewer/viewer
- ACP = fake-claude 桩（claude）；native = debug router 127.0.0.1:8791 `test/*` 场景模型
- 开始时间：2026-10-04 07:0x（CST）；截图 shots/a01–a##；驱动脚本 verification/driver-a/（console-a.jsonl）
- 注：本 subagent 环境的 browser-use 插件不可用（"Browser is not available in subagent"），沿用 QA-B 同款 node+Playwright 驱动，黑盒纪律不变（只 GUI 操作 + 只读观察）。

## PASS 项（边测边记）

| # | 项 | 结论与证据 |
|---|---|---|
| 1 | 登录页错误密码报错 | admin + 错误密码 → 表单内红字 alert「用户名或密码错误」，无跳转、无 console 错。a03 |
| 2 | 登录成功跳转 | admin/admin → 直达工作台，头部「核心已连接」，退出按钮显示「退出 (admin · root)」。a04 |
| 3 | 项目注册（work） | 添加项目 → 目录树钉在 `target/qa-r13-sandbox`（agents-skills/claude-sessions/downloads/work），点 work 自动填路径（逐字填充动画）→ 提交 → 树节点「work local」+「该项目暂无会话」。a05/a06/a07 |
| 4 | 新建会话对话框 | 「在 work 中新建会话」：Agent 下拉（claude）+ provider 缺省提示（「尚未配置 provider 模型——仍可创建会话…」）+ 权限模式（Ask · 逐次询问）+ 创建/取消。a08 |
| 5 | 创建会话 toast | 「已在「work」创建新会话。」瞬时条目验；树计数 work 1、历史 1。a09 |
| 6 | hello 回合 + 自动命名 | hello → hello world，自动改名「未命名会话」→「hello」，Token in 100 · out 10。a11 |
| 7 | Auto 档切换回执 | combobox 切 Auto · 自动执行 → 系统卡「权限模式已切换：自动执行」+ 头部徽章「自动执行」。a13 |
| 8 | **Auto 档独立复测（QA-B 阻断项）** | Auto 档发 `perm`：无审批卡，过程 chip「过程 Bash · rm -rf / 1」→「✓ Bash ✓ 已执行」→ 正文「✓ Bash perm done」，**无「perm turn finished」收尾正文**——与 round10/11 核实的桩 bypassPermissions 设计完全一致，token 300/30。非缺陷。a14/a15 |
| 9 | Allow 档切换回执 | 系统卡「权限模式已切换：放行」+ 徽章「放行」。a16 |
| 10 | parallel 双卡组合裁决（QA-B 阻断项补测） | Ask 档 `parallel` → 两组独立卡（Bash echo first claude:tc-par-1-2 + Read /tmp/fake-parallel.txt claude:tc-par-2-1）各带 仅允许一次/本会话内允许/拒绝 + 上抛（禁用态占位）；第一卡批准→「✓ Bash 已执行 + Bash ok」，第二卡拒绝→「✗ Read 已拒绝 + denied by fake」，环后正文「parallel tools finished」，token 1500/150。a19/a20/a21/a22 |
| 11 | crash 呈现与恢复 | `crash` → 正文「boom」+ 红卡「! 错误 agent process exited or hung (watchdog)」；随后发消息正常回复（hello world），会话可用。a23/a24/a25 |
| 12 | F5 历史保持 | 聚焦会话 F5：URL 保持深链、消息/thinking/错误卡/token（1600·160）全部保持。a26 |
| 13 | 权限模式 F5 保持 | F5 后头部徽章「逐次询问」+ combobox 均 Ask 保持（Auto→Allow→Ask 一路切换后刷新亦保持）。a26 |
| 14 | R12-B-2 回合终点通知（正向） | 浏览器驻留 /sessions 历史页，API 向 native 会话发消息回合完成 → 页面顶部弹 info 瞬时条「会话「api hello native」的回合已完成。」；历史卡片同步 Done。a31 |
| 15 | R12-B-2 负向语义 | 聚焦该会话时 API 触发回合完成 → 不弹通知（既定语义）。a32 |
| 16 | 设置→模型：建 provider | ＋新建（自定义）表单（名称/三协议 Base URL 分字段/API key/默认模型/重命名映射）；高级折叠区展开后填 Anthropic URL=http://127.0.0.1:8791 → fake13 行「自定义 已配 key」+ ★/✎/🗑。a35–a38 |
| 17 | 别名：provider 下拉可选 | 别名表单「目标 provider」下拉出现 fake13 可选；deep13→fake13 保存成功。a42/a43 |
| 18 | Esc 两级关闭（round11 不回归） | Esc 先关嵌套表单、再关设置弹窗（渐进关闭）。a39/a40 |
| 19 | 删除 provider 确认与生效 | 确认弹窗「删除 provider fake13？这会把它从 router 配置中移除。」取消/红色删除；确认后列表回空态。a46/a47 |
| 20 | Agents 目录增改删 | ＋新建 claude-think（path=fake-claude-qa13.exe，**args 表单支持**（启动参数·空格分隔，placeholder 即 `--scenario thinking`））→ 状态「已创建（免重启，创建会话下拉立即可选）」；下拉立现；改名生效；删除确认文案明确（「已创建的会话不受影响…将立即从创建会话下拉中消失」）；删除后下拉只剩 claude/Native Kernel。a48–a58 |
| 21 | claude-think 会话 thinking 回合 | 新 agent 会话回合约 2s：两个「过程 thinking 1」chip + thought out loud + and the answer，token 100/10。a53 |
| 22 | 技能页列出/刷新/删除 | skill-alpha/beta（QA 预置 A/B）+ 刷新/同步按钮 + 仓内计数；刷新无回归；删除确认文案精确（「从技能仓删除 skill-alpha？各 agent 技能目录里的副本保持不动——它们会在下次点「同步」时被清理。」）；删除后 alpha 消失 beta 保留。a59–a62 |
| 23 | usage 页两种粒度 + 刻度完整 | 按天（近 14 天，右缘 2026-10-04 完整）与按小时（今天 00:00–23:00，右缘 23:00 完整）切换正常；维度 总量/输入/输出/缓存；round11 右缘截断修复保持。a63/a64 |
| 24 | 主题切换与持久化 | 外观 tab 三态（跟随系统/深色/浅色）；切深色立即生效；F5 后保持。a66–a69 |
| 25 | SPA 导航横切 | 侧栏 用量/首页/历史 点击后 `performance` navigation entries 恒为 1（无整页刷新、无闪白）；URL 随路由变化。a70 |
| 26 | 项目深入：第二项目/菜单/移除/切换 | 注册 downloads（目录树选择）→ 项目菜单含 重命名/移除项目/上移/下移(末位禁用)；移除确认「此操作只解除注册，可重新添加。」；移除后树中消失；多项目下点项目行切换主面板，跨项目聚焦时给出解释性空态。a71–a75 |
| 27 | 用户管理闭环（root） | 建 qa-live（member 默认）→ 改角色 admin（「角色已改为 admin」）→ 禁用（已禁用；登录被拒）→ 重新启用 → 登录成功（头部 qa-live · admin）→ 删除（「用户 qa-live 已删除」，列表回 admin/member/viewer）；admin 自身行 role/禁用/删除全部禁用（防自锁）。a76–a93/a122/a123 |
| 28 | 禁用用户登录被拒 | qa-live 禁用期间登录 → 「用户名或密码错误」（通用文案，不泄露禁用态）。a83/a89 |
| 29 | admin 与 root 差异面 | admin 角色用户：设置无「用户」tab、无「服务」tab、技能页无「同步」；其余（模型/别名/Agent/技能增删）可见。root 全量。a93（aria 证据）/a95 对照 |
| 30 | member 横切 | 登录 OK；能 GUI 建会话、发消息（回合应答+turn summary）；设置无 用户/服务 tab；技能页无删除无同步、有刷新（**R12-A-1 保持**）；API 侧：POST /api/sessions 201，POST /api/providers 403 类型化。a95/a96/a105/a106 |
| 31 | viewer 横切 | 登录 OK；新建会话入口隐藏；composer 可输入但发送被拒——红色类型化错误条「权限不足：viewer 角色无权执行该操作」，文本留在 composer、无乐观气泡；技能页无删除无同步；添加项目对话框可开但提交 403 不入树；API 直发 POST /api/sessions、DELETE /api/skills/*、POST /api/skills/sync 均 HTTP 403 + 类型化 JSON。a97–a104 |
| 32 | native 渠道三连（环境首次真可用） | 经 provider fake13（anthropic→127.0.0.1:8791）挂模型目录后，新建会话对话框出现 Provider/模型下拉：<br>• **test/thinking**：「模型已切换：test → test/thinking」系统卡 + 「过程 thinking 11」chip + 正文 + turn summary（a113/a114）<br>• **test/tool-use**：tool_use 声明（mkdir -p .sebas-probe）→ 审批卡（含上抛）→ 批准 → 策略 allowed_once → tool_result（Windows 平台报「unix only」，桩工具执行器的平台限制，非 WebUI 缺陷）→ 「tool loop complete」turn summary 2 calls/1 tools（a115–a117）<br>• **test/long**：流式中「停止回复」在位 → 点击 → 「回合已取消（操作者停止了本次回复）。」+ turn summary，跟发回合正常（a118–a121） |
| 33 | 登出 → 登录页零 console 错误 | 登出后登录页无任何 pageerror/console.error（round11 修复保持；台账中同期 401 为禁用用户登录测试故意触发）。a82 + console-a.jsonl |
| 34 | provider 模型目录支撑会话模型选择 | provider 表单「模型」区可增多条模型 id（含 ×/+/vision/audio/video 标签）；保存后 provider 行显示 code 芯片，新建会话对话框模型下拉即来自该目录。a110–a113 |

## 缺陷复核（QA-B 移交）

### B-2 复核（复采 1 次，共 2 例）：thinking 展开面板留白**稳定存在**
- 复采：claude-think（settings 新建的 thinking 场景 agent）会话展开「过程 thinking 1」→ 虚线面板内 💭 thinking 标签 → 大段空带 → hmm → 空带 → thought out loud（a54），与 QA-B b48 形态一致。维持 P3 打磨项，非偶发。

### 观察-1（口径记录，不计缺陷）：usage 页只见 router（native/test）流量，ACP 会话的 usage 不上页
- 现象：usage 页 4 次请求全部来自 native `test` 模型（usage 全零为该模型设计），折线图仅 test 一列；而 ACP（claude）会话每回合约 in 100 · out 10，从未出现在 usage 页。
- 初判：usage.db 归 router（架构上只记经 router 的流量），ACP 直连不经 router——「用量页=router 用量」口径下这是正确行为；若产品预期「用量页=全部 AI 用量」则是覆盖缺口。按口径存疑记录，P3 以下，建议与产品确认口径。a63。

### 观察-2（P3 以下打磨记录）：usage 折线图 y 轴刻度重复
- 现象：y 轴自上而下 1/1/1/0/0（a63/a64），刻度值重复出现，读感像渲染瑕疵（量级 <1 时的取整行为）。不影响数据解读。

### B-1 复核（2/2 采样确认，维持 P3 定性）：Allow·放行 档 `perm` 不出审批卡，形态与 Auto 档完全一致
- 采样1：切放行（回执正常落卡）→ 发 `perm` → 无卡直接「✓ Bash ✓ 已执行 → perm done」，无收尾正文（a16/a17）。
- 采样2：同会话再发 `perm` → 完全相同形态，token 累计 1000/100（a18）。
- **QA-A 复核定性**：维持缺陷，P3。BRIEF 口径明确「ask/edit/allow 档才出审批卡」，但放行档实测与 bypassPermissions 的 Auto 档无差别——要么会话权限模式→ACP 的映射把「放行」译成了 bypassPermissions（实现 bug），要么产品语义实为「放行=不再问」而验收口径过时（口径问题）。UI 本身无渲染缺陷；两说都指向「需要产品/桩口径裁决」，故维持 P3 不升级。100% 可复现（2/2），非偶发。

## 在册项验证（round12 遗留）

| 项 | 结论 |
|---|---|
| R12-B-1（crash 后 token 累计回退） | **PASS（已修，未复现回退）**：crash 前 1500/150 → crash 错误卡（无记账）→ 恢复回合后 1600/160，单调续增无回退。a23–a25 |
| R12-B-2（历史页驻留 + 他会话回合完成 → info 瞬时条） | **PASS（已修）**：正向弹出「会话「api hello native」的回合已完成。」（a31）；聚焦会话不弹（a32，既定语义）。 |
| O-2（crash 恢复后「模型已切换：default → fake」假回执） | **本轮未复现**：crash→恢复全流程未见任何「模型已切换」卡（恢复前后快照均无）。在册项在当前形态下未再出现，仅记录。 |
| O-5（死会话深链首载集中 404） | **形态良好**：死 key 深链首载仅 1 次 404（GET /api/sessions/web%00web-DEAD-0000-999），静置 12s + 交互后增量 0，无持续刷屏；主区呈现有界空态「会话不可得——聚焦的会话加载失败。」侧栏正常。a27/a28 |
| N-1（工作台直建会话不进 session_map） | 未专门复测（需重启 core，agent 无权操作）；本轮 GUI 会话与 API 会话均存活于历史页，无新形态。 |

## console pageerror 台账（F.19，终稿）

- **pageerror：0**（全程无未捕获异常）
- console.error：14 条，全部为浏览器「Failed to load resource」镜像，且全部对应**测试故意触发**的预期 4xx：401×4（错误密码测试 + 禁用用户登录测试）、404×1（死链测试点）、403×9（RBAC 写拒绝测试点）。应用自身零 console 错误。
- HTTP ≥400：14 条，与上同源，无任何非预期 4xx/5xx。
- 明细：verification/driver-a/console-a.jsonl（50 行，含 22 条驱动脚本自身定位错误，非页面产生）。

## 覆盖对照（任务清单 20 项）

| # | 任务 | 状态 |
|---|---|---|
| 1 | 登录页形态 | PASS（PASS-1/2） |
| 2 | RBAC 三角色横切 | PASS（PASS-29/30/31） |
| 3 | 用户管理闭环 | PASS（PASS-27/28） |
| 4 | 设置→模型/providers | PASS（PASS-16–19） |
| 5 | Agents 目录 | PASS（PASS-20/21） |
| 6 | 技能页 | PASS（PASS-22） |
| 7 | usage 页 | PASS（PASS-23；口径观察-1、打磨观察-2） |
| 8 | 主题 + SPA | PASS（PASS-24/25） |
| 9 | 项目深入 | PASS（PASS-26） |
| 10 | auto 档独立复测 | PASS（PASS-8） |
| 11 | B-1 复核×2 | 完成（2/2 确认，维持 P3） |
| 12 | 权限模式 F5 保持 | PASS（PASS-13） |
| 13 | parallel 双卡 | PASS（PASS-10） |
| 14 | crash + R12-B-1 + O-2 | PASS / PASS / O-2 未复现 |
| 15 | F5 历史保持 | PASS（PASS-12） |
| 16 | R12-B-2 | PASS（正负两向，PASS-14/15） |
| 17 | 深链 + 死链 404 | PASS（F5 直达 + O-5 形态良好） |
| 18 | 登出 console | PASS（PASS-33） |
| 19 | native 三连 | PASS（PASS-32） |
| 20 | B-2 复采 | 完成（第 2 例确认，维持 P3） |
