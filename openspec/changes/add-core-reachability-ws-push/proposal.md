## Why

前端对 `/api/summary` 的两处 5s 轮询（app-shell 可达性横幅 + composer 提交门）低效且失真：每个浏览器标签页每 5 秒最多发两次请求，而 handler 每次都全量构建会话行并拉取聚焦会话完整 transcript，读的却只有 `reachability.ok` 一个布尔；断连横幅与提交禁用的翻转最坏延迟 5 秒。状态源本是事件形——core 通道客户端的 `ConnStatus` 翻转全部经过单一收口 `set_status()`，只差一条通知通道。协议基座由先行 change `add-ws-rpc-protocol` 提供。

## What Changes

- 后端：`SessionBackend` trait 新增 `reachability_updates()` 独立广播通道（`permission_requests()` / `subscribe_turn_events()` 先例）；channel 后端在 `set_status()` 收口发布翻转（状态真变才发）；in-process 后端返回立即关闭的接收端。
- 协议面：新增 `core.reachability.get` 请求（返回当前态，客户端连接/重连后调用）与 `core.reachability` Notification（翻转推送），payload 对齐 `/api/summary` 的 `reachability` 形状（`{ok:true}` 或 `{ok:false, kind, cause}`）。
- 前端：删除两处 5s 定时器（`CORE_REACHABILITY_POLL_MS` / `WORKBENCH_REACHABILITY_POLL_MS`）及 composer 挂载时的一次性 fetch；订阅权上收常驻的 app-shell（连上/重连即 get、翻转即更新），composer 提交门消费下传状态。
- shell「login/setup 态跳过轮询」的特判随轮询消亡：auth 开启时 `/ws` 升级前拒绝已天然挡住未认证端。
- `/api/summary` 端点与响应形状不动（dashboard/settings/初始加载继续使用）。

## Capabilities

### New Capabilities

（无——可达性是 `webui` 既有行为面，本 change 只换其数据源与翻转语义。）

### Modified Capabilities

- `webui`：「全局核心可达性横幅」MODIFIED——横幅由轮询驱动改为 WS 推送驱动（初始态 get + 翻转 Notification，恢复即消失、重连即收敛）；「降级与错误表现」MODIFIED——composer 提交门同源改推送，`/api/summary` 降级为纯按需读取端点、不再参与提交门。

## Impact

- 后端：`src/core_channel/client.rs`（`set_status()` 发布）、`sebas-webui/src/session_backend.rs`（trait 通道）、`sebas-webui/src/api.rs`（get handler + 翻转 Notification 支路）。
- 前端：`app-shell.ts`（删轮询、订阅 + get）、`workbench-composer.ts`（删定时器/挂载 fetch，消费下传状态）、必要的状态下传链。
- 测试：Rust 通道翻转→帧序列测试；前端 shell/composer 单测；`testsuite-webui` 沙箱回归（停 core → 横幅即现、提交禁用；core 恢复 → 解除）。
- 协调点（已定序）：落地顺序 `add-ws-rpc-protocol` → 本 change → `add-webui-tiered-notices`；呈现层重整 change 的「全局核心可达性横幅」delta 已按本 change 的推送语义 rebase（fatal 锁定消费推送状态、无轮询），归档顺序即组合顺序。

## Non-goals

- `/api/summary` 瘦身（`active_session` 全量 transcript 下发的优化留后续 change）。
- `execution_bodies` 进推送事件（今天也无轮询消费）。
- SSE 旧链路与 HTTP API 改动。
- 横幅/锁定的呈现层重整（归通知层 change，若在途）。
