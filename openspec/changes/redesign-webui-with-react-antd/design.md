## Context

See `proposal.md` — Why.

当前 `sebas-webui/frontend` 使用 Lit 3.3.3、Web Awesome 3.12.0、TypeScript、Vite 8 与 Vitest 4。实现与 colocated 测试各约 2.4 万行；`settings-modal`、`transcript-view`、`dashboard`、`project-rail`、`workbench-composer` 是最大的迁移风险点。Shadow DOM、Web Awesome 属性/事件、focus workaround 和 source-string 断言均不能直接搬入 React。

API client、协议类型、模型目录、通知与持久化辅助逻辑，以及转录的纯数据转换可复用；WebSocket 中 Lit ReactiveController 的适配需拆开。浏览器页面对象大量依赖 `sebas-*`、`wa-*` 及嵌套 Shadow DOM。Vite 的开发代理和 `build.rs → dist → rust-embed` 路径可保持。

版本基线于 2026-11-06 从 npm 官方 registry 查实：React / React DOM **19.3.1**、Ant Design **6.7.0**，AntD 声明支持 React/React DOM `>=18`。同期工具基线为 `@vitejs/plugin-react` 6.2.1、`@testing-library/react` 16.3.3、`@testing-library/user-event` 14.6.8、`@testing-library/jest-dom` 7.0.1；采用前须核对相互 peer range 并在 lockfile 固化。

proposal 中三项范围/交付策略仍是待评审假设；只有“采用 React + AntD”与“同步重做交互”已获用户明确确认。因此蓝图评审不仅是视觉门槛，也必须重新确认全量替换及一次发布假设。

## Goals / Non-Goals

**Goals:**

- 以单一 React 应用承接产品，API/WS 与 Rust 服务边界不变。
- 用可评审的交互蓝图指导 redesign，而不是默认 AntD 皮肤。
- 将共享数据与渲染生命周期拆开，保证流式更新的局部性与可恢复性。
- 用统一主题、语义测试和真实沙箱旅程证明替换完整。

**Non-Goals:**

- 不在此设计中引入 Redux、React Query、React Router、Tailwind、shadcn 或新图表库。
- 不保留长期运行的新旧 UI 开关，不增加后端兼容 API。
- 不把机械翻译 Lit 模板或旧 Shadow DOM 选择器视为迁移完成。

## Decisions

### D1: 使用 React + AntD 的克制技术组合

采用 `createRoot`、函数组件、Hooks、TypeScript TSX 与官方 `@vitejs/plugin-react`。通用控件、弹层、表单、菜单、树、通知及布局使用 AntD；自定义转录、代码、过程折叠和 SVG 用量图保持受控的领域组件。AntD icons 与既有本地 SVG 必须统一笔画和尺寸，装饰图标隐藏于可访问性树，独立图标按钮提供中文名称。

**理由**：满足用户指定技术栈并复用成熟交互基元，避免同时引入另一套 CSS/组件体系。

**替代**：AntD + Tailwind/shadcn 被否决——增加两套设计来源；全部自绘控件被否决——键盘、弹层及表单成本不必要地升高。

### D2: 蓝图先行，采用“导航轨 + 单工作区 + 会话内待办”

第一任务交付桌面/窄屏的可视化交互蓝图，而非 production UI。方向如下，具体间距、色值和文案以用户通过的稿件为准：

```
桌面
┌────────────────────┬──────────────────────────────────────┐
│ 品牌 / 新建项目     │ 当前项目 · 当前会话 · 连接状态       │
│ 项目与活动会话      ├──────────────────────────────────────┤
│ 跨项目会话 / 归档   │ 会话内待办（审批 / 队列 / 错误）     │
│                    │ 转录 / 未读缝 / 工具过程             │
│ 用量 / 设置 / 用户  │ 多行输入 · 模型与模式 · 发送         │
└────────────────────┴──────────────────────────────────────┘

窄屏：顶部当前会话与导航按钮 → 会话内待办 → 转录 → 输入；
导航为 Drawer，设置仍为无独立 URL 的工作区，审批不被 toast 替代。
```

