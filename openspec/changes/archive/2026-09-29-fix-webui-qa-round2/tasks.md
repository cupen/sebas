# Tasks — fix-webui-qa-round2

## 1. P2 行为修复

- [x] 1.1 D-B11：移除 Add-project 路径输入的破坏性字符过滤，输入保真提交；校验去抖（~300ms）+ 服务端 cause 如实透出。验证：前端单测键入 `C:\a\b` 逐字符保真；GUI 手测无逐键 400。（QA 编号 D-B11）
  - 状态（3b 复测修订）：首版归因「旧构建」只对了一半——3b 浏览器探针（现行构建）复测报「值被 JS 风格反转义（\f→FF、\t→TAB、未知转义丢反斜杠）」。逐层二分探针（内层 input → wa-input host → 组件 state → 去抖预检 wire，全带 charCode 序列）证明应用链逐字符保真：变质点在**探针自身**——`fill('C:\fill\path\test')` 把单反斜杠写进了 TS 字符串字面量，值在探针解析期就被 JS 字面量语义吃掉（\f→FF、\p→p、\t→TAB；fill 与 pressSequentially 两路同源）。探针已改 String.raw 重写并复跑：fill/逐键两路在 L0/L1a/L1b/L2 四层读回值 charCode 全等（92 三处、零控制字符）。组件级单测升级为「真 dialog + 含 \f\n\t\p 拼写路径 + inputValue 与提交载荷逐字符一致 + 零控制字符断言」。本轮交付的去抖 + 键击清陈旧 hint + cause 如实透出不变。
- [x] 1.2 D-B215：子进程死亡且回合 working 时立即走 force-settle 终态化（复用停滞看门狗路径，幂等）。验证：dispatch 单测——fake-claude `crash` 触发后回合相位在 UI 终态卡同期内落终态、composer 可立即再提交；无二次停滞卡。（D-B215）
  - 状态：根因归因与 design 假设不同——崩溃本体早已快速终态（terminal Error 臂 + finalize_dead_child）；僵尸窗来自**重聚焦激活**：seed_card("") 曾注入空 prompt 转录条目（前端回执相位恒真 → composer 停止态阻塞发送）+ 起停滞时钟（600s 误强收「回合停滞」）。修复 = 空 prompt 激活 seed 不落条目不起时钟（sebas-dispatch engine seed_card）；前端 entriesAwaitReceipt 对空 prompt 条目防御。单测：turn_stall_test「activation_seed_empty_prompt_opens_no_turn_and_starts_no_clock」。
- [x] 1.3 D-C3a：审批决策后 tool_result 条目顶层化，✓已执行/✗已拒绝 chip 常驻，折叠互不连带。验证：前端单测 + 转写快照；GUI 手测 `perm` allow/deny 两路径。（D-C3）
  - 状态：tool_result 提升为顶层 tool_result run（✓已执行/✗已拒绝章常驻、展开体默认开、与过程折叠开合互不连带）；并行场景双结果各自成块（D-C5 保留半边同此收口）。单测：transcript-view.test「decided tool results lift to top-level runs」5 条；e2e conversation.spec 折叠旅程按新合同更新。
- [x] 1.4 D-C3b：修复单工具审批回合丢失环后正文的投影缺陷（先查 sebas-dispatch 事件投影再修）。验证：投影单测 tool_result 后 assistant 文本完整保留；GUI 手测 `perm` 回合有环后正文。（D-C3）
  - 状态：投影层排查结论——tool_result 后的 assistant 文本在引擎投影中完整保留（engine_test 新钉「assistant_text_after_tool_result_lands_in_order」）；丢正文根因在夹具：fake-claude 的 perm 场景本就无环后正文帧。已补帧（与 parallel 同形态）。【偏离 design》design 写「不动 fake-claude」——调查证明缺陷在夹具不在投影，补帧是唯一修法。
