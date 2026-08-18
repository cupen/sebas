# sebas 审计：代码与设计一致性

> 日期：2026-08-17
> 对象：`repos-tool/sebas`
> 对标文档：
> - `docs/superpowers/specs/2026-07-26-sebas-design.md`（核心架构）
> - `docs/superpowers/specs/2026-08-14-watchdog-control-plane-design.md`（控制平面）

## 审计概要

| 维度 | 评分 |
|------|------|
| 核心架构一致性 | 🟢 高度一致 — 飞书→路由→ACP→流式卡片核心链路完整贯通 |
| 控制平面一致性 | 🟢 高度一致 — ControlService + RPC + auth/confirmation/updater 按设计分期实现 |
| 安全问题 | 🟡 1 项待确认 — WebUI 安全检查在子进程实现，watchdog 层未主动拦截 |
| 未完成项 | 🔵 均符合设计文档分期策略（Phase 0-3 done, Phase 4+ pending） |

---

## 一、核心架构（2026-07-26 设计）vs 代码

| # | 检查项 | 设计说 | 代码做 | 结论 |
|---|--------|--------|--------|------|
| 1 | 进程模型 | 飞书 WS → Router → ACP 子进程 | `ws_loop` → `RouterEventHandler` → `SessionManager` | ✅ |
| 2 | 消息信封 | FeishuIn/Out, AcpCommand/Event | 枚举定义与设计 §3.1 匹配，扩展了 FormCb/GatewayAction | ✅ |
| 3 | Session 映射 | `HashMap<SessionKey, Handle>` | `router::maps::SessionMap` 实现 | ✅ |
| 4 | 卡片流式更新 | root card + AcpEvent → UpdateCard, emoji 👀→🚧→✅ | `dispatch.rs` + `acp_events.rs` 实现，emoji 用 EYES→OnIt→DONE | ✅ |
| 5 | 权限审批 | SendCard 权限卡 → ButtonCb → PermissionReply | 全链路实现 | ✅ |
| 6 | Slash 命令 | `/new`, `/sessions`, `/switch`, `/cancel` 等 | 全部实现，扩展了 `/provider`, `/settings`, `/btw` | ✅ |
| 7 | 配置系统 | 3 字段必填 + 全默认值 + env 覆盖 | `Config::parse()` 完全匹配 | ✅ |
| 8 | 启动校验 | §6.4: 目录可写 + 二进制可达 | `validate_runtime()` 实现 | ✅ |
| 9 | 错误处理 | §4.1 分类表 | SebasError enum + 分类处理 | ✅ |
| 10 | 重启恢复 | 读 state_file → 懒加载 ACP session | `restore_session_map()` → `acp_resume_and_activate()` | ✅ |
| 11 | 目录结构 | `src/feishu/`, `src/acp/` 内联模块 | 拆分为 workspace crates | ⚠️ 设计过时，但 README 正确 |
| 12 | SessionKey 无 user_id | 设计说"已预留" | 实际只有 chat_id + thread_id | ⚠️ 设计 §7 自己纠正了 |
| 13 | 媒体消息 | 图片/文件/音频下载并传入 agent | feishu 媒体模块实现 | ✅ |
| 14 | 会话超时 | idle_kill_secs 默认 48h | `SessionManager` 配置 | ✅ |

---

## 二、控制平面（2026-08-14 设计）vs code

### 已实现且一致

| 组件 | 设计位置 | 代码位置 | 结论 |
|------|----------|----------|------|
| ControlRequest 枚举 | §8 | `control.rs` | ✅ 含 Status/RestartCore/StopCore/StartCore/Update/Rollback/ServiceSet/ServiceRestart/ServiceStatus |
| Actor 模型 | §6 | `control.rs` | ✅ WebUi/Feishu/Cli/System |
| ControlService 状态机 | §9 | `control.rs` | ✅ PendingConfirmation→Accepted→Running→Succeeded/Failed/Canceled/TimedOut |
| 互斥锁 | §9 冲突策略表 | `control.rs` `is_exclusive()` | ✅ update/rollback/restart/stop 互斥，完成释放 |
| Control RPC 协议 | §5.3 | `control_rpc.rs` | ✅ Unix socket, JSONL, version 1, secret auth |
| Authorization 签名断言 | §6 | `auth.rs` | ✅ MAC tag, nonce replay, expiry, parameter binding |
| Confirmation grants | §7 | `confirmation.rs` + `executor.rs` | ✅ 单次使用、短有效期、绑定 principal/action/channel/params，并发安全 |
| Updater 子进程 | §9 | `updater.rs` | ✅ 超时 SIGTERM→SIGKILL, dev vs release 不同超时 |
| Event timeline | §10 | `events.rs` | ✅ 单调 seq, bounded VecDeque(200), `since(seq)` 查询 |
| RedactedDiagnostic | §10 | `events.rs` | ✅ 脱敏 secret/token/password 等 |
| Phase 3 Feishu 控制路由 | §12 | 核心 router + RPC | ✅ 已合并（git: a9645fb/c8bf7a0） |
| WebUI 非 loopback 拒绝 | §11 | `webui_cmd.rs:65-69` | ✅ 子进程自身检查 |

