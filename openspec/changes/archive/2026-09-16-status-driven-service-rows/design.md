# Design: status-driven-service-rows

## Context

- 后端唯一合成点：`src/watchdog/executor.rs` 的 `service_status()`——列表头注入 `watchdog` 行、尾注入 `updater` 行（状态取自 update 操作记录）。控制面 `ControlRequest::ServiceSet/ServiceRestart` 的 `service` 字段是 `ManagedService` 枚举（core/webui/router/feishu/im），`"watchdog"`/`"updater"` 在 wire 层反序列化即失败——「不可控」已是结构事实。
- 前端唯一渲染点：`sebas-webui/frontend/src/views/settings-modal.ts` 的 `renderServiceRow()`。现状：非 core 行无条件渲染 ▶■⟳，core 行只渲染 ⟳ 且绑定的是通用 `runServiceAction('restart', 'core')` → `/api/admin/services/core/restart`，而该路由在控制 RPC 层必拒 core（webui_cmd.rs 注释自认 core 走 `/api/admin/restart`，但前端从未调用）。布局 `.service-card` 为 `info(flex:1) | status | actions(auto)`，动作区宽度随行内容变化，状态列横向漂移。
- 消费面：`/api/admin/services` 仅供 webui；`sebas ctl services`（main.rs）与 IM `/services`（im_cmd.rs）直接消费 control RPC 的 `ServiceStatus` 响应。三处共享同一列表形状变化。

## Goals / Non-Goals

**Goals:**

- 列表形状：`ServiceStatus` 恰好等于受管 entry 快照集合，无注入行。
- 动作语义：按钮 = 当前 status 下的真实下一步动作，杜绝必拒/空操作按钮。
- 布局：状态列跨行同一 x 对齐，定宽动作区 + 占位。

**Non-Goals:**

- 不改 ManagedService 枚举、控制 RPC 执法、force 停止保护、确认弹窗流程。
- 不改 updater / 升级回滚机制本身；`/api/admin/restart` 后端路由保留（admin API 面，CLI 的 RestartCore 走独立直连路径）。
- 不做 IM/CLI 渲染样式改版——它们只被动少两行。

## Decisions

**D1 — 后端删合成行，而非前端过滤。** 合成行对三个消费面都是噪音，且「行出现在列表里」与「行可被控制」的矛盾在后端消除才彻底。替代案（前端 filter、API 加 include 参数）被否：前者留下三面两种事实，后者无人需要。影响：`executor.rs:1258` 等钉死 watchdog 行的测试改为断言「恰为受管集合」。

**D2 — 动作映射以 actual status 为准（非 desired）。** 用户选定。规则表：

| status | 动作区 |
|---|---|
| running | ■ |
| stopped / disabled | ▶ |
| starting / restarting | 禁用过渡占位（spinner/省略号，不可点） |
| degraded / failed-startup | ■ + ⟳ |

过渡态选择「占位」而非「隐藏」是为了配合 D4 定宽对齐；选择 status 而非 desired 的代价（watchdog 自动重启期间无法表达意图）由 busy 禁用 + 刷新后到位弥补。⟳ 在非过渡态恒渲染（stopped 时等效 ▶，无害）；未知 status 值按过渡占位降级（fail-safe，不猜按钮）。

**D3 — core 行纯只读，删除前端死方法。** core 的 ⟳ 现状是哑弹（绑定必拒路径），拆除即修复；`api.restartCore()` 在前端无调用方，一并删除。后端 `/api/admin/restart` 保留：admin API 面有独立规格与 RBAC 测试，且「core 重启走确认的危险动作路径」语义（RestartCore）仍是升级/回滚流程的承重点。

**D4 — 定宽动作区实现对齐（CSS-only）。** `.service-actions` 从 auto 宽改为固定宽（按 2 钮宽度），`justify-content: flex-end`；不足处自然留白。不引入 grid / 新组件。状态列无需改动——它右贴动作区，动作区定宽后状态列即对齐。

## Risks / Trade-offs

- [CLI/IM 消费方可能有人依赖 watchdog/updater 行] → 两者都是本仓自有渲染端（main.rs / im_cmd.rs），同步检查无特判逻辑即可；`ServiceStatusFor` 单服务查询不受影响。
- [列表收窄对外部脚本是响应形状变化] → `/api/admin/services` 仅本 webui 消费，收窄条目不破坏字段结构；proposal Impact 已声明。
- [degraded 下 ■ 会放弃自动重试] → 这是操作员显式意图，且 desired 持久化需再次 ▶ 才恢复，符合「下车道」语义。
- [定宽动作区若按钮样式变化（字号/内边距）需同步常量] → 宽度用 CSS 自定义属性声明一次，两处引用。

## Migration Plan

单次提交原子落地：后端删合成行 + 测试更新 → 前端动作映射 + 对齐 + 死方法清理 → `cargo test` + 前端 vitest 全绿 → 沙箱（`invoke testsuite-webui-sandbox`）目验四行、互斥、对齐。回滚 = revert 单提交。

## Open Questions

（无——grilling 已闭合：隐藏层级、互斥规则、core 只读、降级态动作均由操作员裁定。）
