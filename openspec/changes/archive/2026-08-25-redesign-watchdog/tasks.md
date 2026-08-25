## 1. 死代码清除（先减后建）

- [x] 1.1 管道协议收缩为 Ready 单行：`ChildMsg` 删 `Upgrade`/`UpgradeDev`/`Rollback`，删 `send_watchdog_command`/`WATCHDOG_TX` 与 run.rs 子进程侧常驻监听 loop，`init_watchdog_ipc` 缩为「启动完成写一行 ready」。验证：`cargo build` 通过且 `grep -rn "send_watchdog_command\|ChildMsg::Upgrade" src/` 无结果，`cargo test ipc` 通过
- [x] 1.2 删 watchdog.rs 的 `run_update()` 与 `handle_ipc()` 的升级/回滚分支及 `ParentIpc::ok/error/done` 助手（restart_rx 分支暂留）。验证：`cargo test watchdog` 通过，`/upgrade` 走 RPC 路径不受影响（现有 dispatch 测试）
- [x] 1.3 auth.rs 删减至 principal 类型 + `actor_to_principal`/`principal_to_actor` 两个转换（删 Verifier/MacProvider/SignedAssertion/AssertionBuilder/ActorVerifier/VerifiedActor/RejectionReason 及其测试）。验证：`grep -rn "Verifier\|MacProvider\|SignedAssertion" src/ | grep -v "^src/watchdog/auth.rs"` 无结果，`cargo test` 全绿
- [x] 1.4 updater.rs 删 `UpdateSignal`/`ControlPlaneImpact`/`classify_update_impact`/`recommended_recovery`/`update_signal_message` 及测试（`classify_readiness_failure` 保留待 2.3 接线）。验证：`cargo test watchdog::updater` 通过，全仓 grep 无引用
- [x] 1.5 config 删 `max_retries`/`retry_delay_secs`/`check_on_start` 字段与 default 函数；watchdog 段解析前用 toml::Value 扫描废弃键并 warn 一行（不报错）。验证：新单测「含废弃字段的 TOML 解析成功且 stderr 有 warn」通过

## 2. ServiceManager 与统一监督循环

- [x] 2.1 新建 `src/watchdog/supervisor.rs`：`ServiceName` 枚举 + `SupervisedService` 监督 task（spawn 规格、私有命令 mpsc、快照上报、按服务独立崩溃退避：窗口 1h / 上限 3 / 超限睡 30s 重置）。验证：FakeChild 单测覆盖「意外退出→按退避重启」「stop 命令→不重启」「超限→30s 后重置继续」
- [x] 2.2 升级 `src/watchdog/services.rs` 为 `ServiceManager`：句柄表、快照聚合查询、命令转发、`~/.sebas/services.json` 读写（config 默认 → 文件覆盖 → 运行时覆盖三层合成）。验证：单测覆盖三层合成与 persist 写盘/不写盘
- [x] 2.3 core 监督 task：readiness 门（管道读 `ready`）+ 退出分类调用 `classify_readiness_failure()`（删除 watchdog.rs 内联重复实现）+ `just_performed_update`/`received_ready` 迁入 task 局部状态 + 自动回滚钩子。验证：单测覆盖「升级后未 Ready→回滚不计 crash」「Ready 后退出→正常退避」
- [x] 2.4 `run_watchdog()` 重装配：`Watchdog` 结构体消解，watchdog.rs 瘦身为装配 + 收尸；debug gateway 并入 gateway 服务条目（desired 来自 `--debug`）。验证：`cargo test watchdog` 通过；`sebas watchdog --debug` 手动冒烟启动日志正常
- [x] 2.5 服务接入：webui（沿用 `[watchdog.webui] enabled`）、gateway（新增 `[watchdog.gateway] enabled`，默认 false）、各自 spawn 规格与 SEBAS_CONTROL_SECRET env。验证：`[watchdog.gateway] enabled=true` 时 gateway 子进程被 spawn 且崩溃后自动重启（集成测试或手动 kill 冒烟）

## 3. 控制面接线

- [x] 3.1 executor 持 `ServiceManager` 句柄：`plan_for` 增 `ServiceAction` 分支（ServiceSet/ServiceRestart 立即 settle）；`RestartCore` 改调 `services.restart(core)`；删 `restart_tx/rx` 通道与 `PostAction`。验证：`cargo test watchdog::executor` 通过，含「RestartCore 经 ServiceManager 生效」新测试
- [x] 3.2 `service_status()`/`service_status_for()` 改读 ServiceManager 快照：core/webui/gateway 真实状态（running/restarting/stopped/disabled + pid + uptime_secs），删 feishu 硬编码行。验证：单测断言「webui 被 stop 后状态为 stopped」「feishu 不在服务列表」
- [x] 3.3 ServiceSet/ServiceRestart 语义落地：webui/gateway 执行（含 persist 写 services.json）；命名 core → Rejected 并指向 RestartCore。验证：control_rpc 层测试覆盖三种路径（执行/持久化/拒绝）

## 4. 端到端与收尾

- [x] 4.1 端到端冒烟：`sebas watchdog --debug` 启动 → `/services` 真实状态 → kill webui 子进程 → 观察自动重启与状态翻转 → `/gateway off` + persist → 重启 watchdog 后 gateway 仍关闭。验证：手动冒烟清单执行一遍并记录输出
- [x] 4.2 全量质量门：`cargo test`（全 workspace）、`cargo fmt --check`、`cargo clippy` 无新告警。验证：三条命令零失败
- [x] 4.3 beads 收尾：关闭 sebas-v8i / sebas-08c / sebas-ivg（close reason 注明由 redesign-watchdog 吸收），`bd close` 后 `bd ready` 无这三条
- [x] 4.4 归档准备：`openspec validate redesign-watchdog --strict` 通过；确认 delta 与实现一致（ServiceSet wire 值 on/off、feishu 行删除等均已在 delta 中）
