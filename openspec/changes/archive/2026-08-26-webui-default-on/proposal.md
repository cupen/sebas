## Why

WebUI 目前是全 opt-in：`run --webui` flag 默认 false、watchdog
`[watchdog.webui] enabled` 默认 false、或手动 `sebas webui` 子命令。新用户
装好 sebas 后没有任何可视化入口，也感知不到 dashboard 的存在；把 WebUI
变为默认启动项可以显著降低上手门槛，并与近期 gateway admin API /
provider 管理 UI 的投入形成闭环。

## What Changes

- watchdog 模式下 `[watchdog.webui] enabled` 默认翻转为 `true`：watchdog
  默认 spawn 并监督 WebUI 子进程（携带 control secret，含完整 `/admin/*`
  与 `/gateway` 能力）。
- 裸 `sebas run`（非 watchdog）默认仍**不**启动 WebUI，但 `--webui` 保持
  可用；新增 `--no-webui` 对称 flag（与 `--webui` 互斥）。
- **BREAKING**（行为级，非 API 级）：默认占用 `127.0.0.1:9797`。端口被
  占用时 WebUI 子进程以特定退出码（75）退出，watchdog supervisor 识别后
  标记该服务为 `Degraded` 并停止重试，绝不 crash 主进程；后续可通过控制
  平面 restart 重试。
- `ServiceState` 新增 `Degraded` 变体，控制平面 service status 可查询。
- Ownership guard 由端口绑定原子性天然保证（不额外实现锁文件/IPC）。
- 启动日志新增一条 info：WebUI 地址与是否 degraded。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`: 「Watchdog lifecycle ownership」requirement 的默认值从 opt-in
  翻转为默认启用（`enabled = false` 显式关闭）；新增「bind 失败退出码」
  与「Supervisor Degraded 状态」requirement。

## Impact

- `src/config.rs`：`WatchdogWebUiConfig::default()` 的 `enabled` 翻转，
  serde default 同步。
- `src/webui_cmd.rs`：bind 失败时 `exit(75)` 而非返回 `Err`。
- `src/watchdog/supervisor.rs`：`ServiceState` 新增 `Degraded` 变体；
  `supervise()` 的 `Exit::Crashed` 分支识别退出码 75 → 标记 degraded。
- `src/watchdog/executor.rs` + `control_rpc.rs`：`Degraded` 序列化支持。
- `src/watchdog/services.rs`：合并 `should_start_watchdog_webui` 内联。
- `src/cli.rs` / `src/main.rs`：`--no-webui` flag。
- `config/config.toml.example`：示例配置更新并注释新默认值。
- 既有用户如依赖 9797 端口或不愿多一个进程，需要显式
  `enabled = false`（发布说明中标注）。

## Non-goals

- 不改变 WebUI 的安全基线（仍 loopback-only，admin 密码鉴权不变）。
- 不改变裸 `sebas run` 的默认行为（默认仍不启动 WebUI）。
- 不涉及远程/非 loopback 暴露或 TLS。
- 不做 WebUI 功能新增，只改生命周期默认值与降级策略。