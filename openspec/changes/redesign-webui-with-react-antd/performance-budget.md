# 性能验收预算（performance budget）

- 状态：**评审草案**。数字是 4.6 实现的验收目标，不是当前测量结果；本阶段（L1）只记录既有前端基线，不宣称 React 新版性能。
- 对照基线（2026-11-08，`40d8ce3`，`pnpm build`）：JS 1,767.57 kB / **533.81 kB gzip**，CSS 111.05 kB / **16.69 kB gzip**，单 chunk 超过 Vite 500 kB 警告阈值。日志：`/tmp/sebas-react-antd-baseline/frontend-build.log`。
- 所有预算都必须在同一沙箱、同一 fixture、同一测量脚本下复现；报告 p50/p95/max 与运行环境，不用单次采样下结论。

## 1. 测量口径

| 项 | 定义 | 工具 |
|---|---|---|
| 交互响应 | 用户输入事件 → 下一次 paint（PerformanceObserver `event`/`paint` + CDP trace） | Playwright + Chromium trace |
| 长任务 | 主线程任务 >50ms；预算关注 >200ms 的硬失败 | PerformanceObserver `longtask` |
| 帧间隔 | rAF 相邻帧时间；流式期间 p95 | 页面内 rAF 采样 |
| 请求数 | 指定时间窗内 `fetch`/XHR/WS 请求计数，按 URL 分类 | Playwright `page.on('request')` + WS 帧计数 |
| 渲染次数 | 每个转录条目组件的 render 计数（测试构建注入计数器） | React Profiler / 自定义计数器 |
| Markdown 调用 | `marked` / DOMPurify / highlight.js 调用次数 | 测试构建包装计数 |
| 内存 | `performance.memory.usedJSHeapSize` 与 CDP `HeapProfiler` | Chromium |
| 溢出 | `document.documentElement.scrollWidth <= clientWidth + 1`；元素 bounding box 越界 | Playwright evaluate |
| 可访问性 | axe-core（或等价）关键规则；手动对比度/焦点检查 | Playwright + 人工复核 |

Fixture（确定性，无随机、无真实 provider）：

- 长转录：500 条，混合用户/助手 Markdown（含 10k 字符条目、GFM 表格、代码块、长 URL、CJK 长句）、thinking、工具调用/结果配对、审批卡、notice。
- 滴流：40 chunk/s × 60s（2400 帧），每帧 1–40 字符。
- 突发：2s 内 200 条 append + 4 个并行工具事件。
- 会话：3 个项目 × 各 10 个会话，其中一个 500 条聚焦会话。
- 视口：1440×900、1024×768、768×900、375×720。
- 主题：dark/light 各跑一次；`prefers-reduced-motion` 单独一次。

运行纪律：每个场景 5 次，预热 1 次不计；不使用固定 sleep 等结果；记录 commit、浏览器版本、CPU/内存、是否 headless；产物放 `/tmp/sebas-react-antd-baseline/perf/<scenario>/`。

## 2. 硬预算（不满足即失败）

### 2.1 流式与数据边界

| 指标 | 预算 | 失败判据 |
|---|---|---|
| 每帧全量转录请求 | **0** | 任一 delta 触发 `GET /api/sessions/{key}`（无 `entries_after`） |
| 流式期间 Markdown 重解析 | **0 次/帧** | 任一流式帧调用 `marked`/DOMPurify |
| 回合结算 Markdown 解析 | 1 次/助手回合 | 同一结算回合重复解析 >1 |
| 未变化历史条目重渲染 | **0 次/delta** | 计数器显示 position ≤ 已知最大值的条目 render |
| 重复 DOM 条目 | 0 | 同一 `position` 在 DOM 出现 >1 次 |
| 长任务 | 无 >200ms；p95 ≤50ms | 任一 >200ms 或 p95 >50ms |
| 总阻塞时间（TBT） | 60s 滴流期间 ≤300ms | 超出 |
| 请求预算 | 初始 detail 1 + catch-up ≤1 + WS 帧；重连额外 ≤1 detail | 超预算或按帧请求 |

### 2.2 交互响应（长转录 500 条场景）