### 偏差与未完成项

| 偏差项 | 设计说 | 代码做 | 影响 | 备注 |
|--------|--------|--------|------|------|
| supervisor.rs | 独立文件 | 内联在 `mod.rs` `Watchdog::run()` | 低 | 逻辑等价，只是文件拆分问题 |
| adapters/ 目录 | `adapters/webui.rs` 等 | 未拆分 | 低 | 设计说"长期建议" |
| ServiceManager (Phase 4) | 完整 desired/runtime state | 只有 `WebUiLifecycle` 骨架 | 中 | `ControlSet`/`ControlRestart` 显式 reject，代码注释说明 |
| WebUI 托管模式 | 进程内 adapter | 子进程 `sebas webui --config` | 中 | 过渡方案，`webui_cmd.rs` 注释说明 Phase 2.3+ |
| service_status() 硬编码 | 从 ServiceManager 取真实状态 | 硬编码 "running"/"enabled" | 低 | Phase 4 后解决 |
| ConfirmationGrant 字段 | 用 action_hash/principal_hash | 存原始值 | 低 | 功能等价，token 本身不透明 |
| watchdog 自身升级 | §14: core-only + IPC compat | 只重启 core，自身不 reexec | 无 | 符合设计策略 |
| WebUI 子进程生命周期 | watchdog 管理 | watchdog 退出时 kill，core 重启不受影响 | 无 | 行为正确，测试覆盖 |

---

## 三、关键设计约束逐项验证

### 操作互斥（§9）
`is_exclusive()` 覆盖 `RestartCore | StopCore | StartCore | Update{..} | Rollback{..}`，与冲突策略表一致 ✅
测试：`exclusive_operations_return_busy_until_finished`、`consecutive_updates_do_not_deadlock_on_the_exclusive_lock`

### Actor 安全边界（§6.3）
| 规则 | 状态 | 验证 |
|------|------|------|
| RPC client 不能传 `Actor::System` | ✅ | `RpcActor` 枚举无 `System` 变体 |
| Feishu owner 身份由 watchdog 验证 | ✅ | `feishu_principal_channel()` 不接受 Cli/System |
| replayed assertion 拒绝 | ✅ | `auth.rs` nonce 去重 |
| 过期 assertion 拒绝 | ✅ | `auth.rs` expiry check |
| 参数被替换的 assertion 拒绝 | ✅ | `auth.rs` MAC tag 校验 |

### Confirmation 并发安全（§7）
`Mutex<HashMap<...>>` 保护 grant 状态，`redeem()` 原子。两线程并发 redeem 同一 token → 恰好一成功一 `AlreadyRedeemed` ✅
测试：`concurrent_confirm_only_one_execution`

### Panic 后锁释放（§9）
`run_accepted()` 用 `AssertUnwindSafe` + `catch_unwind` → runner panic 仍释放 exclusive lock ✅
测试：`panicking_runner_still_releases_the_lock`

---

## 四、建议修复项

### P1 — 安全（1 项）

**WebUI 非 loopback 检查：watchdog 层应主动拦截，而非依赖子进程**

- 当前：`spawn_webui_process()` 直接把配置传给子进程，子进程启动后 `is_loopback()` 检查（`webui_cmd.rs:65-69`，方法定义在 `services.rs:220`）再拒绝退出
- 问题：watchdog 日志显示 dashboard 地址但子进程立即退出，启动静默失败
- 建议：在 `spawn_webui_process()` 传给子进程前显式检查，非 loopback 直接 warn 并返回 None

```rust
// src/watchdog.rs spawn_webui_process() 内，should_start 检查后：
if let Some(endpoint) = WebUiEndpoint::from_config(&config.webui) {
    if !endpoint.is_loopback() {
        warn!("watchdog.webui.host {} is not loopback; refusing to start", endpoint.host);
        return None;
    }
}
```

### P2 — 代码质量（2 项）
- **service_status() 硬编码**：所有服务返回 "running" 不反映真实状态；Phase 4 前可对 core 用 operation_count>0 判定（已部分实现）
- **`run_watchdog` 创建两次 ControlService**：显式 new 一次 + `Watchdog::new()` 一次，虽被 `with_control` 替换但易混淆

### P3 — 文档/注释（2 项）
- **`spawn_webui_process` 缺少过渡注释**（标注 Phase 2 过渡 → Phase 4 ServiceManager）
- **`webui_cmd.rs` 降级行为不记录**：`SEBAS_CONTROL_SECRET` 未设置时 admin 控制路由变只读，应记录日志

---

## 五、总结

代码与设计高度一致。核心架构（飞书 WebSocket → 路由 → ACP 子进程 → 流式卡片）全链路贯通，控制平面按 Phase 0-3 策略稳步推进，已实现功能与设计约束完全一致。未完成项（ServiceManager Phase 4、Feishu transport broker Phase 5）均在设计分期内，代码注释明确标注 `service_unavailable`。

发现 1 项 P1 安全建议（WebUI 非 loopback 检查应在 watchdog 层主动拦截），修复成本低。