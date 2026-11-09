## Why

当前 WebUI 基于 Lit 3 与 Web Awesome，约 2.4 万行实现代码的渲染、样式、生命周期及测试与二者深度耦合，并积累了多项组件兼容补丁。我们希望统一采用现代 React 生态，同时解决现有工作流与交互设计问题，而不是把旧界面机械翻译成另一套框架。

截至 2026-11-06，经 npm 官方 registry 核实，React / React DOM 最新稳定版为 **19.3.1**，Ant Design 为 **6.7.0**；本 change 以此为版本基线，实施时须复核并明确记录升级结果。

## What Changes

- 以 React、React DOM、TypeScript 与 Ant Design 重建全部 WebUI 页面及组件，最终移除 Lit、Web Awesome 及其兼容垫片。
- 重新设计核心工作流、导航与信息层级，覆盖工作台、项目/会话、对话/审批、设置、用量及登录/首启页面；先交付并评审交互蓝图，再实施。
- 保留现有 HTTP/WS 协议、鉴权/RBAC、增量会话同步、Markdown 安全处理、深链、浏览器持久化语义及 Rust 单二进制资源内嵌机制。
- 建立统一的 Ant Design 主题、中文文案、可访问性、深浅主题、窄屏及减少动态效果规范，保留无 CDN 约束。
- 将框架相关的 Vitest 与 Playwright 选择器迁移到语义角色、可访问名称和稳定测试标识，以完整旅程作为功能回归门槛。
- **BREAKING（仅限前端实现与测试内部）**：Lit 自定义元素、Shadow DOM、Web Awesome 标签和属性不再是前端实现契约；不承诺外部自动化对这些内部 DOM 的兼容。
- 默认采用分页面迁移、完整验收后一次发布，不向最终用户长期提供新旧界面切换。

### Review assumptions（尚未获用户拍板）

- 最终全量替换旧栈，而不是只做一个试点。
- 重做核心工作流，而非仅更新视觉与控件。
- 新旧界面可在开发期共存，但正式版本一次切换、不保留长期双栈。

以上是由未回答的范围、改版深度及发布方式问题形成的可评审默认方案；设计蓝图评审是继续实施的必要门槛。

## Capabilities

### New Capabilities

- `react-webui`: 定义 React/Ant Design 前端的完整替换、交互蓝图、主题与可访问性、流式性能、资源打包、测试迁移及旧栈退场要求。

### Modified Capabilities

- `agent-workbench`: 在不改变业务协议与深链的前提下，调整项目、会话、对话、审批及设置相关核心工作流的信息组织与交互呈现。
- `testsuite-webui-browser`: 浏览器验收从 Lit/Web Awesome 内部结构耦合迁移到用户可观察语义，并覆盖新交互、视觉及响应式门槛。

## Impact

- 主要影响 `sebas-webui/frontend/` 的全部视图、组件、样式、入口及前端单元测试。
- 影响 `tests/testsuite-webui/` 的页面对象、选择器、交互旅程与验收账本；Rust 资源嵌入逻辑原则上保持不变。
- 新增 React、React DOM、Ant Design、React Vite 插件及 React 测试工具，保留 Vite、Vitest、TypeScript 与现有 Markdown 安全依赖。
- API/WS wire 形状、Rust 域逻辑、数据存储、agent 执行体及真实运行实例均不在改造范围。

## Non-goals

- 不重写后端，不新增或变更业务 API、WS RPC、数据库或 IPC 协议。
- 不引入 Tailwind、shadcn、第二套组件库或另一套路由/全局状态框架。
- 不在本 change 增加文件编辑、终端、diff 审查等新业务能力。
- 不要求真实 provider 凭据或真实上游模型验收，不触碰操作员实例。
- 不把 Ant Design 默认样式直接视为完成的设计，也不在蓝图评审前开始全量实现。
