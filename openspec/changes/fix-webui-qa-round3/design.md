# Design — fix-webui-qa-round3

## 逐项决策与落点

| 编号 | 缺陷 | 落点与做法 |
|---|---|---|
| D1 (P2) | thinking 折叠误标：正文块挂「PROCESS thinking」芯片、thinking 缺省形态与正文不可区分 | `transcript-view.ts` 的 run 分派：text 条目永不入过程 run；仅 thinking/tool 条目构成 process run。芯片/折叠行只渲染在 process run 与二级折叠。thinking 二级折叠标题带 thinking 标识。单测：thinking 场景快照断言「正文段零过程芯片 + thinking 二级折叠标题」 |
| D2 (P2) | mode 下拉选择一次后鼠标无法再展开（箭头翻转但列表不渲染；键盘可开） | mode 下拉组件（composer 工具条与创建对话框共用）：展开态布尔与实际渲染一致性修复（疑似 blur/Outside-click 处理器在选项选中后未复位）。单测：选择→再点击→列表可见 |
| D3 (P2) | `/sessions` 栅格 1280 裁剪（scrollWidth 1336 > 1280，overflow-x hidden） | 会话列表栅格改可换行（auto-fill/minmax）或横向可滚动二者取一（实现择优）；断言 scrollWidth ≤ clientWidth。单测：1280 视口布局快照 |
| D4 (P3) | usage 刷新按钮溢出 24px | 控件行 flex-wrap / min-width 收纳；1280 视口快照断言 |
| D5 (P3) | provider 探测错误长文本右缘裁切 | 错误反馈区 white-space 换行或可展开；断言长错误完整可读 |
| D6 (P3) | 重命名后详情头部标题不同步（侧栏已新、头部仍旧） | 改名成功回调同步焦点会话头部标题（共享同一标题源，禁止两处独立缓存）；单测：改名后头部与侧栏文本一致 |
| D7 (P3) | History 显示归档前旧标题 | 归档快照的 label 取归档时刻现用标签（后端 archive 条目 label 或前端列表取 label 字段一致化——实现时查 archive 条目 wire，若 label 在归档时已落库则修列表取数）；单测：改名→归档→History 显示新名 |
| D8 (P3) | usage 统计卡不随窗口过滤 | 统计卡取数与图表同一响应/窗口参数联动；单测：切粒度后卡片数字重算 |
| D9 (P3) | 项目重排序无 UI | `project-rail.ts` 项目菜单补上移/下移（或拖拽，实现择优），调既有 `POST /api/projects/reorder`；持久化断言 |
| D10 (P3) | 会话选择不写 URL | router pushState `/sessions/{key}`（focus 时），清焦回落 `/`；启动时按 URL 恢复焦点（未知键降级不白屏）；与既有 session-deep-link 路由复用。单测：选择→地址更新；刷新→焦点恢复 |
| D11 (P3) | 登录前 `/ws` 401 重试噪音 | ws 客户端：未认证态（401 拒绝后）静默抑制或长退避（≥30s），认证成功后恢复。单测：未认证页面 console 无重复 401 |

## 关键决策

- **D10 与「rail 标记不取自 URL」的既有语义并存**：URL 只是地址栏投影 + 刷新自持，焦点指针仍由交互派生——两者不冲突（deep-link 访问本身就是聚焦路径之一）。
- **D7 修前后端择一**：先查 `/api/archive` 条目 wire 是否已带归档时刻 label；带则纯前端取数修正，不带则后端 archive 快照补齐（改动最小者）。
- **D3 布局方案**（换行 vs 横向滚动）：倾向换行（minmax 卡片），信息密度不损失；若卡片最小宽约束冲突再退横向滚动。spec 两条都允许，实现自选并在验收截图注明。
- 全部为前端行为（除 D7 可能触后端 archive label），无 wire 变更。

## 被否备选

- 工作台头部加回合状态文字徽章（W1 观察）——否：停止按钮 + rail 圆点已表达运行态，加徽章与 `/sessions` 卡片职责重复；维持现状，不立项。
- usage 口径改后端聚合计数——否：端点已按窗口聚合，纯前端联动即可，不动接口。

## 假设

- 1280×720 是本期布局验收基线（QA 全程视口）；更窄视口（移动端）不在本期范围。
- D2 若根因在上游 web-component（wa-select 类），修复落在我们的包装层，不 vendor 改造。
