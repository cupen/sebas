## 1. 基线、范围与交互蓝图

- [x] 1.1 盘点现有页面、路由、组件、API/WS 调用、localStorage key 及在途 change，形成复用/重写/删除清单；检查清单覆盖全部 `sebas-*` 视图及关联 capability。
- [x] 1.2 在不覆盖现有工作区改动的前提下建立干净迁移基线并运行前端测试、类型检查、生产构建和浏览器套件，保存预存失败/flaky/跳过项及现场路径。
- [x] 1.3 建立浏览器旅程迁移矩阵，逐条标记等价迁移、经评审调整或删除及理由；核对 ACP/native 双通路与 `tests/acceptance/COVERAGE.md` 无静默缩面。
- [x] 1.4 交付桌面工作台、项目/会话、设置、用量、登录/首启的交互蓝图，覆盖空/载入/错误/断线状态；在独立可视化预览中检查 1440/1024/768 布局，不修改生产入口。
- [x] 1.5 交付 375px 窄屏、审批/队列、流式转录、深浅主题、键盘与减少动态效果蓝图；检查导航抽屉、焦点路径、输入/待办遮挡及状态辨识。
- [x] 1.6 将蓝图、性能预算及全量替换/核心工作流改版/一次发布三项默认假设提交用户评审，记录结论；若范围改变，先更新 proposal/specs/design 并确认对应门禁仍可执行。

## 2. React 基础设施与数据边界

- [x] 2.1 复核并安装锁定的 React 19.3.1、React DOM 19.3.1、AntD 6.7.0、官方 React Vite 插件和 React 测试工具，记录版本及 peer 检查；`pnpm install --frozen-lockfile` 与 `pnpm build` 成功。
- [x] 2.2 配置 TSX、React 测试环境、CSS Modules 和隔离开发入口，保持 Vite 5273/strictPort、sandbox 代理与 `dist` 输出；独立预览可启动且未启动/停止 operator 的 dev server。
- [x] 2.3 实现 AntD 中文 locale、明暗 algorithm、应用语义 token、系统主题/显式主题和 reduced-motion 适配；主题测试及关键色值对比度检查通过。
- [x] 2.4 将现有 route matcher、深链原始编码参数、retired redirects 和 localStorage adapter 接入 React；路由/偏好单测覆盖连续深链切换、未知路径与既有 key 读取。
- [x] 2.5 将 API client、协议类型、模型目录、通知及纯转录转换复用为框架无关模块，拆除 WS 对 Lit ReactiveController 的依赖；既有纯逻辑测试及 WS 重连/鉴权测试通过。
- [x] 2.6 实现 auth/shell、project/session、conversation、settings 的可订阅 store 与 `useSyncExternalStore` adapter；测试验证选择器局部更新、退订清理、stale response 隔离和世代失效重取。

## 3. 低风险页面与公共交互迁移


- [x] 3.1 实现 React 应用壳、全局通知、fatal 锁定、用户入口及可访问导航；免登录/登录/首启/RBAC 与 core 翻转的对应浏览器旅程通过。
- [x] 3.2 迁移登录、首启和错误/空/载入组件，统一中文校验、密码管理器兼容及焦点恢复；鉴权组件测试与真实 auth-on/zero-user 浏览器旅程通过。
- [x] 3.3 迁移用量页面和本地 SVG 图表，复用既有 API 真值及过滤语义；usage 单测、深链、空/错误态及浏览器旅程通过。
- [x] 3.4 迁移项目导航、目录选择器、项目增删排序及跨项目会话概览，提供窄屏 Drawer 和排序替代操作；项目管理与 folder-picker 旅程通过。
- [x] 3.5 迁移会话创建、聚焦、命名、关闭、归档/恢复及确认交互，保持 agent/mode/model 请求形状；会话管理、连续深链和归档只读旅程通过。


- [x] 3.6 分批迁移设置的 Services/Users/Models/Aliases/Agents/Skills/Env/Appearance 分区为无独立 URL 的浮动窗口，保留草稿及 RBAC；各分区单元测试和对应浏览器旅程通过。

## 4. 对话核心与性能迁移

- [x] 4.1 实现 dashboard 的项目/焦点状态连接和 composer 的提交、停止、排队、斜杠命令、模型/模式交互；多轮、队列、取消、输入守卫及 reachability 旅程通过。
- [x] 4.2 实现按会话/条目订阅的转录呈现、稳定 key、未读缝、滚动跟随与工具过程折叠；测试证明切换/迟到帧不串会话且历史展开态稳定。
- [x] 4.3 迁移纯文本 live tail 与结算 Markdown 渲染，保留 DOMPurify 和 highlight.js 边界；恶意 HTML、长文、代码、表格、特殊字符及流式结算测试通过。
- [ ] 4.4 实现会话内统一待办、独立并行审批和排队操作，决策结果同步转录且不改变阅读位置；ACP 与 native 审批旅程分别通过。
- [ ] 4.5 迁移 WS 重连、去重/乱序、游标恢复、空回合、错误与子进程死亡呈现；断线恢复及 ACP/native 流式旅程在无新增固定 sleep 下通过。
- [x] 4.6 建立确定性长文、burst 和并行工具性能 harness，记录 React render、长任务、请求数、Markdown 调用及交互响应；证明无逐帧全量拉取/历史重解析并满足 1.6 评审预算。




## 5. 测试收口与旧栈退场

- [x] 5.1 完成 React Testing Library 与 Playwright 页面对象的语义选择器迁移，删除旧 Shadow DOM/WA 内部断言；检查迁移矩阵所有条目均有明确结果。
- [x] 5.2 补齐 375px、深浅主题、键盘、reduced-motion、焦点与横向溢出旅程，对蓝图关键页面进行独立视觉验收；所有发现问题修复并复验。
- [x] 5.3 一次切换生产入口，删除 Lit/Web Awesome、WA polyfills、workarounds、旧预览和无用测试；依赖树、源码及最终 `dist` 扫描无旧运行依赖。
- [x] 5.4 更新前端开发、构建与测试文档，说明 React/AntD 版本、主题、数据适配层和 sandbox 验证；文档命令在隔离环境实际执行成功。
- [ ] 5.5 在隔离沙箱执行 `pnpm test`、`pnpm exec tsc --noEmit`、`pnpm build`、`cargo build` 和 `invoke testsuite-webui`，再验证生产 SPA fallback、自托管资源及完整 API/WS 旅程；输出基线对照、跳过项与最终日志，不触碰真实实例。