设置由紧凑居中 Modal 改为带分区导航的宽工作区，但不恢复 `/settings` 路由。待办统一呈现当前会话的审批、队列、错误，不声称跨会话全局审批聚合——后者缺乏既有聚合读模型，且不是本 change 新增业务能力的范围。

视觉方向为中性色开发工具、紧凑栅格、系统字体、一个主强调色；不采纳设计检索结果中的营销 Hero、AI 紫粉渐变或外链字体。机器状态与“等待你处理”分别使用状态色和 signal 语义，保留文字/形状通道。

蓝图需覆盖所有关键页面及状态、1440/1024/768/375 宽度、深浅主题、键盘/减少动态效果，并记录用户评审结论。若评审推翻 proposal 中全量或一次发布假设，先修订工件再实施。

### D3: 保留路由和浏览器兼容语义，不引入路由库

将现有 route matcher、retired redirects、History API 导航封装为 React Hook/Context。`/sessions/:key` 的原始编码参数不得提前解码，仍在同一工作台聚焦，不生成独立详情视图；`/sessions` 保留跨项目列表，`/usage` 保留独立深链。

localStorage 的现有 key 与数据语义继续兼容，主题、rail 宽度、展开状态、上次模型组合、未读锚点等由独立 adapter 读写；布局改版需新增偏好时新增 key，不能改写旧值含义。

**替代**：React Router 被否决——当前仅少量 matcher 且有特殊编码约束，引库没有解决实际难题。

### D4: 以“外部数据适配层 + 领域状态 + 局部订阅”管理状态

不引入全局状态库。将 API/WS 从 Lit controller 解耦为框架无关的数据源，并用可订阅 store + `useSyncExternalStore` 向 React 暴露快照；按 auth/shell、project/session、conversation、settings 划分所有权。React Context 仅用于稳定的 service、身份和共享客户端，不承载逐流式帧变化的大对象。

每会话转录保留 cursor、generation、entry identity 和 stale-response guard。WS 只请求轻量列表刷新或聚焦会话的 `entries_after` 增量，绝不按帧重拉全量。事件处理沿用现有纯 reducer/转换函数；服务端真值、临时 UI 状态及表单草稿分别管理。

**替代**：Redux/Zustand/React Query 暂不采用——协议同步是领域逻辑而非普通服务端缓存，先保持适配层显式且依赖最小。

### D5: 流式转录按条目隔离更新，Markdown 结算保持现有路径

条目使用稳定 ID 作为 key；store 支持按会话/条目订阅，搭配 `memo`，避免任一 delta 重渲染整份历史。长文滴流期间保持纯文本 live tail，回合结算后才执行 `marked → DOMPurify → highlight.js`；React 中仅以隔离的 sanitized-html 组件承载消毒结果，禁止普通文本进入 `dangerouslySetInnerHTML`。

历史代码/折叠/工具结果组件不依赖父级大列表反复 render。窗口化只有在真实 profiling 证明必要时才引入，且需解决未读定位、展开状态、浏览器查找与滚动锚定，不能作为默认“性能优化”。持续滴流采用确定性 burst harness 测量；首轮以现网浏览器基线作对照，将“无全量请求/全转录重解析、输入/审批/滚动可完成、无明显布局跳动”作为硬门槛，具体 ms/长任务预算由蓝图与基线报告共同评审后记录。

### D6: 统一 AntD token 与应用语义 token

以 `ConfigProvider` 配置中文 locale、明暗 algorithm 和组件 token，同时将现有 `--sebas-*` 中仍有意义的 status/signal/notice/代码呈现语义映射为应用 CSS variables。页面和领域组件采用普通 CSS + CSS Modules，不再依赖 Shadow DOM；AntD 弹层 portal 与应用主题同源。

