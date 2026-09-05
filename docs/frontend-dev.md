# 前端联调（Vite 热更新 + Rust 后端）

开发 WebUI 前端时不必每次 `cargo build`——Vite dev server 提供秒级热更新，
后端表面（JSON API / WebSocket / 健康检查 / Gateway BFF）由代理转发到本机
Rust 进程，浏览器全程同源，与生产嵌入形态路径一致。

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
`/api`、`/router/api`、`/ws`（WebSocket）、`/health` → `127.0.0.1:9797`（退役 SPA 路径 `/gateway` 仍归一化到 `/`）。
前端代码全部使用相对路径请求，因此无需任何环境变量或代码改动。

## 注意事项

- **端口**：Vite 固定使用 5273（strictPort），被占用时直接报错而不是顺延，
  释放端口后重启即可；后端端口 9797 不受影响。
- **改前端即时生效**（HMR）；改 Rust 代码需要重新编译重启后端进程，前端无需动。
- **WebSocket 实时事件**走代理 `ws: true` 透传，`session.created/updated/removed`
  事件链路与生产形态一致；断线由前端自动重连。
- **Router BFF**（`/router/api/*`，POST/PUT/DELETE）：后端守卫只放行 loopback
  origin，Vite dev origin 同属 127.0.0.1，联调时行为与生产一致（无
  `SEBAS_CONTROL_SECRET` 时返回 503）。
- **测试**：`pnpm test`（vitest，不依赖后端）；联调中的端到端
  验证用浏览器或 curl 直接打 Vite 地址即可。

> 仓库集成测试与沙箱联调约定见 `AGENTS.md`。
