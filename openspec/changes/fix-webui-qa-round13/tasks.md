# fix-webui-qa-round13 Tasks

## 1. B-2：thinking 展开面板留白收敛

- [x] 1.1 定位展开态样式来源（min-height/gap/margin 之一），压缩「💭 thinking」
  标签与内容间空带；折叠态形态与长内容滚动行为不变
  （验证：前端单测断言展开态高度语义；截图对照 QA b48/a54 形态）
  > 状态：实际来源 = `.body` 的 `white-space: pre-wrap`（round11 4.1）经类名
  > 与继承进入展开面板——模板缩进换行与 markdown 块间换行逐个成为可见空行
  > （非 design 假设的 min-height/gap/margin，见汇报偏离项）。修法仍限定展开
  > 态样式：`.fold-body` / `.fold-body .body` 恢复 normal，pre 代码块与横滚
  > 语义不动。视觉对照留 6.1 GUI 抽查。
- [x] 1.2 单测翻新：thinking 呈现既有用例补展开态断言，全套绿

## 2. 观察-2：usage 图表 y 轴刻度去重

- [x] 2.1 刻度生成处对等值相邻标签去重（等值只保留一个，轴单调不变）
  （验证：单测以小值域序列（max=1、max=5）断言相邻互异，1/1/1/0/0 形态绝迹）
- [x] 2.2 usage 视图单测翻新 + day/hour 两粒度回归

## 3. native 禁用提示如实化

- [x] 3.1 `new-session-dialog` 的 native 禁用提示改为 core env 启用条件
  （提及 `SEBAS_AGENT_PROVIDER_API_KEY` 或 `SEBAS_AGENT_ROUTER_URL` 至少其一），
  移除「到「设置 → 模型」配置」引导；其它 agent 的「设置 → Agent」提示不动
  （验证：单测断言提示含 env 词、不含设置页引导；QA round13 b09/b18 场景复核）
- [x] 3.2 既有 new-session-dialog 单测翻新，全套绿

## 4. 观察-1：usage 页口径钉死

- [x] 4.1 usage 视图加一行数据源说明（含 router 字样，措辞按 design 假设）
  （验证：单测断言说明行存在且含「router」；纯 ACP 窗口仍呈现无数据空态）
- [x] 4.2 spec delta 对应场景（相邻刻度不重复、纯 ACP 空态）在单测层各有承载
  （验证：usage 视图单测覆盖两场景；specs/usage-statistics delta 与实现对读）

## 5. B-1 收口：AGENTS.md 过时句修正（无代码）

- [x] 5.1 AGENTS.md「审批路径（ask/edit/allow 档）才有环后正文」改为「审批路径
  （ask/edit 档）才有环后正文；allow 与 auto 同为放行档（round2 M-C6），无审批卡
  无环后正文」
  （验证：文本对读 mode-vocabulary.ts 注释一致；QA-BRIEF/REPORT 的 B-1 记录已
  改判标注）
  > 状态：AGENTS.md 已改、与 mode-vocabulary.ts（M-C6 裁决 + 互相点名等价）
  > 对读一致。QA-BRIEF/REPORT 的 B-1 改判标注已由主 agent 补齐（QA-BRIEF.md
  > 触发词节【round13 收口更正】+ REPORT.md 缺陷总账改判行），本任务全部收口。

## 6. 收口验证

- [x] 6.1 前端单测全绿（`pnpm test` / 既有 runner）；round13 沙箱 GUI 抽查销账：
  thinking 面板、usage 刻度与说明行、native 禁用提示三项逐一眼见为实（复用
  `target/qa-r13-sandbox/`，native env 已齐）
  （验证：命令输出 + 抽查截图归档 verification/shots/）
  > 状态（主 agent 3b/6.1 销账）：pnpm test 943 全绿、pnpm build 过；GUI 抽查三项
  > 眼见为实——`shots/r13_thinking_expanded.png`（B-2：标签与内容紧凑，b48 空带绝迹）、
  > `shots/r13_usage_source_note.png`（说明行一字不差 + y 轴 0/62/123/185/246 相邻互异）、
  > `shots/r13_native_hint.png`（无 env 一次性实例 9878：禁用项提示点名两个 env 变量、
  > 明说 WebUI 内无法配置、无设置页误导引导）。驱动脚本 driver-a/r13_check.mjs、
  > r13_think2.mjs、r13_hint.mjs。
- [x] 6.2 webui 浏览器旅程不回归：相关用例定向跑（testsuite-webui 9899 装配）
  （验证：定向套件绿）
  > 状态（3c review subagent 补跑）：首轮 12 过 1 红（US2 断言过时 + first-paint
  > 旧 native 文案断言），修正两处测试侧断言后复跑 **13/13 绿**；第二批
  > conversation/conversation-streaming/qa-round3 **16/16 绿**；另补
  > `agent_kinds_test.rs`（provider 配置不翻转 native 可用性）7/7 绿、
  > pnpm 943 复跑绿、openspec validate --strict valid。顺带收口 3c 发现的主
  > spec 矛盾：change 内新增 specs/agent-workbench delta（native cause 口径
  > 反转），proposal Capabilities 同步。