系统主题变化及显式明暗切换同步更新 `color-scheme`、AntD algorithm 和本地语义 token。保持系统字体、无 CDN、普通文字 ≥4.5:1、控件/焦点 ≥3:1 与 reduced-motion；不直接把旧 WA 主题类名或默认英文校验带入新应用。

### D7: 低风险优先分片，但最终一次发布

开发顺序：基础设施与蓝图 → auth/usage/简单反馈 → 项目/会话与设置 → composer/dashboard → transcript/审批最后。每片交付相应单元测试与 Playwright 旅程，高风险转录在其余状态边界稳定后接入。

开发期可通过独立入口或沙箱装配比较新旧界面，但不得让 operator 的 `pnpm run dev`、端口 5273 或真实实例受影响；自己的验证使用 `dev:sandbox` 和隔离沙箱。最终一次切换生产入口，移除 Lit、WA、polyfills、workarounds 与旧测试，不提供用户长期回退开关。此策略连同 proposal 的假设在蓝图评审时确认。

### D8: 测试面向公开语义，基线与候选分开报告

Vitest 增加 React Testing Library、user-event 与 jest-dom；沿用 jsdom/happy-dom 中能够稳定支撑 AntD 的环境，必要时统一为一种并说明理由。测试组件可访问行为和 store 逻辑，不测试 React 私有实例或 AntD 内部 DOM。

Playwright 优先 `getByRole` / `getByLabel` / `getByText`，动态复杂控件才使用稳定的 `data-testid`；先更新 page objects，再处理必要直选器，禁止穿透 WA Shadow DOM 的遗留写法。维护旅程迁移矩阵，保留 ACP 与 native 双通路。

开工前在干净基线运行既有套件并记录预存红/flaky，不覆盖当前工作区已有修改。候选以 `invoke testsuite-webui`、前端测试、`tsc --noEmit`、生产构建及沙箱 HTTP/浏览器验证为门槛；构建扫描最终 `dist` 不含旧运行依赖。真实 provider 仍不纳入 agent 验收。

## Risks / Trade-offs

- **大规模重写与在途前端 change 冲突** → 开始前冻结迁移基线并列出仍在修改的 capability；按页面分片、短周期同步，不在长分支静默偏离 main。
- **交互 redesign 与迁移回归难以区分** → 蓝图明确预期变更，旅程矩阵区分“有意调整”与“应保持”，不把旧断言全部删除。
- **React 重渲染使长转录退化** → 条目级订阅、memo、纯文本 live tail、确定性 burst 测试；禁止按帧全量解析。
- **AntD portal/focus/表单与现有浏览器测试冲突** → 采用公开语义、统一主题容器与键盘旅程，不复制内部 class 断言。
- **开发期双栈增大包体及状态所有权混乱** → 明确单向数据适配层和退场条件，最终构建扫描确认旧栈清零。
- **设置工作区扩建导致范围膨胀** → 只重组既有功能，不新增 provider、权限或服务能力，不新增路由。
- **现有未提交改动及基线红影响判断** → 不动无关代码，先记录基线与当前 diff；无法判定的失败如实报告。
- **一次发布假设尚未确认** → 蓝图评审时拍板；如需逐页发布，先修订 specs/design/tasks，再实现。

## Migration Plan

1. 盘点并冻结行为基线、覆盖矩阵、浏览器偏好与 API/WS 复用清单，记录预存失败。
2. 交付交互蓝图、主题 token 和性能预算，完成用户评审及三项默认假设确认。
3. 搭建 React 工具链和框架无关数据适配层，保持现有 Vite/Rust 打包。
4. 分片迁移页面、组件和测试，先低风险后流式核心；每片用真实沙箱验收。
5. 切换唯一生产入口并删除旧栈，运行完整门禁、生产包检查、无 CDN/无旧依赖检查及视觉评审。
6. 回滚仅限开发期：恢复旧入口与依赖，不改任何后端数据或用户状态。最终发布后若需回退，以代码版本回滚处理，不维护运行时双栈。
