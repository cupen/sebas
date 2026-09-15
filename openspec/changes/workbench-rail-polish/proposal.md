# workbench-rail-polish

## Why

工作台 rail 与品牌层有三处日常摩擦：登录页/首启页/rail 顶三处的「agent router」副标题词不达意；项目行选中高亮与会话行当前高亮用同一套 accent 样式，同屏点亮时层级糊成一片；新建会话成功后项目组被 toggle 误折叠，新会话行不可见，用户得重新找入口才能开口。

## What Changes

- 三处品牌副标题（`app-shell.ts` rail 顶、`login-view.ts`、`setup-view.ts`）由「agent router」统一改为「你忠诚的 AI 伙伴」，样式不变（纯文案，无 spec 需求变更）。
- rail 高亮分层：会话行「当前会话」标记保留 accent 底色；项目行选中态改为中性提亮（表面色提亮 + 文字变亮，不用 accent 色），active 语义不变（仍表示主区正在显示的项目）。
- 新建会话后的焦点链：创建成功后项目组强制展开（修掉 `onSelect` toggle 误折叠），新会话行立即呈选中态，键盘焦点落进 composer 输入框，用户可立刻打字。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `agent-workbench`: 新增「rail 选中层级视觉可分」需求（项目选中中性提亮、会话当前 accent，两者同屏可辨）；新增「创建会话后焦点落在占位会话」需求（项目组保持展开、新会话行可见且选中、输入框获得键盘焦点）。

## Impact

- `sebas-webui/frontend/src/app-shell.ts`、`login-view.ts`、`setup-view.ts`（副标题文案）；`project-rail.ts`（高亮样式、创建成功后的展开与焦点链）；`workbench-composer.ts` 或 dashboard 焦点链（输入框对焦）。
- 无协议/API/持久化变更；现有单元测试与 webui 浏览器测试需同步更新。

## Non-goals

- 不改 rail 信息架构（History/Waiting 组、项目分组、行内菜单）。
- 不改会话高亮的交互语义（仍由 `active_session_key` 驱动）。
- 不动对话流呈现与折叠（另立 change：workbench-natural-conversation-flow）。
- 不引入新色彩令牌或多色相区分（动主题令牌留待真有第二种状态语义时）。
