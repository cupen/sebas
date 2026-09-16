## Why

全局错误提示目前只有两条互不相干的非锁定横幅（WS 断线、core 不可达），而「core 不可达时页面浏览不受影响」的既有语义会让操作者在瘫痪的后端上继续操作，徒增困惑与误提交；其余 API 错误散落在各视图内联或被吞掉。需要一套按严重级别分级的全局通知层，并在核心瘫痪时锁定操作直到恢复。

## What Changes

- 新增视口级 top-center 通知层，承载四级通知：info（蓝 toast，约 5s 自动消）、warn（琥珀：瞬时 toast 约 8s / 持续态驻留横幅）、error（红 toast，驻留须手动关）、fatal（驻留横幅 + 全屏锁定遮罩）。
- fatal 语义：core 不可达（`/api/summary` 的 `reachability.ok = false`）时整个应用框架 `inert` 锁定（浏览也锁），横幅自身可交互；5s 轮询沿用，恢复后自动解锁并弹「核心已恢复」info toast；auth 门禁页（登录/首启设置）不在锁定范围。
- 删除现有 ws-banner 与 core-banner，行为收编进新层（WS 断线 = 持续 warn 驻留横幅；两者同时在场纵向堆叠）。
- `client.ts` 统一拦截未豁免的 API 失败，按影响面自动判级弹出（操作失败 → warn toast）；表单与带重试按钮的内联错误点 opt-out 保留内联，避免双弹。
- 前端 `ReachabilityInfo` 接后端已有的 `reachability.kind`（startup_failed / auth_rejected / disconnected），fatal 横幅按 kind 分文案，cause 原文保留。
- 判级原则「前端说了算 + 影响面定级」：应用瘫痪 = fatal、视图/能力不可用 = error、单操作失败可重试 = warn、无损通知 = info；HTTP 状态只是信号。
- 实现选型：瞬时条用官方 `wa-toast` 栈经 `wa-overrides.css` 换肤 sebas token；驻留横幅与锁定遮罩自研轻量组件（视觉方案见 design）。

## Capabilities

### New Capabilities

（无——通知层是 `webui` 既有全局行为面的重整，不另立能力面。）

### Modified Capabilities

- `webui`：「全局核心可达性横幅」MODIFIED 为 fatal 锁定语义（含 `reachability.kind` 分文案与恢复提示）；ADDED「分级通知层」需求（四级形态、来源判级、栈行为、a11y、WS 断线横幅收编）。「降级与错误表现」的既有契约（断线指示器、内联重试态、网络级失败区分）语义不变、由新层满足，不出 delta。

## Impact

- 前端：`app-shell.ts`（旧横幅删除、新层挂载、锁定控制）、新增 notice-layer 组件与通知 store、`api/client.ts`（错误拦截钩子 + opt-out 参数）、`api/client.ts` 的 `ReachabilityInfo` 类型补 `kind`、`styles/tokens.css` 与 `wa-overrides.css`（info 蓝等变体 token）。
- 后端：无改动（`reachability.kind` 已在 `/api/summary` 输出中）。
- 测试：notice-layer / app-shell 单测，`testsuite-webui-browser` 增补锁定与分级场景。

## Non-goals

- 后端 error body 增加级别字段（判级纯前端）。
- 401 处理流程改造（维持既有跳登录）。
- 视图内联错误迁移（settings 表单、composer 提交、列表加载重试保留内联 + opt-out）。
- 瞬时 warn 的实际生产者（本期无来源，规则先立、有源再接）。
- info 承载正向成功提示（如「已复制」）的强制收编（通道可用，不强推）。
