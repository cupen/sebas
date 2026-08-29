## Why

WebUI 目前是"看起来像仪表盘"，但实测有三类硬伤：**样式缺失**——模板引用的约 40 个 class（全部 `codex-*`、`timeline-*`、`toast`、`copy-btn`、`page-body`）在 `style.css` 中无任何规则；**实时更新是死的**——`hx-trigger="sse:update"` 没有配套的 `hx-ext="sse"` / `sse-connect="/events"`，`/events` 流从未被消费，列表只能靠手动刷新；**导航指向 404**——侧栏 "Agent" 链接的 `/agent` 路由不存在，`agent.html` 也未注册。加上零 `@media`、零 `:focus-visible`、Markdown/高亮依赖公网 CDN，而 README 明确承诺"离开座位用手机查看进度"——手机上不可用、断网时代码块不渲染。

## What Changes

- **建立设计系统**：token 化的调色板与字体（自托管 woff2，不走 CDN）、状态色阶（Starting/Queued/Working/Done/Failed/Dormant）、间距与圆角刻度，全部集中在 `style.css` 的 `:root`。
- **重做信息骨架**：5 个居中大数字卡片压缩为单行状态条；会话表格改为状态槽 + 状态词 + 等宽标识的行式布局。
- **接上 SSE**：`base.html` 建立单个 `EventSource('/events')`，去抖后刷新会话列表分片，让列表真正实时。
- **补齐样式缺口**：审计模板 class，逐一补规则或删除死代码（含装饰性、不可用的 filter tabs）。
- **消除模板重复**：5 个 `admin_*.html` 各自复制整套 HTML 外壳，改为 `extends base.html`；合并两份重复的 `showToast`。
- **质量地线**：响应式到手机、`:focus-visible` 焦点环、`prefers-reduced-motion`、`prefers-color-scheme` 深色模式、图标按钮补 `aria-label`。
- **本地化资产**：`marked` / `highlight.js` 与字体一并 vendored 进 `static/`（htmx 已是此模式）。
- **文案修正**：`Uptime (s)` 原始秒数改为人类可读；`Phase` 列当前直接显示飞书 reaction API 的枚举值（`Get` / `OnIt` / `CrossMark`），改为控制台自己的状态词；澄清 "Focus 只影响本页显示，不改变 Feishu 路由"；空状态改为可操作的邀请。
- 移除失效的 `/agent` 导航项（该页需要后端路由支持，另开 change）。

## Non-goals

- 不实现 `/agent` 聊天页的后端路由与 `/api/agent/*` 接口。
- 不引入构建步骤、npm 或前端框架——保持 minijinja + htmx + 手写 CSS。
- 不改动认证、绑定地址、mutation origin 校验等安全基线。
- 不改 `/api/sessions/*` 的请求与响应格式。

## Capabilities

### New Capabilities
- `webui-console-ui`: WebUI 的视觉与交互契约——设计 token、状态语义与色阶、响应式断点、可访问性地线、离线可用性。

### Modified Capabilities
- `webui`: 路由与资产要求变更——静态资产改为完全本地（不再依赖外部 CDN）；`/` 概览的呈现由计数卡片改为状态条 + 实时会话板；SSE 流成为 UI 的必需消费方。

## Impact

- `webui/static/style.css`（重写）、`webui/static/` 新增 vendored JS 与自托管字体。
- `webui/templates/*.html` 全部触及；`admin_*.html` 结构性重构为 `extends base.html`。
- `webui/src/models.rs` + `routes.rs`：`SessionRow` 新增由 `(MappingState, phase)` 投影出的展示用状态标签。
- `webui/src/server.rs`：模板注册表随模板增删调整。
- `webui/tests/session_endpoints_test.rs`：断言 HTML 片段的用例需同步。
- 无新增 Rust 依赖。
