## Context

当前四条「启动失败」处理路径各走各的：watchdog 对受管子进程的 `spawn failed` 走 `warn!` + 1 s 退避无限重试（`src/watchdog/supervisor.rs:353`），systemd 看不出根因；`New-binary auto-rollback` 在 `src/watchdog/updater.rs` 与 `openspec/specs/watchdog/spec.md:71` 都允许 rollback 失败后 silently continue；bare core 形态（`sebas core --webui` 沙箱联调）下启动失败仅在 stderr 写一行就退，webui 与 watchdog 端都不知道；`sebas-webui/src/session_backend.rs:412` 注释明确「web_spawn never fails structurally: ... the spawn failure surfaces as a Removed event later」——故意延后；`sebas-dispatch/src/dispatch.rs:112/129` 对 `web_spawn` 的 spawn 失败只 `warn!` 不传播。本次把分散处理收敛为 fail-fast：spawn 失败有限重试+终态、rollback 失败终止 watchdog、启动失败统一 EX_TEMPFAIL (75) 退出码、web_spawn 失败 inline 到 transcript。

## Goals / Non-Goals

**Goals:**
- 受管服务 spawn 失败有限重试（默认 3，可由 `[watchdog] max_spawn_failures` 配置）后进入 `failed-startup` 终态；watchdog 以 75 退出。
- rollback 失败后 watchdog 同样进入 `failed-startup` 终态、不再 silently continue。
- 所有 sebas 子命令（core/webui/router/run）启动失败 → 退出码 75、stderr 末行 `startup-failure: <原因>`、可写入 `SEBAS_STARTUP_ERROR_FILE`。
- webui web_spawn 派生 acp 子进程失败 → 立即 inline 到 transcript（不延后到 Removed）。
- 失败摘要经既有 `sebas ctl status` / `GET /api/summary` 暴露给触发者。
- 既有 happy-path（运行期崩溃退避、auto-rollback 触发后的重启、核心 happy-path）不变。

**Non-Goals:**
- 不引入新的上报通道（继续 systemd unit 状态 + stderr + `sebas ctl status` + 飞书 boot 通知）。
- 不重写 watchdog 监督循环结构。
- 不动 happy-path 的 readiness 协议、crash counter、crash backoff 既有规约。
- 不支持运行期动态回滚（仅启动阶段触发回滚）。
- 不改 systemd unit 模板（`Restart=on-failure` 已经会因 75 走指数退避——验证这条假设）。

## Decisions

### D1：spawn 失败重试上限默认 3，由 `[watchdog] max_spawn_failures` 配置
- 决策：受管服务连续 spawn 失败 N 次（N=默认 3）→ 该 service 状态置 `failed-startup`、watchdog 进程以 EX_TEMPFAIL (75) 退出；窗口内每次失败写入结构化日志（含 stderr 摘要）。
- 依据：3 次既能给配置错误（多 pl 一次性起不来）足够诊断时间，又避免 `Restart=on-failure` 把 watchdog 整体无限重启掩盖根因。
- 备选：N=1（一次失败就退）→ 否决：太过激进，单次 transient 故障会让 watchdog 整体退；N=∞（现状）→ 否决：违反本次规约。

### D2：`SEBAS_STARTUP_ERROR_FILE` 作为 bare-core 沙箱验收的契约
- 决策：所有 sebas 子命令在 ready 之前 fatal 时 SHALL 写一行 `startup-failure: <可读原因>` 到 `SEBAS_STARTUP_ERROR_FILE`（若环境变量设置）；同时 stderr 末行也保证同一行。
- 依据：沙箱联调（AGENTS.md）依赖此契约做断言（脚本读错误摘要决定下一步）；stdem 也方便 systemd 的 `ExecStartPre`/集成测试读取。
- 备选：只写 stderr → 否决：沙箱在 systemd 下 stderr 与日志分离，单一行可被丢到主 syslog，触发者不好定位；双写保险。

### D3：退出码 75（EX_TEMPFAIL）的语义边界
- 决策：启动失败（spawn/ready 前 fatal/rollback 失败）→ 退出码 75；运行时崩溃（已 ready 后 panic）→ 既有退出码（典型 1）；watchdog 整体退出 75 时 systemd `Restart=on-failure` 走指数退避。
- 依据：75 是 sysexits.h 标准「临时失败」，systemd 默认 `Restart=on-failure` 对此码通常会指数退避（默认 100 ms→5 s，最大 5 min）。这一行为既避免了快速重启循环，又保留了 systemd 主动重试的能力。
- 备选：退出码 1 → 否决：与运行时崩溃不可区分；用户无法用 systemd/journald 按码过滤。

### D4：rollback 失败不再 silently continue，统一进入 `failed-startup` 终态
- 决策：auto-rollback 失败（无 backup 或 rollback 命令失败）→ watchdog 写结构化日志、置 `failed-startup`、退出 75、`sebas ctl status` 报告。
- 依据：当前 `openspec/specs/watchdog/spec.md:71` 写明 "logs the failure and continues its supervision loop"——这正是你提的"假装没发生"案例，必须改。
- 备选：rollback 失败后再重试 rollback 几次 → 否决：rollback 失败大概率是磁盘/二进制不可用，重试不能改善；快失败让操作员介入。