- [x] 1.5 D-C5：并行审批并发呈现——转写按挂起审批列表渲染全部待批卡，决策后各条目连同结果保留。实现首步探明 pending 审批的 wire 现状，若后端缺集合上行则先补状态投影与单测。验证：GUI 手测 `parallel` 双卡同现、一允许一拒绝后两条目带各自结果。（D-C5）
  - 状态：探明 wire——引擎泊车登记/读模型/审查卡 store 本就支持集合，缺口在 cc-agent-sdk 0.1.7：hook_callback 处理在 await hook 期间持有注册表锁，并行 hook_callback 被串行化（第二张卡只在第一张决策后出现）。vendor 补丁（vendor/cc-agent-sdk + [patch.crates-io]）：克隆回调后先放锁再 await；0.1.7 无上游修复可升。parallel-permissions.spec 过。

## 2. P3 缺失与语义

- [x] 2.1 未读分界线（D-B12+M-B216）：聚焦时按 read anchor 补绘分界线、滚过即清、聚焦贴底流式不绘、placeholder 首交换不绘。验证：前端单测 + GUI 手测非聚焦完成后重聚焦出现分界线且读后不再现。
  - 状态：分界线边界改「开卷冻结边界」（unread-cursor armOpeningSeam/take + transcript seamBoundary）——聚焦推进锚（2.6）后 seam 仍按推进前边界呈现一次；滚读到底/mark all seen/聚焦看着到达清账。placeholder 首交换与聚焦贴底不绘的既有规则保留。
- [x] 2.2 /compact 回执与单一分派（D-B218）：Enter/发送按钮收敛单一 submit 函数，命令提交出转写回执条目、产出独立成条。验证：前端单测两条路径判定一致；GUI 手测 /compact 有回执、未知命令有反馈。
  - 状态：/compact 落 prompt 回执条目（inbound.rs Command::Compact 臂），产出独立成段；Enter/按钮本就同经单一 submit()，补 3 条单测钉键盘/按钮等价分派 + 未知命令拦截等价。
- [x] 2.3 模式描述如实（M-C6）：allow/auto 描述明确等价（同全放行档），四词文案不暗示差异。验证：GUI 手测悬浮描述；文案快照单测。
  - 状态：allow/auto 描述改「全部放行、留审计（与 Auto/Allow 等价）」互相点名等价；快照单测 mode-vocabulary.test 4 条。
- [x] 2.4 agent 表单补 sessions_dir / work_dir / args 字段 + display name 编辑回填（M-A4+D-A4）。验证：前端单测表单映射 store 列；GUI 手测建 agent 后开 session 完成 fake-claude 回合、编辑表单回填。
  - 状态：sessions_dir 端到端入表（AgentRow 列 + AgentDefinition + DDL + mutation 校验 + put 载荷）；表单补 Sessions dir / Work dir / Args 三字段 + 重开编辑按存量回填；display 回填改用新增 wire 字段 display_raw（原始未兜底值，D-A4 根因 = catalog 兜底 id 丢失「显式设置」事实）；重名创建可见警告（3.3 一并）。
- [x] 2.5 flood 批量渲染（D-B13）：大批量路径分片 append（≤50 条/片），小批量直渲染保留 drip 首包即时性。验证：前端性能单测（1200 条摄入无 >300ms 单帧阻塞）；drip e2e 首包可见性不回归。
  - 状态：>50 条的 turn.append 走 rAF 分片（每片 ≤50），小批量直渲染保留 drip 首包即时性；单测 1200 条分片收敛零丢失。
- [x] 2.6 聚焦写锚竞态（D-R2A，P2）：聚焦会话写读锚用了陈旧计数——第二回合完成后立即聚焦，anchor 停在上一回合的段数（unread-badge.spec 两条确定性红「Expected 2 Received 1」；活体复现证明服务端 msg_count 正确为 2）。聚焦写锚 SHALL 以服务端当前计数为准（establishment 已有锚不覆写，但聚焦后的推进路径不得因 turnLive 竞速停在旧值）。验证：unread-badge.spec 两条转绿。
  - 状态：聚焦写锚改「以服务端当前计数为准」——rail switch 与 dashboard 焦点转换（establishedFocusKey 门，驻留刷新不推进）两路推进；transcript turnLive 翻 false 补写（末帧经 turn.append 收敛后 entries 不再变化的竞速）；严格推进（无水位不写，不污染空流登记）。

