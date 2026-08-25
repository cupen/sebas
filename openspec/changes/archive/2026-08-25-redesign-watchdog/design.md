# Design: 重新设计 watchdog 监督模型

## Context

现状关键事实（调查结论，见 proposal.md 的动机）：

- 控制面已有"正牌"执行路径 `ControlExecutor`（executor.rs），但 `Watchdog::run_update()`
  保留了一条绕过它的管道 IPC 重复路径；子进程侧（dispatch.rs）实际已全部改走 RPC
  socket，管道里的 `upgrade`/`upgrade-dev`/`rollback` 命令不可达。
- core 是唯一被真正监督的子进程；webui / debug-gateway 子进程 spawn 后只在退出时
  被回收，中间崩溃无人重启；`service_status()` 对 webui/gateway/feishu 硬编码
  `"running"`。
- auth.rs 581 行中约 550 行（Verifier/MacProvider/SignedAssertion/AssertionBuilder/
  ActorVerifier/VerifiedActor/RejectionReason）零调用方；updater.rs 的
  UpdateSignal/ControlPlaneImpact/classify_readiness_failure/recommended_recovery
  四组策略函数零调用方（监督循环用内联 if 重写了同一逻辑）。
- 配置 `watchdog.upgrade.max_retries/retry_delay_secs/check_on_start` 无消费者
  （sebas-ivg）。
- watchdog 模式下 core 不带 `--gateway` 启动，生产 gateway 完全不在监管内
  （sebas-08c 待迁）；生产/裸模式的 gateway 与 webui 以 in-process tokio task 形态
  活在 `sebas run` 里。

## Goals / Non-Goals

**Goals:**

- 一条监督循环管所有子进程；一条执行路径管所有控制操作；服务状态说真话。
- 删除全部已确认的死代码路径，让"executor.rs 文档头声称的不变量"名副其实。
- 兼容性：不 opt-in 就无新进程；现有 RPC 客户端（dispatch、`sebas control`、
  webui）协议面只增不改（ServiceSet/ServiceRestart 从拒绝变执行）。

**Non-Goals:**

- 不改 core 内部（ACP/router/会话）；不改确认流与鉴权模型；不做时间线持久化。
- 裸 core 模式（`sebas run [--gateway] [--webui]` 的 in-process 形态）保持原样，
  不迁入服务表——服务表只在 watchdog 模式存在。

## Decisions

### D1: 每服务一个监督 task，而不是一个大 select 循环

`ServiceManager` 持有一张 `HashMap<ServiceName, ServiceHandle>`；每个受管服务一个
tokio task，独占持有自己的 `Child`、崩溃计数器、期望状态。task 内部 select 三路
事件：子进程退出、命令通道（start/stop/restart/set_desired）、watchdog 关停信号。
`ServiceHandle` 对外暴露命令 mpsc + 状态快照（`watch::channel` 每 service 一个，
或共享 `Arc<Mutex<Snapshot>>`；选后者，状态量小、无高频读）。

- 备选：单一 `select!` 大循环（今天的形态泛化）。否决：所有服务的退出/重启/退避
  状态挤在一个结构里，core 特有逻辑（readiness + 自动回滚）会把泛型分支污染回去；
  每 service 一个 task 让崩溃策略状态天然私有、无锁。
- `child.wait()` 的取消安全：监督 task 内不跨 await 点保持 `&mut child` 之外的
  借用；wait 与命令通道同层 select，取消后重入 wait 即可（tokio Child::wait 可重入）。

### D2: core = 带 readiness 门的服务；自动回滚是 core 监督 task 的钩子

监督 task 泛型化：spawn spec + 可选 readiness 阶段。core 的 readiness 阶段 =
读管道直到 `{"cmd":"ready"}`；`just_performed_update`/`received_ready` 从
`Watchdog` 结构体字段移进 core 监督 task 的局部状态，退出分类直接调用
updater.rs 现成的 `classify_readiness_failure()`（把死策略函数接线，删掉内联
重复实现）。自动回滚逻辑整体迁入 core task；`UpdateSignal`/`ControlPlaneImpact`/
`classify_update_impact`/`recommended_recovery` 若接线后仍无调用方则删除
（当前判定：删，update_signal_message 一并删）。

`Watchdog` 结构体随之消解：`run_watchdog()` 变成纯装配函数（ServiceManager +
RPC server + 各服务 task + 收尸），watchdog.rs 预计从 722 行缩到 ~200 行。

### D3: 管道协议收缩为 Ready 单行

`ChildMsg` 只剩 `Ready`；`ParentMsg` 与子进程侧的常驻监听 loop（`init_watchdog_ipc`
的 ok/error/done 分支）、`send_watchdog_command`、`WATCHDOG_TX`、`ParentIpc` 的
`ok/error/done` 助手全部删除。子进程侧只剩：启动完成时写一行 `ready` 到 stdout。
子进程存活检测改为监督 task 直接 `child.wait()`（不再依赖管道 EOF）。stderr 保持
inherit，早期致命错误靠日志，不走管道。

- 备选：保留管道做双向错误通道。否决：无消费者（升级进度已走 RPC 事件流），
  留着就是下次歧义的种子。

### D4: executor 持有 ServiceManager 句柄，PostAction 通道退役

`ControlExecutor` 新增字段 `services: ServiceManager`（clone 句柄）。`plan_for`
增加 `Execution::ServiceAction` 分支：ServiceSet/ServiceRestart 翻译成对
ServiceManager 的命令，立即 settle（服务操作是秒级，无需 detached runner）。
`RestartCore` 从 `restart_tx` mpsc 改为 `services.restart(Core, is_upgrade)`；
`restart_rx` 及其在 `handle_ipc` 里的 select 分支删除。
`submit_blocking`（IPC 路径专用）随 run_update 一起删除——RPC 是唯一入口。

