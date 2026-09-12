# Design: unify-router-process-shape

## Context

router 现有两种形态：内嵌（`run.rs` 在 `core --router` 下起同进程 router，
随机端口记日志）与独立（watchdog 子进程 `sebas router --config [--debug]`、
手工 `sebas router`）。经拷问确认：provider/模型/别名/defaults 数据与
presets 已全部走 core 状态库（`state_snapshot("presets")` 等），设置页
不依赖 router 进程；agent 上游有 Off/Direct/Router 三档，Direct 不经
router。router 进程的剩余价值 = 外部 CLI 网关 + Router 档会话。
watchdog 规格已有「router 默认关 + 服务页持久化开关」，本轮不重复建设。

## Goals / Non-Goals

Goals：删内嵌形态、`run`/`core` 旗标清理、router 停止保护（活跃 routed
会话拒绝 + force 逃生门）、服务页强制出口流、测试装配两进程化。

Non-Goals（proposal 已列）：裸核控制面、外部 CLI 消费者保护、provider
三档语义、router 自身鉴权。

## Decisions

### D1: 形态统一 = 只删不加

内嵌 router 是纯删除：`run.rs` 的内嵌启动块（含「router started
(core --router)」日志、随机端口回读）与 `cli.rs` 的 `--router`/`--debug`
旗标（`--debug` 在 run 上仅服务内嵌 router，随形态消亡）。watchdog 的
router 子进程 spawn 与手工 CLI 不动——三者共用 `sebas_router::server::run`
内核，删的只是 core 侧装配。`--debug` 在 watchdog 路径继续生效
（`config.router.enabled || debug` 语义不变）。

### D2: 停止保护的执法点在 watchdog executor，事实源是 core 状态库

权威执法点 = stop 决策所在处（executor 处理 `ServiceSet`）。「活跃
routed 会话」= 状态库中 provider mode 为 Router 且状态非终态
（Running/Warming；Queue 中未起的不算——router 停了它们 spawn 时会诚实
报错，属可接受失败）。executor 经 core session channel 查询计数：复用
`state_snapshot` 机制加一个轻量 domain（如 `router_activity`），watchdog
以 core channel client 身份连接（secret 走既有 config 目录发现链）。
core 不可达 → 放行停止：core 不在就没有任何活跃流，fail-open 是诚实语义
（fail-closed 反而挡住一个无风险的清理动作）。管道（readiness handshake）
按规格不动。

### D3: force 的 wire 与语义

`ServiceSet` 增 `force: bool`（serde 默认 false），仅对
`{router, off}` 有意义，其他组合忽略。拒绝响应带 code
`active_routed_sessions` + 会话计数，前端据此渲染。竞态模型：不做预防性
预查询（弹窗由拒绝驱动），force 是操作者显式意图——确认期间会话结束则
force 白白绕过（无害），新会话起来则操作者已明示承担。`ServiceRestart`
不加 force（router 的 restart 语义是「重启回可用」，不删流）。

### D4: 服务页交互 = 拒绝驱动弹窗

既有 disable 已有 confirm 弹窗（点击即弹）。router 停止流在 confirm 之
后仍可能被服务端拒（400 + `active_routed_sessions` + 计数）——此时弹第二
层对话框：计数 + 「流式会话将中断」文案 + 「强制停止」按钮。前端不做
活跃数预查询（避免新增只读端点与竞态窗口）；409/400 之外的失败走既有
内联错误。

### D5: 测试装配两进程化

浏览器套件与 e2e 的 `core --router --debug --webui` 单进程改为：core
（`--webui`，无 router 旗标）+ `sebas router --config <沙箱配置> --debug`
子进程；清理侧 SIGTERM 两进程。沙箱菜谱（AGENTS.md）同步改两进程写法。
detached 变体（core + 独立 webui）本就无 router，不动。

## Risks / Trade-offs

- [watchdog↔core 新增查询依赖] → 复用既有 core channel client 与 secret
  发现链，无新鉴权面；查询失败按 D2 语义放行，不阻塞管理操作。
- [`--router` 删除破坏既有脚本] → BREAKING 已在 proposal 声明；仓库内
  引用面（AGENTS.md、tasks.py、e2e）同变更内迁移；未发布产品无外部脚本
  存量。
- [Queue 中 routed 会话在 router 停后 spawn 失败] → 诚实报错
  （SEBAS_PROVIDER_ERROR 路径），不静默降级 Direct；属既定语义。
- [保护只覆盖 agent 会话] → 外部 CLI 消费者不可统计，文档明示；强停的
  操作者显式意图已覆盖该风险敞口。

## Migration Plan

无数据迁移：删形态 + 加拒绝逻辑，均为代码行为。回滚即回退二进制（force
字段旧 watchdog 忽略未知字段——serde 默认容错，需实现时确认 `deny_unknown_
fields` 未开）。配置键无增删。

## Open Questions

- `router_activity` snapshot domain 的具体 payload（计数 or 会话 key 列表）
  实现期定——前端只消费计数，列表不必要。
