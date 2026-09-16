## Context

动机见 proposal（Why）。落设计前已核实的现状约束：

- `app-shell.ts` 有两条**非锁定**横幅：`ws-banner`（琥珀）与 `core-banner`（红，5s 轮询 `/api/summary` 驱动），骑在主区顶部；spec 既有 Requirement「全局核心可达性横幅」明文「SHALL 不阻塞浏览」——本期将其反转为 fatal 锁定。
- 项目无全局 toast 系统；Web Awesome **3.12.0** 自带 `wa-toast`（栈）+ `wa-toast-item`，本设计已核对其样式 API：栈支持 `placement="top-center"`（`inset-block-start:0; inline-start:50%; translate:-50%`）、`--width`（默认 28rem）、`--gap`、`::part(stack)`、≤480px 自动全宽；条目可见色全部走 token——`--accent-color`（左侧 4px 色条 + 图标 + 自动消失倒计时环的指示色，`:host` 源码定义并消费；默认 `--wa-color-fill-loud`）、`--wa-color-surface-raised`（底）、`--wa-color-surface-border`（边）、`--wa-color-text-normal`（文）、`--wa-shadow-l`；出入场动画与 `prefers-reduced-motion` 内建；**关闭按钮无条件内建渲染（3.12.0 无 `closable` 属性）**；live region 按变体自动选 role（danger→alert/assertive，其余→status/polite）。
- `wa-overrides.css` 已把 `--wa-color-surface-*` / `--wa-color-text-*` / brand 色阶映射到 sebas 调色板——toast 的表面/文字/边框因此**零覆盖即入景**，只需换 accent。
- `tokens.css` 无 warn/info 专用状态 token（旧横幅的 `var(--sebas-status-warn, #b45309)` 一直在吃 fallback 字面量）；且现行 core-banner 白字落在 `--sebas-status-failed`（#f47174）上，对比度不足 AA——本期一并修正。
- 后端 `/api/summary` 已输出 `reachability.kind`（`startup_failed | auth_rejected | disconnected`，`api.rs:113`），前端 `ReachabilityInfo` 只声明了 `{ok, cause?}` 未接。

## Goals / Non-Goals

**Goals:**

- 单一视口级 top-center 通知层承载四级通知（形态与判级规则见 specs delta，此处不复述）。
- 瞬时条完全站在 Web Awesome 组件上（不重造动画/计时/无障碍广播），只做 token 换肤。
- fatal 锁定的焦点管理与视觉处理一次做对（含 auth 门禁页豁免）。

**Non-Goals:** 见 proposal Non-goals；另加两条设计边界——不重定义 `--sebas-status-*` 既有语义（status 色相描述机器状态，notice 是系统对人的通道，二者以 alias 关联而非合并）；不动 `wa-overrides.css` 的既有 brand/surface 映射。

## Decisions

**D1 — 通知层为单一 Lit 组件 `sebas-notice-layer`，挂 app-shell 根部**
视口级 `position: fixed; top: var(--sebas-notice-top, 12px); left: 50%; translate: -50%`，纵向 flex 堆叠，`z-index` 取 settings 弹窗（`wa-dialog` overlay）实测值 +1（实现期读数，tasks 落数字）。持续横幅（warn/fatal）渲染在层顶部，toast 栈经共享 `--sebas-notice-top-offset` 变量整体下移横幅高度——两个 fixed 容器不重叠。备选（否）：横幅留在 main 区、toast 独立 fixed——两套定位上下文迟早打架；把 wa-toast 塞进自研容器——其 host 是 fixed popover，塞不进文档流。

**D2 — 瞬时条：官方 `wa-toast` 栈 + 变体映射，只换 accent**
`placement="top-center"`、`--width: 28rem`。四级→WA 变体与换肤（全部走已核实的文档化 API，attributes 优先、token 次之）：

| 级别 | wa-toast-item 变体 | accent token | 时长 |
|---|---|---|---|
| info | `brand`（live=polite） | `--sebas-notice-info`（= `--sebas-status-starting` #6ea8fe 蓝） | 5s 自动消 |
| warn | `warning`（live=polite） | `--sebas-notice-warn`（= `--sebas-status-working` #f2b04e 琥珀） | 8s 自动消 |
| error | `danger`（live=alert/assertive，WA 内建） | `--sebas-notice-error`（= `--sebas-status-failed` #f47174 红） | 驻留 |
| fatal | 不走 toast（D3 驻留横幅） | — | 驻留 + 锁 |