### D5：web_spawn 失败 inline 到 transcript，不延后到 Removed
- 决策：`sebas-webui/src/session_backend.rs:412` 的 "web_spawn never fails structurally: the spawn failure surfaces as a Removed event later" 注释策略改为：spawn 调用失败立刻通过 dispatch 推到 transcript 作为一个明确的错误事件；Removed 事件仍可作为后续信号。
- 依据：当前注释明示这是「故意延后」——违反你的规约（隐藏当期错误）。spawn 失败是用户最关心的当期问题，inline 是诚实呈现。
- 备选：保留延后但加 UI 兜底 → 否决：双路径不可避免分叉；以 spec 直接改。

### D6：`sebas ctl status` 与 `GET /api/summary` 暴露 startup-failure 原因
- 决策：`sebas ctl status` 在某服务 `failed-startup` 时增加字段 `startup_failure: { service: "core", count: 3, last_stderr: "...", at: "<iso>" }`；`GET /api/summary` 的 `reachability.cause` 在 core 启动失败时携带该摘要。
- 依据：触发者（操作员 / CI / systemd 后续接管）必须能从单一入口看到失败原因，避免 grep 多文件。
- 备选：单独 `sebas ctl startup-failure` 子命令 → 否决：增加 CLI 表面而无收益。

### D7：失败计数 reset 边界
- 决策：spawn 失败计数器在以下情况 reset：(a) 该服务成功 spawn 并 ready；(b) 距离上次失败 1 小时以上；(c) watchdog 进程重启。
- 依据：与既有 `Crash backoff`（spec line 34）的 1 h 窗口对齐，保持规约一致性。
- 备选：仅靠 watchdog 重启 reset → 否决：过严，导致 watchdog 启动几小时后的瞬态故障无法重置。

### D8：不引入新通道，保留 systemd unit + stderr + ctl status 三件套
- 决策：本期不新增上报通道（不写飞书 boot 通知改版、不改 systemd unit 模板、不引入 webhook）。
- 依据：现有 systemd unit (`Restart=on-failure`) 已经对 75 走指数退避；`sebas ctl status` 已有 `failed-startup` 字段扩展点；最小动作面符合"启动失败即可见"规约。
- 备选：新增 systemd notify 协议 → 否决：spec 改动面大、单元模板要改，超出本期。

## Risks / Trade-offs

- [R1] `Restart=on-failure` 对 75 的具体行为依赖 systemd 配置 → mitigation：在 tasks 4.1 跑 `sebas run` 在沙箱里启动后立刻停掉、观察 systemd `Restart=on-failure` 的真实退避间隔；若与默认不一致，更新 systemd unit 模板的注释（不动模板本身）。
- [R2] `[watchdog] max_spawn_failures` 默认 3 可能太短 → mitigation：可配置；测试覆盖 N=1/N=3/N=10 三个边界；本期不动默认。
- [R3] web_spawn 失败 inline 可能让 transcript 出现重复错误事件（多次失败累计） → mitigation：合并同一会话内相邻 N 秒的同类失败事件，避免 transcript 刷屏。
- [R4] 沙箱测试需要构造「启动失败」场景 → mitigation：在 sandbox 跑 `target/debug/sebas core -c /tmp/garbage.toml` 验证 75 + stderr 末行；fake-spawner 在 supervisor 单测里已经被覆盖（`src/watchdog/supervisor.rs:748/770`）。
- [R5] `SEBAS_STARTUP_ERROR_FILE` 在 windows 下的路径处理 → mitigation：使用 `PathBuf` 而非字符串拼接；测试用 sandbox `/tmp` 风格路径，Windows 沙箱已经走 cygpath。
- [R6] `sebas ctl status` 增加字段后既有 CLI 输出解析可能破坏 → mitigation：增加字段而非改字段；新字段缺失时按缺省空对象处理（向后兼容）。

## Migration Plan

按四笔 commit 顺序独立可回滚：

1. **退出码与错误摘要（commit 1）**：所有 sebas 子命令入口添加 startup-failure 检测 + stderr 末行 + `SEBAS_STARTUP_ERROR_FILE` 写入 + 退出码 75；bare core 形态联调覆盖。运行 `pnpm test` 与 `invoke testsuite-webui-sandbox`（验证沙箱联调）+ `cargo test --workspace`。
2. **watchdog 终态化（commit 2）**：`src/watchdog/supervisor.rs` spawn 失败重试上限接入；`src/watchdog/updater.rs` rollback 失败终止化；`sebas ctl status` 与 `GET /api/summary` 新增 startup-failure 字段。运行 `cargo test --workspace` + `invoke testsuite-e2e`。
3. **webui web_spawn inline（commit 3）**：`sebas-webui/src/session_backend.rs` + `sebas-dispatch/src/dispatch.rs` 改为 inline；vitest 单测覆盖 spawn-failed transcript 事件。运行 `pnpm test` + `invoke testsuite-webui --case session-roundtrip`。
4. **账本与单元（commit 4）**：`tests/acceptance/COVERAGE.md` 追加本 change 索引；`tests/testsuite-webui/tests/errors.spec.ts` 增加"spawn failure inline"用例；3 连绿验收。

回滚：每笔 commit 是单一关注点（错误摘要/watchdog 终态/webui inline/账本），可独立 revert。

## Open Questions

无。spec 的两条 ADDED（webui web_spawn 失败立即内显）+ 四份 spec 的 MODIFIED 已经把规约钉死；tasks 实现按 4 笔 commit 推进即可。