# fix-webui-qa-round13 Design

## Context

round13 QA（`verification/REPORT.md`）五个收口点，全部落在前端呈现与文档口径层，
无后端/wire 变更。QA 证据：findings-a.md（A 簇）、findings-b.md（B 簇）。

## Goals / Non-Goals

- Goals：B-2 留白收敛、usage 刻度去重、native 提示如实、usage 口径钉死（spec + 页面
  说明行）、AGENTS.md 过时句修正
- Non-Goals：见 proposal（round12 尾巴、N-1、cron 合并、allow 档行为）

## 关键决策

### D1：B-1 改判非缺陷（验收口径修正，非代码）

证据链：`sebas-webui/frontend/src/views/mode-vocabulary.ts` 注释明载 fix-webui-qa-round2
2.3（M-C6）裁决——`allow` 与 `auto` 在门控行为上**同一档**（同映射 bypass tier，
permission-flow spec 明文），round7 4.1 又把两条 description 改为互相点名等价
（「全部放行、留审计（与 Auto/Allow 等价）」）。QA-B/A 观测的「allow 档 `perm` 无
审批卡直接执行且无环后正文」正是该档的既定形态。

- 收口动作：修 AGENTS.md 一句（「审批路径（ask/edit/allow 档）才有环后正文」→
  「审批路径（ask/edit 档）才有环后正文；allow 与 auto 同为放行档，无卡无环后正文」）+
  更正本轮 QA 记录（BRIEF/REPORT，主 agent 已做/随收口做）
- 被否备选：改映射让 allow 出审批卡——与 round2 既有裁决和前端词汇直接冲突，否

### D2：B-2 thinking 面板留白——样式收敛，不重构

展开面板的「💭 thinking」标签与内容间空带来自最小高度/间距撑高（单采样+复采稳定）。
修法限定在展开态样式（min-height/gap/margin），不动转录块模型与折叠态形态；前端单测
钉展开态高度语义（如内容行数与面板高度关系），不引入视觉回归快照新依赖。

### D3：观察-2 usage 刻度去重——刻度生成函数去重，不换图表库

y 轴刻度由前端生成（小值域下产生等值相邻标签）。修法：刻度计算处对相邻等值标签去重
（等值只保留一个），轴整体仍单调；不更换渲染库、不动数据聚合。单测以小值域序列
（如 max=1 与 max=5）断言相邻互异。

### D4：native 提示如实——文案改述 env 条件，不改启用机制

`new-session-dialog.ts` 的 native 禁用提示改为如实陈述：需 core 进程以
`SEBAS_AGENT_PROVIDER_API_KEY` 或 `SEBAS_AGENT_ROUTER_URL` 启动（WebUI 内无法满足）；
移除「到「设置 → 模型」配置」引导。被否备选：让设置页可启用 native——env 是部署面
条件，设置页无权点亮，round12/QA 两轮实证配置无效；否。服务端 `/api/summary` 的
cause 文案已如实（QA-B 取证），不动。

### D5：观察-1 usage 口径——spec 钉死 + 一行数据源说明

usage 页定位 = router 用量记录呈现（`usage.db` 单写者归 router，架构不动）。spec 增
刻度唯一性 + 数据源口径两条（delta 见 specs/usage-statistics）；页面加一行口径说明
（「统计来自 router 流量；ACP 直连会话的 token 计数见会话头部」，措辞实现时定）。
被否备选：usage 页并入 ACP 用量——破坏 usage.db 单写者归属（ACP 写入者无聚合管道），
否。

## 假设（次要细节）

- 数据源说明行的具体措辞与安放位置（摘要区下方或图表上方）实现时定，单测钉「存在且
  含 router 字样」
- B-2 修复后面板在长内容（>1000 字符）下的滚动行为不变
