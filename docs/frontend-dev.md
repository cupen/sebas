# 前端联调（Vite 热更新 + Rust 后端）

WebUI 前端是 React 19 + Ant Design 6 的单页应用，源码在 `sebas-webui/frontend/`。
生产形态是**单二进制**：`cargo build` 时 `build.rs` 先跑 `pnpm build`，Vite 输出
`frontend/dist`，`rust-embed` 把它编进 `sebas`；`/` 与所有未命中路径（SPA fallback）
都由嵌入的 `index.html` 提供，`/assets/*` 提供内容哈希后的静态资源。浏览器与后端
始终同源，前端只用相对路径请求 API。

开发时不必每次 `cargo build`——Vite dev server 提供秒级热更新，后端表面
（JSON API / WebSocket / 健康检查 / Gateway BFF）由代理转发到本机 Rust 进程。

## 启动

```bash
# 终端 1：Rust 后端（裸跑或 watchdog 形态均可）
./target/release/sebas core --config <你的配置> --webui
# WebUI API 默认监听 127.0.0.1:9797

# 终端 2：Vite dev server（前端源码热更新）
cd sebas-webui/frontend && pnpm dev
# 输出 Local: http://localhost:5273/（专用端口，strictPort——被占用时直接报错而不是顺延）
```

浏览器打开 Vite 输出的地址即可。`sebas-webui/frontend/vite.config.ts` 已配置代理：
`/api`、`/gateway`、`/ws`（WebSocket）、`/health` → `127.0.0.1:9797`。

沙箱联调（一次性实例，端口 9877，不碰 `~/.sebas` 与 9797）：

```bash
cd sebas-webui/frontend && pnpm dev:sandbox
# 或显式指定后端：SEBAS_WEBUI_BACKEND=http://127.0.0.1:9879 pnpm dev
```

后端目标按优先级解析：`SEBAS_WEBUI_BACKEND`（完整 origin）> `--mode sandbox`
（9877）> 默认（9797）。

## 主题

单一开关是 `<html>` 上的 `data-theme`（`dark` / `light`）：

- `index.html` 的内联脚本在 bundle 执行前预绘制，避免浅色系统上的闪黑；
- `src/react/theme/ThemeProvider.tsx` 接管运行时写入，并把语义色板以 17 个
  `--sebas-*` 自定义属性内联，再在 AntD `ConfigProvider` 里换成同色 token；
- `src/styles/tokens.css` 的浅色重映射只认 `:root[data-theme='light']`；
- 模式（`system` / `dark` / `light`）由设置页持久化到 `localStorage['sebas:theme']`，
  读写逻辑在 `src/react/boundary/preferences.ts`，`src/theme.ts` 只剩 `ThemeMode` 类型。

## 代码分层

- `src/react/`：唯一的 UI 层。`main.tsx` → `App.tsx`（路由 / RBAC / 主题 / 通知宿主）→
  `app/`（Shell、ProjectRail、会话创建、登录 / 首启、目录选择器）、`pages/`（会话总览、
  设置、用量）、`conversation/`（会话面板、转录、输入框）、`state/`（外部 store +
  `useSyncExternalStore`）。
- `src/views/*.ts`：与框架无关的纯模型 / 持久化逻辑（转录折叠、未读锚、用量序列、
  分栏比例、模式词表、工作台持久化）。React 组件从这些模块取值，测试也直接打这些模块。
- `src/api/`：HTTP 客户端、模型目录、共享 WebSocket 与事件解析。
- `src/components/`：`markdown.ts`（marked + DOMPurify + highlight.js）与
  `chart-geometry.ts`（SVG 折线几何，纯函数）。
- `src/styles/`：`tokens.css`（语义色板）与 `app.css`（全局基线 + AntD 让位规则）。

样式一律用 CSS Modules（`*.module.css`），类名与 AntD 组件 props 分离；不要新增全局
选择器去覆盖 AntD 内部结构。

## 测试与构建

```bash
cd sebas-webui/frontend
pnpm test                 # vitest（happy-dom；个别文件用 jsdom）
pnpm exec tsc --noEmit    # 整个前端类型检查
pnpm build                # 生产构建 → dist/（唯一入口 index.html）
```

浏览器旅程（Playwright）由 `tests/testsuite-webui/` 驱动，沙箱装配细节见该目录
`README.md`；React 生产入口的旅程挂在 `playwright.react*.config.ts`。

## 性能夹具（仅开发）

`perf.html` + `src/perf/main.tsx` 是长转录渲染的性能量测夹具，用来复现 4.6 的
滚动 / 折叠证据。它**不在** Vite 构建输入里，因此不会进入 `dist` 或嵌入二进制：

```bash
cd sebas-webui/frontend && pnpm dev
# 打开 http://localhost:5273/perf.html
```

## 注意事项

- **端口**：Vite 固定使用 5273（strictPort），被占用时直接报错而不是顺延。
- **改前端即时生效**（HMR）；改 Rust 代码需要重新编译重启后端进程。
- **WebSocket 实时事件**走代理 `ws: true` 透传，`session.created/updated/removed`
  事件链路与生产形态一致；断线由前端自动重连。
- **Gateway BFF**（`/gateway`，POST/PUT/DELETE）：后端守卫只放行 loopback origin，
  Vite dev origin 同属 127.0.0.1，联调时行为与生产一致（无 `SEBAS_CONTROL_SECRET`
  时返回 503）。
- **测试**：`pnpm test`（vitest，不依赖后端）；联调中的端到端验证用浏览器或 curl
  直接打 Vite 地址即可。

> 仓库集成测试与沙箱联调约定见 `AGENTS.md`。
