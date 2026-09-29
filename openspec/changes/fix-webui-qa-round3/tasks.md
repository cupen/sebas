# Tasks — fix-webui-qa-round3

## 1. P2 行为修复

- [ ] 1.1 D1 thinking 折叠成员与可区分性：run 分派修至「text 条目永不入过程 run」；thinking 二级折叠标题标明 thinking；正文段零过程芯片。验证：transcript-view 单测（thinking 场景快照）+ GUI 手测 thinking agent 回合截图。（QA W2 #8）
- [ ] 1.2 D2 mode 下拉重复展开：选中后指针再次点击可重新展开，箭头与显隐一致（composer 工具条 + 创建对话框两处）。验证：前端单测 + GUI 手测。（QA W2 缺陷 3）
- [ ] 1.3 D3 `/sessions` 栅格 1280 收纳：换行或横向滚动，卡片操作可达。验证：1280 视口布局断言（scrollWidth ≤ clientWidth 或存在滚动容器）+ GUI 截图。（QA W1 D1）
- [ ] 1.4 D8 usage 统计卡随窗口联动：卡片与图表同窗口重算。验证：前端单测 + GUI 手测切粒度前后截图。（QA W3 观察 2）

## 2. P3 呈现与功能补全

- [ ] 2.1 D4 usage 控件收纳（1280 无溢出）。（QA W3 D1）
- [ ] 2.2 D5 provider 探测错误文本完整可读（换行/展开）。（QA W3 D2）
- [ ] 2.3 D6 重命名后详情头部即时同步；侧栏与头部同一标题源。（QA W3 D3）
- [ ] 2.4 D7 History 条目标签取归档时刻现用标签（先查 archive wire 是否带 label，按 design 择前端取数或后端快照补齐）。（QA W1 D3）
- [ ] 2.5 D9 项目栏重排序入口（上移/下移或拖拽），走既有 `/api/projects/reorder`，持久化。（QA W1 D2）
- [ ] 2.6 D10 会话选择写 URL（pushState `/sessions/{key}`，清焦回落 `/`，刷新自持焦点，未知键降级不白屏）。（QA W3 D4）
- [ ] 2.7 D11 登录前 `/ws` 静默/长退避，消除 401 console 噪音。（QA W4 D4）

## 3. 回归与验收

- [ ] 3.1 全量 `cargo test` + 前端单测 + 既有 Playwright 套件（conversation / parallel-permissions / unread-badge / settings 等）不回归。
- [ ] 3.2 GUI 逐项手测（1280×720 沙箱）：上表 11 项逐项复核截图留档（对照 qa-evidence/w1、w3 的缺陷截图）。
- [ ] 3.3 thinking / drip / flood / perm / parallel 五个 fake 场景各跑一回合，确认核心链路无回归。