ServiceSet 命名 core → `Rejected`，message 指向 RestartCore（spec delta 已定）。
ServiceRestart(webui/gateway) 非危险操作，Feishu actor 直接执行，不进确认流
（危险名单维持 Update/Rollback/RestartCore 不变）。

### D5: 期望状态三层合一：config 默认 → services.json 覆盖 → 运行时 ServiceSet

- 初始 desired = config（`[watchdog.webui] enabled`、新增 `[watchdog.gateway]
  enabled`，均默认 false）。
- `data_dir` 之外新文件 `~/.sebas/services.json`（`{"webui": "off"}` 形态），
  启动时覆盖 config 默认——只有 `persist: true` 的 ServiceSet 会写它。
- 运行时 ServiceSet 改内存态；`persist: false` 的改动 watchdog 重启后回到
  config+file 的合成值。
- config 关闭 + 无覆盖 → 状态 `disabled`，不 spawn；config 关闭 + persist 覆盖
  on → 正常受管（config 只是初值，运行时状态是权威——统一模型，拒绝"config 关了
  就永久锁死"的特例）。
- 不选写回用户 TOML：重写用户配置文件侵入性太强且和注释/格式打架。

### D6: ServiceStatus 数据源 = ServiceManager 快照，删掉 feishu 假行

`executor.service_status()` 改读快照：core/webui/gateway 来自各监督 task 的
真实状态（running / restarting / stopped / disabled，含 pid）；watchdog 自身行
保留（真值）；updater 行保留 `running_exclusive` 语义（真值）；**feishu 行删除**
（它是 core 进程内的一个模块，不是受管服务；`/services` 输出少一行，符合
"状态说真话"）。gateway 的 debug 子进程并入 gateway 服务条目（debug 模式下
desired 来自 `--debug` 而非 config）。

### D7: auth.rs 删减至两个转换函数

保留 `FeishuPrincipal`/`WebUiPrincipal`/`AssertionPrincipal` +
`actor_to_principal`/`principal_to_actor`（executor/confirmation 在用）。删
Verifier/MacProvider/DefaultMacProvider/SignedAssertion/AssertionBuilder/
ActorVerifier/VerifierConfig/VerifiedActor/RejectionReason 及其测试
（≈550 行）。鉴权模型不变：socket 权限 + 启动 secret + envelope actor。

### D8: 死配置字段移除，未知字段告警不报错

`watchdog.upgrade` 删 `max_retries`/`retry_delay_secs`/`check_on_start` 三个
字段及 default 函数；`updater_timeout()` 只看 `updater_timeout_secs`/
`dev_build_timeout_secs`（现状即如此，守卫测试保留）。旧配置带这些字段 →
serde 反序列化忽略 + 启动 warn 一行提示字段已废弃，**不**失败（部署里真有人
写着这些字段）。注意 serde 默认忽略未知字段，warn 需要 `deny_unknown_fields`
之外的手段：解析时用 `toml::Value` 先扫一层做 diff，仅在 watchdog 段存在废弃
键时 warn。

### D9: 模块落点

```
src/watchdog.rs            run_watchdog() 装配 + 收尸（瘦身后 ~200 行）
src/watchdog/supervisor.rs 新：监督 task（泛型 spawn + readiness 钩子 + 退避）+ ServiceName
src/watchdog/services.rs   ServiceManager：句柄表、快照、命令、services.json 读写
src/watchdog/{control,executor,control_rpc,confirmation,events,updater}.rs  结构基本不动
src/watchdog/auth.rs       删减（见 D7）
src/ipc.rs                 协议收缩（见 D3）；ParentIpc 或并入 supervisor.rs 后删除
```

## Risks / Trade-offs

- [监督 task 内 child.wait() 的 select 取消语义出错 → 卡死或漏检退出] →
  每服务 task 的 wait 分支成功后同步收割 `child.wait()` 结果再走分类；为
  `SupervisedService` 写"崩溃→重启→再崩溃"与"stop 命令 vs 意外退出"的单元
  测试（FakeChild 注入）。
- [ServiceSet 与监督 task 并发竞争（stop 中又来 restart）] → 命令进 task 私有
  mpsc 串行处理，天然无竞争；快照与命令的先后由通道顺序保证。
- [gateway opt-in 后与裸模式 `run --gateway` 端口冲突（同机同端口）] → 文档
  明确二者互斥；gateway 服务 spawn 失败按监督退避重试，状态如实显示
  restarting。
- [删 StopCore/StartCore/submit_blocking 等"未来可能有用"的接口] → git 历史可
  找回；spec 未承诺这些接口，留半成品接口的代价（每个读者都要弄清它通不通）
  高于找回成本。
- [services.json 与 state.json 并存，用户困惑] → 文件头注释 + `/services` 输出
  里 persist 状态可见。

## Migration Plan

1. 单二进制常规升级路径（本变更自身走 `sebas update` dev 流程）。
2. 升级后首次启动：无新配置 → 行为等同旧版（webui 维持 config 决定，gateway
   仍不启动）；旧死字段仅 warn。
3. opt-in 顺序：先 `[watchdog.gateway] enabled = true` 观察服务表与真实状态，
   再按需 ServiceSet persist。
4. 回滚：二进制级回滚（`versions/rollback`）即回到旧监督模型；`services.json`
   对旧版无意义但不干扰。

## Open Questions

- `RpcServiceStatus.uptime_secs` 是否顺手填真值（监督 task 记 spawn Instant，
  成本极低）——实现时定，不影响契约（字段已存在、当前恒 None）。
- webui/gateway 是否加端口级 readiness 探测（当前 liveness 即 running）——留待
  后续，接口上不阻塞。