| 操作 | 预算（p95） | 备注 |
|---|---|---|
| 键入 → paint | ≤50ms（p99 ≤100ms） | 输入框单次字符 |
| 发送 → 本地排队反馈可见 | ≤100ms | 不等服务端 |
| 审批允许/拒绝 → 卡片状态 + 转录条目 | ≤200ms | 并行卡互不阻塞 |
| rail 点击会话 → 目标对话首屏内容 | ≤300ms | 既有节流窗口内 |
| 切换设置分区 → 内容可见 | ≤150ms | 草稿保留 |
| 打开/关闭 Drawer（375） | ≤200ms；reduced-motion 下即时 | 焦点归还 |
| 跳到最新 | ≤300ms 可见并可点 | 脱离底部时出现 |

### 2.3 滚动与阅读位置

| 指标 | 预算 |
|---|---|
| 贴底跟随时新条目到达后距底 | ≤8px，100ms 内 |
| 已脱离时新条目 | 自动滚动距离 = 0px |
| 审批决策后阅读位置 | 视口偏移变化 ≤2px |
| 展开/收起历史折叠 | 相邻条目位置不跳变（≤2px） |

### 2.4 响应式与主题

| 检查 | 预算 |
|---|---|
| 375/768/1024/1440 横向溢出 | 0（`scrollWidth <= clientWidth + 1`） |
| 375 主操作触控目标 | ≥44×44px（不重叠） |
| 深/浅主题关键页 | 无不可辨识文本；正文 ≥4.5:1，控件/焦点 ≥3:1 |
| reduced-motion | 非必要动画/过渡时长为 0；内容仍实时更新 |

## 3. 软预算（超出需说明并评审）

| 指标 | 目标 | 处理 |
|---|---|---|
| 初始路由 JS gzip | ≤550 kB（基线 533.81 kB，允许 +3%） | 超出先做路由/分区懒加载与 AntD 按需引入；仍超出提交评审 |
| 总 JS gzip（含懒加载 chunk） | ≤700 kB | 记录每 chunk；不得用 CDN |
| CSS gzip | ≤40 kB | 以 AntD token + CSS Modules 控制 |
| FCP | ≤1000ms p75（沙箱本机，无 CPU 节流） | 超出记录并定位；不以远端网络为借口 |
| LCP | ≤1800ms p75 | 同上 |
| 500 条转录 JS heap | ≤250 MB | 超出先查重复订阅/闭包/未卸载 |
| 5 分钟滴流 heap 增长 | ≤20% | 超出视为泄漏风险，硬阻塞 |
| 会话切换 p95 | ≤300ms（硬预算） | 若基线更慢，按「不劣化于基线 +10%」复核 |
| 主题切换 | ≤100ms 完成重绘 | 超出检查是否重挂整棵转录 |

## 4. 明确禁止的“优化”

1. 以虚拟化换取性能但破坏未读定位、展开状态、工具配对、浏览器内查找：**不接受**。若 4.6 引入窗口化，必须同时通过上述四项旅程并记录证据。
2. 按流式帧全量拉取或全量重解析历史：**硬失败**。
3. 用 `React.memo` 掩盖错误的状态所有权（整棵 store 快照变化导致全量重渲染）：先修订阅粒度，再谈 memo。
4. 用固定 sleep/重试制造稳定：测试纪律不变（既有 `conversation-streaming` 的 `retries: 0`、零固定 sleep 保留）。
5. 把 AntD 默认样式或全量引入当作性能中性的：包体与首屏必须实测。

## 5. 验收产物

4.6/5.5 必须提交：

- `perf/stream-drip-60s.json`：帧间隔、长任务、TBT、请求计数、Markdown 调用、render 计数。
- `perf/stream-burst-2s.json`：同上 + 并行工具/审批。
- `perf/long-transcript-500.json`：交互响应、滚动、内存、DOM 唯一性。
- `perf/responsive-375-1440.json`：四档溢出、触控目标、主题对比度。
- `perf/bundle.json`：`vite build` 每 chunk 原始/gzip 大小、是否含旧栈依赖。
- 与 Lit 基线的对照表（同 fixture）：交互 p95、帧间隔 p95、请求数、内存、包体。
- 失败项与预存失败分开列出，不把环境缺失计为通过。

## 6. 本阶段（L1）范围声明

本阶段只做：

1. 既有 Lit 前端的 `pnpm test` / `tsc --noEmit` / `pnpm build` 基线（见 `baseline-report.md`）。
2. 蓝图的静态检查与独立浏览器截图（非生产 GUI 验收）。
3. 本文档作为 1.6 评审材料。

本阶段**没有**测量 React/AntD 性能，也不声称任何新版性能结论。
