# workbench-rail-polish — Design

## Context

三处副标题硬编码（`app-shell.ts:605`、`login-view.ts:172`、`setup-view.ts:200`），无测试引用。rail 高亮现状：项目行 `.row.active` 与会话行 `li.session-item.current` 是同一套 `--sebas-accent-soft` 底 + accent 文字（`project-rail.ts:194/262`）；项目行 active 由 `activePath`（app-shell `selectedPath`，随 `rail-select` 设置）驱动，会话行 current 由 `active_session_key` 驱动。创建会话的焦点链 bug 在 `project-rail.ts:480`：`confirmNewSession` 成功后调用 `this.onSelect(p.path)`，而 `onSelect` 是 toggle——从已展开项目行「+」进来时必折叠。composer 对焦无现成先例（`openSession` 点击不抢键盘焦点，本设计也不改变这一点）。

## Goals / Non-Goals

**Goals:**

- 三处副标题一处定义、三处引用，消灭漂移。
- 高亮分层只动 CSS，不改两处高亮的驱动语义。
- 创建成功后的焦点链：展开、选中、对焦三个动作显式化，不再借用 toggle。

**Non-Goals:**

- 不新增主题令牌/色相（见 D1 被否项）。
- 不改 `rail-select` 事件与主区切换语义。
- 不给 openSession/会话切换加 composer 对焦。

## Decisions

- **D1 高亮分层走既有令牌**：项目行 active 改 `background: var(--sebas-surface-2/3)` + `color: var(--sebas-text-bright)`（hover 同底色时以字重或更亮一档区分，实现取现有变量组合，不新造令牌）；会话行 current 原样保留。被否：双色相区分（引入第二色彩语义要动 `theme.ts` 全局令牌，为单一状态不值得）；被否：去掉项目 active（主区显示哪个项目的反馈仍有价值，语义保留只换皮）。
- **D2 焦点链三步显式化**：`confirmNewSession` 成功后 ①`this.expanded = { …this.expanded, [p.path]: true }`（强制展开，非 toggle）②不调用 `onSelect`，`focusedKey` 交给 `refresh()` 后的 `active_session_key` 回填（创建请求服务端已 set_focus）③ composer 对焦经 dashboard 向 `sebas-workbench-composer` 发一次性焦点请求（自定义事件或公开 `focusInput()` 方法，实现取侵入小者）。被否：给 `onSelect` 加 force 参数（依旧触发 `rail-select` 导致主区项目切换语义混入创建路径，职责不清）。
- **D3 副标题常量化**：导出 `APP_TAGLINE = '你忠诚的 AI 伙伴'`（放 `theme.ts` 旁或 `app-shell.ts` 导出，取引用最自然处），三处模板引用之。被否：三处各写字面量（下一次改口令还会漏）。

## Risks / Trade-offs

- [中性高亮与 hover 底色同为 surface 系，选中态可能不够醒目] → 选中态叠加文字变亮 + 字重，浏览器测试里断言类名而非像素；视觉验收留给 sandbox 冒烟。
- [composer 对焦在移动端/触屏会弹键盘] → 仅创建成功这一个时机对焦，会话切换不抢；可接受。
- [副标题改中文而登录页可能面向未登录态] → 三处统一是拍板决策（拷问轮 2 默认项），UI 其余文案本就以中文为主。

## Open Questions

（无——拷问轮 2 未获回答的项已按推荐默认拍板并在 proposal/specs 固化：高亮方案取「会话保 accent、项目改中性」；对焦取「含键盘焦点」；副标题三处统一。）
