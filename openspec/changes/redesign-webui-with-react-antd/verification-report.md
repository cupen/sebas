# 实施验收记录

## 已通过的验收

- 生产入口：`index.html` → `src/react/main.tsx`，Vite 单一 `index` 输入；Lit/Web Awesome 从依赖、源码、dist 与 Rust 二进制扫描移除。
- 前端：`pnpm test` 42 文件 / 382 用例通过；`pnpm exec tsc --noEmit` 0 错误；`pnpm build` 成功；`cargo build --bin sebas` 成功。
- 主浏览器 config：179 通过 / 0 失败 / 1 个平台条件跳过（`--retries=0`）。
- React 专项 30/30；auth 35/35；auth-setup 4/4；deployment 1/1；dead-core 1/1；detached 7 通过 + 1 个平台条件跳过。
- 最终 `invoke testsuite-webui` 的非 native 链路合计 **225 通过 / 1 跳过**；日志 `/tmp/sebas-react-antd-l2/testsuite-webui.log`。
- 生产 SPA fallback、深链、静态资源 immutable、无 CDN、API 不误落 SPA、旧栈二进制扫描均通过；日志 `/tmp/sebas-react-antd-cutover/`。
- 独立 visual judge 对六张生产 PNG 的整体 gate 为 **PASS**：desktop dark/light、settings dark、usage light、375 mobile、375 Drawer。375 Drawer 真实测量导航/主题/关闭/项目及会话操作 ≥44px；截图 `/tmp/sebas-react-antd-visual/`，最终 Drawer `mobile-drawer-touch-final.png`。
- 性能硬预算实测通过：无逐帧全量拉取/历史 Markdown 重解析/历史 render，长任务与滚动/内存/响应式满足批准预算；产物 `/tmp/sebas-react-antd-stage4b/perf/`。

## 未完成的 native 验收（不得宣称整个 change 完成）

最终 `invoke testsuite-webui` 退出非零：native config **6 失败**，均在 `POST /api/sessions/{key}/model` 对 `test/*` 返回 HTTP 404。失败沙箱 `/tmp/sbtestsuite._fjic_jd`。

现行 `make-pi-the-builtin-agent` 将 `agent == "native"` 转发到 ACP pi（`src/agent_backend.rs` 的 spawn/create_placeholder），而 native Playwright config 仍以 debug router 的 native 内核 `test/*` 场景为前提。这是既有后端/测试语义冲突，不属于本前端 change 的协议范围；未用 ACP 旅程冒充 native 通过，未修改无关 Rust 工作区改动。

后续 issue：**sebas-f7hc（P1）统一 native 场景测试与 pi 内置槽位语义**。需决定 native 测试改驱 pi 场景或恢复独立 native 内核测试路径，再完成 4.4、4.5 与 5.5 的完整验收。当前 change 不归档。
