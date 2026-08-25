# 重新设计 watchdog 监督模型

## Why

watchdog 经 12 次提交渐进演化，设计与实现已明显漂移：存在两条执行路径（管道 IPC 的
`run_update()` 绕过了 ControlExecutor 声称的"唯一路径"不变量）；约 1200 行死代码
（auth.rs 签名断言机器零调用方、管道升级分支不可达、updater.rs 三个策略函数无调用方）；
服务状态硬编码返回 "running"（说谎）；webui/gateway 子进程 spawn 后无人监管，崩了不会
重启。三个开放 beads（sebas-v8i ServiceManager、sebas-08c gateway 生命周期、
sebas-ivg 死配置字段）都指向同一个根因：监督模型从未被统一设计过。

## What Changes

- **统一受管服务表**：core / webui / gateway 收进一个 ServiceManager，每个服务声明
  spawn 规格、重启策略、就绪判定与真实状态（进程存活 + 端口探测）。
- **单一执行路径**：删除管道 IPC 的 Upgrade/Rollback 分支与 `run_update()`，
  ControlExecutor 成为唯一控制执行路径；管道协议收缩为 Ready 握手（加早期错误行）。
- **服务管理接线**：`ServiceSet` / `ServiceRestart` / `StopCore` / `StartCore` 从
  `service_unavailable` 拒绝变为真正可执行（吸收 sebas-v8i）。
- **gateway 生命周期迁入 watchdog**（吸收 sebas-08c）：watchdog 模式下 gateway 是
  受管子进程；裸 core 模式保留 in-process 形态。
- **死代码清除**：auth.rs 仅保留 actor↔principal 转换；无调用方的策略函数改为真正
  被监督循环使用或删除；死配置字段 `max_retries` / `retry_delay_secs` /
  `check_on_start` 移除（吸收 sebas-ivg）。
- **崩溃策略统一**：一套按服务参数化的重启退避策略；core 保留 NewBinaryNotReady
  自动回滚特例；webui/gateway 崩溃后按策略重启（原来永不重启）。
- **BREAKING**：管道 IPC 子进程协议收缩（同一 binary 一起发布，无外部兼容面）；
  watchdog 配置中的死字段不再被解析（设置了这些字段的旧配置会得到未知字段警告）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `watchdog`：监督模型从"core 特例 + 无人监管的附属子进程"改为统一受管服务表；
  服务管理 RPC 从拒绝变为可用；管道 IPC 协议收缩为 Ready-only；服务状态必须真实；
  死配置字段移除。核心子进程监督、崩溃退避、自动回滚、控制 RPC 鉴权、危险操作
  确认、事件时间线、裸 core 降级模式的需求语义基本保留，按统一模型重述。

## Impact

- `src/watchdog.rs` 大幅瘦身（监督循环 + 服务表）；
  `src/watchdog/services.rs` 升级为 ServiceManager 核心；
  `src/watchdog/auth.rs` 删减至转换函数；`src/watchdog/updater.rs` 策略函数接线。
- `src/ipc.rs` 协议收缩；`src/run.rs` 裸 core 模式提示与 in-process 服务边界调整；
  `src/dispatch.rs` 服务命令适配真实语义。
- beads sebas-v8i / sebas-08c / sebas-ivg 被本变更吸收后关闭。
- 外部行为变化点：`/gateway on|off`、`/restart`、`ServiceSet/Restart` 生效；
  `ServiceStatus` 返回真实状态。

## Non-goals

- 不改 watchdog 与外部 systemd/launchd 的集成方式（`UpdateSignal::WatchdogServiceRestartRequired` 语义保持）。
- 不做事件时间线持久化 / 审计日志（仍为内存 ring buffer）。
- 不改危险操作确认流、RPC 鉴权模型（secret + actor envelope）。
- 不引入 per-service 版本管理与回滚（自动回滚仍仅对 core）。
- 不改 core 内部架构（ACP driver、router、会话管理）。