条目底/边/字沿用主题（surface-raised / surface-border / text-normal 已 sebas 化），左侧 4px accent 条 + 图标 + 文案三通道传达级别（满足「颜色不是唯一通道」）；自动消失的条目自带 progress-ring 倒计时（accent 同色）。时长由 `duration` 承载、layer 统一配置：info/warn 给秒数自动消，**error 给 `duration=0` 驻留至手动关闭**（关闭按钮 WA 内建恒在，info/warn 可随时提前手关，无需开关属性）。

**D3 — 持续横幅与锁定遮罩自研（`sebas-notice-banner` / 遮罩层）**
持续态不合身 toast（见共识），复用现有 banner 的视觉语言（全宽条、图标 + 文案、role=alert）：warn 态底色 `--sebas-notice-warn-bg`（深琥珀、对白字 AA）；fatal 态底色 `--sebas-notice-fatal-bg`（**深红新 token**，修现行白字对 #f47174 的对比缺陷；文案按 kind 分档 + cause 原文，不可关）。fatal 锁定：对 `wa-split-panel.frame`（rail + main 的共同祖先）施加 `inert` + 视觉遮罩（`--wa-color-overlay-modal` 同款半透明、居中一张原因卡与轮询提示）；横幅在遮罩之上保持可交互（tabindex=-1，锁定时焦点移入，恢复归还此前焦点、元素已失则落 body）。auth 门禁页在 shell 渲染分支之外，天然不受影响。备选（否）：遮罩只盖 main 留 rail 可点——rail 的项目树同样依赖 core，可点也无意义，全锁语义更简单。

**D4 — 判级入口：`notice.ts` store + client.ts 拦截器**
模块级轻量 store（订阅广播，同 `sebas:ws-state`/`sebas:refetch` 的事件风格）：`notify({level, message, dedupeKey?})` 供视图显式上报（error 级生产者）；client.ts 在 `ApiError` 抛出前统一拦截未豁免调用，按影响面映射（网络失败/5xx 于操作调用 → warn「操作失败」类文案；视图级加载失败由视图 opt-out 后自行内联或上报 error）。豁免名单集中在 client.ts 一处以注释互链（settings 表单、composer 提交、dashboard/列表加载、`/api/summary` 轮询、401 跳登录路径）。栈上限 3（挤最旧瞬时条）与 8s 去重窗口在 store 内实现。

**D5 — kind 分文案**
`ReachabilityInfo` 补 `kind?: 'startup_failed' | 'auth_rejected' | 'disconnected'`；fatal 横幅三档文案（启动失败 / 核心拒绝接入 / 连接断开）+ cause 原文小字；`kind` 缺失时退化为通用「核心不可达」。接掉 app-shell 注释里的既有 TODO（cover-A kind 字段）。

**D6 — token 增补（tokens.css notice 组）**
`--sebas-notice-info/warn/error`（alias 既有 status hue）+ `--sebas-notice-warn-bg/fatal-bg`（深底、AA 对白字）。新增不改动既有 status 语义。

**D7 — 窄屏与动效**
≤480px：WA 栈自动全宽贴顶（源码证实），横幅同层全宽，其余不变。出入场一律用 WA 内建动画（show/hide + FLIP 重排）；`prefers-reduced-motion` 由 WA 处理，自研横幅沿用既有 `sebas-view-in` 的 reduced-motion 先例。

## Risks / Trade-offs

- [wa-toast（fixed popover）与自研横幅同屏重叠] → 共享 `--sebas-notice-top-offset` 变量，横幅在场时栈整体下移；单测断言偏移生效。
- [拦截器漏弹/双弹] → 豁免名单集中一处 + 注释互链本 change；vitest 覆盖「表单点不弹、未豁免点必弹」。
- [fatal 锁定与既有 `sebas:refetch`/WS 重连叠加] → 无冲突：refetch 是数据层行为；锁定期间视图冻结可接受，恢复后 refetch 收敛。
- [inert 在测试环境（happy-dom）行为差异] → 断言停留在 attribute 层（`inert` 存在/移除），不模拟焦点漫游。
- [旧 `data-testid="core-unreachable-banner"` 的既有测试迁移] → tasks 显式列入（改指新横幅 testid）。
- [5s 轮询间隔内恢复感知延迟] → 「核心已恢复」info toast 补偿；不缩短轮询（既有间隔是稳定性决策）。

## Migration Plan

纯前端：新增组件与 store → client.ts 接拦截 → app-shell 删旧横幅换新层 → 测试迁移。无数据迁移；回滚 = revert 单 commit。旧横幅相关 spec 场景（「全局核心可达性横幅」）由本期 delta MODIFIED 承接，归档时自动替换。

## Open Questions

（无——时长/去重窗口等次要数值已按共识记为假设，实现期可微调。）
