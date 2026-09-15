# workbench-natural-conversation-flow — Tasks

## 1. 分组层

- [x] 1.1 实现交替 run 切分纯函数：agent turn 条目序列 → `TextRun | ProcessRun` 交替序列（连续同类合并、error 不入 run），替换单 ProcessBlock 归并；单测覆盖「正文-过程-正文交错」「连续同类合并」「error 独立」
- [x] 1.2 ProcessRun 稳定 id（首条目 position）与展开状态 Map；单测覆盖「重分组后 id 稳定」「展开状态跨重渲染恢复」

## 2. 渲染层

- [x] 2.1 一 run 一折：process-run 渲染为收起折叠（内嵌既有二级折叠与结构化标题）；折叠行摘要实时渲染进行中工具 title + 条目计数；单测覆盖「默认收起」「摘要随增量更新」「展开后新条目就地追加且不收起」
- [x] 2.2 去卡片化样式：agent turn 撤 `.bubble` 卡片壳、作者标签裸排小字行；`.is-user` 收敛轻底色块（去边框阴影）；单测断言 agent 侧无卡片类、用户侧保留 tinted 类
- [x] 2.3 「已收到」receipt 徽标随新消息形态回归验证（单测不回归）

## 3. 端到端验证

- [x] 3.1 fake-claude 沙箱完整 turn：按 AGENTS.md 手工食谱起双进程沙箱（`--scenario thinking/bash` agent），真机浏览器 DOM 断言留档：工具回合渲染为单折叠（稳定 id=首条目 position、默认收起、摘要=process+结构化 title「Bash · echo hi」+计数）、展开见二级折叠（结构化 title、默认收起）；多回合文本流裸排（作者标签+正文、无 .bubble 卡片）、用户侧 tinted 保留、跨回合 [折, 文, 折, 文] 时间序正确。IAB 截图通道失效，视觉证据由 3.3 Playwright 覆盖；thinking 增量在现有 claude 驱动管线不落盘属既有后端行为（单测已覆盖 thinking run 渲染），fake-claude 场景限制单回合内 text→tool→text 交错由单测覆盖
- [x] 3.2 未读缝/贴底自动滚动/角标不闪回归：`invoke testsuite-acceptance` 9/9 旅程全绿
- [x] 3.3 `invoke testsuite-webui-server`（Playwright 面）transcript 相关用例更新并通过：conversation.spec 过程折叠用例更新到新摘要 DOM（label/running/fold-count 三段）后通过；全量套件 62 过/9 挂——对照基线（stash 改动重跑同批）7 个同样挂（分支既有）+ session-roundtrip 负载 flake（pre-A 基线全量亦复现挂，solo 稳过；已立案跟进），本 change 相关面全绿