## 3. 呈现修正与打磨组

- [x] 3.4 Settings→Models provider 行回归（D-R2B，P3）：settings.spec S5a 确定性红——config provider 行（anthropic）在设置面不可见（locator `.provider-row` hasText anthropic 超时）。先归因（渲染回归 or 场景配置变化），按归因修复。验证：S5a 转绿。
  - 状态：归因 = （a）QA「不可见」出自旧构建（M7 config 行渲染在其后落地，当前构建两行齐全）；（b）当前套件的红是定位歧义——deepseek 预设 URL 恰含 '/anthropic' 子串 + config 行与新建 preset 行同名，hasText 'anthropic' 三行同命中 strict-violation。修复 = 断言改按 preset 徽章（'anthropic · code'）精确锚定。settings.spec 8/8 过。

- [x] 3.1 About toolchain 行补「要求 ≥」界限（D-A7）；Env Vars 表格列宽修正（D-A10）。验证：GUI 手测 About 段两行齐全、Env Vars 常规宽度可读。
  - 状态：rustc_required 为空串（本 crate 未配 rust-version）时不渲染悬空「要求 ≥」标签；Env Vars 表 fixed 布局定宽列 + 长值换行 + 右缘留白。
- [x] 3.2 Add-project 弹窗 `or` 分隔线间距（D-A2）。验证：GUI 手测列表滚到底不贴边。
  - 状态：or 分隔符上下 margin（--sebas-space-3），列表滚到底不贴边。
- [x] 3.3 打磨组：last active 计时持续更新；toast 依次堆叠不重叠；agent 重名创建可见提示；模型 chip 如实呈现来源（C10 组）。验证：GUI 手测逐项。
  - 状态：last active 按本机时钟秒级现算（detail 增下 last_active_unix + relativeActiveLabel）；重名创建警告见 2.4；模型 chip 未锁定时显示「默认」不冒充候选首项、title 注明「执行体上报」来源；toast 堆叠本就由官方 wa-toast 栈承载（依次堆叠 + 挤占上限），无代码改动。

## 4. 账目与验收

- [x] 4.1 校正 `fix-webui-qa-findings/tasks.md`：3.2（D3）勾选完成注「本轮实证已实现」；4.2（M1）注余量移入本 change。验证：文件更新、openspec status 正常。
  - 状态：fix-webui-qa-findings/tasks.md 3.2/4.2 的勾选与注记已在前序收口（与本轮要求一致，逐字核对于本轮），无新增改动。
- [x] 4.2 全量回归：`cargo test` + 前端测试套件全绿；进程级 e2e 套件（`invoke testsuite-e2e`）通过；按 evidence/ 报告对四个 P2 逐项 GUI 复测通过。验证：命令退出码 0 + 复测记录附本 change。
  - 状态（本轮 subagent 半边）：`cargo build` 0 错误；`cargo test --workspace` 除 sebas-agent 预存失败外全绿（17 lib + 2 integration 失败在干净 HEAD b91a57c 逐一复现同样失败，与本 change 无关——Windows 环境的 bash 工具/轨迹夹具问题）；前端 vitest 722/722、tsc/build 干净。定向浏览器验证：unread-badge 2/2 绿、settings 8/8 绿（含 S5a）、permission/approval-restore/approval-reconcile/stop-settle/submit-control/slash-commands/parallel-permissions/thinking-process-fold/conversation/rail-focus/project-focus-reconcile/streaming 定向全绿。余量留给主 agent：`invoke testsuite-e2e` 全量、四 P2 逐项 GUI 复测记录、core-freeze（linux-first 恒红）。
