# sebas 进程 · 通道 · 子命令入口总览

> **定位与维护方式**：本文回答三个问题——哪些进程、谁拉起谁（§1）；进程之间用什么
> 说话、去哪找、带什么钥匙（§2）；每个子命令从哪个入口进、委托给哪个 crate（§3）。
> **本文以代码为准**：内容为 **2026-09-09** 的代码快照，锚点一律用 `file::fn`
> （不锚行号）。行为变化时请同步更新对应章节。设计意图与行为规格查 `openspec/`
> （specs 为行为需求，changes 为变更提案）；使用说明见 `README.md`。
>
> 本文**取代**已删除的旧版架构总览（原 `docs/` 下的单文件 architecture 总览，
> 内容已过时：Gateway 旧命名、受管服务漏 im、crate 数错误、不存在的隐藏别名；
> 需要考古见 git 历史）。旧文档中仍然正确的结论
> （默认启动策略"仅 WebUI 默认启用"，§1.3）已收编于此。
>
> 关键源文件：`src/main.rs`（分发）· `src/cli.rs`（命令定义）· `src/watchdog.rs`
> （监督与派生）· `src/ipc.rs`（pipe readiness）· `src/watchdog/control_rpc.rs`
> （control RPC）· `src/core_channel/`（会话通道）· `src/run.rs`（core 编排）·
> `src/webui_cmd.rs` / `src/router_cmd.rs` / `src/im_cmd.rs`（子命令门）。

## 0. 一页心智模型：两平面 + 一根管

sebas 是**单一二进制、多子命令**：子命令决定进程角色，进程之间只有三条本地通信
路径。记住"两平面 + 一根管"就不会走错通道：

- **管理面** = control RPC（Unix socket → watchdog）：升级/回滚/重启/服务启停，
  一切"控制"走这里。
- **数据面** = core session channel（Unix socket → core）：会话快照/订阅/发消息，
  一切"会话数据"走这里。
- **readiness 管** = 继承的 stdout pipe（core 专用）：core 就绪握手，仅此一件。

```
   ┌───────── 管理面：control RPC（control.sock，→ watchdog）─────────┐
   │   cli（ctl/status/services）  webui（admin 面）  im（/命令 转发） │
   └───────────────────────────────┬─────────────────────────────────┘
                                   ▼
                        sebas run —— watchdog（唯一监督者）
                                   │ spawn / 监督 / 升级回滚（current_exe() 派生）
   ┌───────── 数据面：core session channel（core.sock，→ core）───────┐
   │   webui（SessionBackend 客户端）  im（CoreSessionPort）           │
   │   router（state 订阅 → provider/alias 热重载投影）                │
   └───────────────────────────────┬─────────────────────────────────┘
                                   ▼
                        sebas core —— 会话单一权威
                    （内嵌形态：--webui / --router 可同进程内嵌）

   readiness 管（pipe）：core ── {"cmd":"ready"} ──► watchdog
   （时序契约：core 先 bind 通道 socket + 落盘 secret，再发 ready——
     「Running」从此蕴含「已武装」，run.rs::arm_core_channel）
```

## 1. 进程：watchdog 为根的派生树

### 1.1 派生树（含默认启停）

`sebas run`（watchdog 守护，`src/watchdog.rs::run_watchdog`）是**唯一**会拉起
其他进程的角色。它以 `std::env::current_exe()` 派生同一二进制的其他子命令，
四个 `ServiceSpawner` 一一对应（`src/watchdog.rs`）：

```
            sebas run (watchdog, watchdog.rs::run_watchdog)
             ├─ sebas core ── 会话权威：sessions / adapters / channel server
             │    (CoreSpawner, SEBAS_IPC=1, stdout=pipe readiness 门)
             ├─ sebas webui ── 独立 dashboard (WebUiSpawner, 无 readiness 门)
             ├─ sebas router ── LLM 透传代理 (RouterSpawner, --debug 可选)
             └─ sebas im ── 飞书 WS 宿主 (ImSpawner, [watchdog.im] 随 feishu)
```

| 子进程 | Spawner（`src/watchdog.rs`） | argv | 注入 env | readiness 门 |
|---|---|---|---|---|
| core | `CoreSpawner` | `core --config <path>` | `SEBAS_IPC=1` · `SEBAS_CONTROL_SECRET` · `SEBAS_CORE_SECRET` | **有**：stdout 管道 `{"cmd":"ready"}`（`src/ipc.rs::ChildIpc::ready`） |
| webui | `WebUiSpawner` | `webui --config <path>` | `SEBAS_CONTROL_SECRET` · `SEBAS_CORE_SECRET` | 无：spawn 即 Running |
| router | `RouterSpawner` | `router [--debug] --config <path>` | `SEBAS_CONTROL_SECRET` · `SEBAS_CORE_SECRET` · `SEBAS_CORE_SOCKET` · `SEBAS_ROUTER_CONFIG` | 无：spawn 即 Running |
| im | `ImSpawner` | `im --config <path>` | `SEBAS_CONTROL_SECRET` · `SEBAS_CORE_SECRET` | 无：spawn 即 Running |

（webui/im 的 argv 与 env 组装在公共函数 `src/watchdog.rs::spawn_aux_process`；
core 的 stdout 管道在 ready 之后**继续排空到 EOF**——子进程未配日志文件时
stdout 就是它的输出流，读端关闭会导致 EPIPE 刷屏甚至卡死子进程，
`CoreSpawner::spawn` 内有详注。子进程在 watchdog 下把日志写 stderr
（`src/run.rs::init_tracing`），stdout 让位给 readiness 协议。）

### 1.2 内嵌形态：core 可同进程内嵌 webui / router

`sebas core` 自身就是编排点（`src/run.rs::run`）：

- `core --webui`：同进程内起 dashboard，直接用进程内的 `DualSessionBackend`
  （不经过通道 socket），入口 `sebas_webui::run_with_admin_adapter_and_auth`。
- `core --router`：在随机端口（`127.0.0.1:0`）起内置 router
  （`sebas_router::server::serve_with_listener`），实际端口写进日志。

即：**同一个 webui/router 实体代码有两种进程形态**——core 内嵌（`src/run.rs`）
与独立子进程（`src/webui_cmd.rs::run` / `src/router_cmd.rs::run`，由 watchdog
派生）。排查时先分清在跟哪种形态说话。

### 1.3 默认启动策略（收编自旧文档的正确结论）

**core 恒启动（无开关）；WebUI 默认启用；router 默认停用；im 跟随飞书。**
（serde default 与回退逻辑实测于 `src/config.rs`；enable-core-by-default）：

| 服务 | 配置键 | 默认 | 出处 |
|---|---|---|---|
| core | 无（`[watchdog.core]` 只剩 `channel_path` / `secret_file`） | **恒启动**（不可停用）；旧的 `enabled` 键被忽略并告警 | `src/watchdog.rs::run_watchdog`（`services.register_core`）+ `src/config.rs::warn_deprecated_watchdog_keys` |
| webui | `[watchdog.webui] enabled` | **开**（`src/config.rs::default_webui_enabled` → `true`；host `127.0.0.1`，port `9797`） | `src/config.rs::WatchdogWebUiConfig` |
| router | `[watchdog.router] enabled` | **关**；`sebas run --debug` 强制开（`config.router.enabled \|\| debug`） | `src/config.rs::WatchdogRouterConfig` + `src/watchdog.rs::run_watchdog` |
| im | `[watchdog.im] enabled` | **随 feishu**：显式值优先，缺省 = `cfg.feishu.is_enabled()`（显式 `[feishu] enabled`，缺省回退 app_id+app_secret 双非空） | `src/config.rs::WatchdogImConfig` / `FeishuConfig::is_enabled` + `src/main.rs`（`Cmd::Run` 分支传 `im_enabled_default`） |

期望态三层合成（`src/watchdog/services.rs`，`register` / `initial_desired`）——
**仅适用于可开关的 webui / router / im**：

```
config 默认（config.toml [watchdog.*].enabled）
    → ~/.sebas/services.json 覆盖（WebUI 服务页 / CLI ServiceSet{persist:true}）
        → 运行时 ServiceSet（未 persist 时仅本 watchdog 生命周期内有效）
```

core 例外：`register_core` 忽略 config 与 `services.json` 覆盖层，期望态恒为
`Enabled`（services.json 里的历史 `core` 覆盖被忽略并告警）。四个服务**始终注册**
进 `ServiceManager`（`src/watchdog.rs::run_watchdog`）：可开关服务初值停用时也会
出现在服务页，可随时启用。

### 1.4 生命周期与退出语义

- **ready ⟹ 已武装**：core 的 ready 打点在核心通道 bind + secret 落盘之后
  （`src/run.rs::run` 调 `arm_core_channel` 后才 `send_watchdog_ready`）；
  bind 失败（路径被存活进程占用）→ 返回 Err → 进程以 **75**（EX_TEMPFAIL）
  退出，ready 永不发出。
- **webui bind 失败**：端口被占 → 以 75 退出；监督器把退出码 75 识别为
  `Degraded`（不自动重试，等 Restart），常量 `src/watchdog.rs::EXIT_BIND_FAILED`，
  判定在 `src/watchdog/supervisor.rs`。
- **fail-fast**：任一受管服务连续 spawn 失败达 `max_spawn_failures`（默认 3）→
  `failed-startup` 终态 → watchdog 关停全部子进程并以 75 退出
  （`src/watchdog/supervisor.rs` + `src/main.rs::startup_failure_exit`）。
- **监督参数**（`src/watchdog/supervisor.rs` 常量区）：崩溃窗口 3600s、窗口内
  崩溃上限 3、超限冷却 30s、重启等待 1s、spawn 重试 5s、优雅停止宽限 5s。
- **watchdog 自身**：SIGINT/SIGTERM → `shutdown_all` 后退出（显式信号处理是
  必须的——`kill_on_drop` 在信号默认终止下不会运行，直接退出会孤儿化 core
  并让飞书 WS 双实例竞争，`src/watchdog.rs::run_watchdog` 尾部有详注）。
- **双进程部署是一等公民**：watchdog + 独立 core/webui 的形态有专门验收
  （`tests/testsuite-webui/tests/helpers/detached.ts` 及 deployment journey）。

## 2. 通道：三套通信机制对照

### 2.1 对照表

| 通道 | 方向 | 位置发现 | 认证 | 生命周期 |
|---|---|---|---|---|
| **pipe readiness**（`src/ipc.rs::ParentIpc`/`ChildIpc`） | core → watchdog 单向 | 无 socket——spawn 时继承的 stdout fd | `SEBAS_IPC=1` 环境标记（`src/ipc.rs::is_under_watchdog`） | spawn → ready → 持续排空到 EOF；控制命令**不**走管道（Ready-only 协议，`ChildMsg` 仅剩 `Ready`） |
| **control RPC**（`src/watchdog/control_rpc.rs::serve` / `::request`） | cli / webui / im / core → watchdog（管理面） | **约定路径函数**：`src/watchdog/control_rpc.rs::default_socket_path`——`$XDG_RUNTIME_DIR/sebas/control.sock`，未设时 per-uid 临时目录回退（恒以 `control.sock` 结尾）；客户端同函数解析，`--socket` / `$SEBAS_CONTROL_SOCKET` 可覆盖（`src/main.rs::run_control`） | **仅 secret 比对 + socket mode 0600，无对端 uid 校验**（`handle_envelope`：`envelope.secret != server_secret` → `unauthorized`；actor 里的 uid 只是审计元数据） | watchdog 存活期；命令面 = `RpcControlRequest` 全部 11 个变体（§2.3）；重操作异步受理（`operation_id`） |
| **core session channel**（`src/core_channel/server.rs::serve` / `::serve_bound`） | webui / im / router → core（数据面）；core 是服务端 | **注入 env / config 同源解析**：router 只认 `$SEBAS_CORE_SOCKET`（watchdog 按 `[watchdog.core] channel_path` 或缺省注入，`sebas-router/src/core_channel.rs::socket_path`）；webui / im / core 自己按同一 config 解析（`src/core_channel/server.rs::socket_path`——`channel_path` 覆盖 `$XDG_RUNTIME_DIR/sebas/core.sock` 或 per-uid 回退） | **双因子**：`SO_PEERCRED` 对端 uid 相等（先于一切读取，`server.rs::peer_uid_ok`）+ secret 握手行（`server.rs::read_handshake`，成功后服务端回 `{"handshake":"ok"}` ack）——仅此通道有 uid 校验 | core 存活期；快照先行（snapshot-then-subscribe）；会话流滞后订阅者被 drop，状态流滞后则重发快照；优雅退出删 socket 文件（secret 文件**不**删） |

两条 socket 通道的**位置发现机制不同**（排查走错通道的高发区）：
control 靠两端共用一个约定路径函数；core channel 靠 watchdog 注入 env /
各方按同一份 config 计算。**认证也不同**：control 是纯 secret 比对（本机任意
进程拿到 secret 即可），core channel 叠加内核级同 uid 校验。

### 2.2 core session channel 的自动武装（secret 不再依赖手工配置）

core 启动时**无条件武装**通道（`src/run.rs::arm_core_channel`）：

1. bind（`src/core_channel/server.rs::bind_channel_socket`：0600、僵尸 socket
   回收、活进程占用则硬失败）——**先于 ready**；
2. 解析 secret：`SEBAS_CORE_SECRET` env 优先（watchdog 注入路径不变）；缺失时
   现场生成（`src/core_channel/secret.rs::generate`，32 字节 CSPRNG hex）；
3. 两种来源都原子写入 secret 文件（`secret.rs::write_secret_file`：tmp+rename、
   0600），路径由 `src/config.rs::core_secret_file_path` 解析——
   `[watchdog.core] secret_file` 显式键优先，缺省 `<config 文件所在目录>/core.secret`；
4. 最后才发 ready。

客户端（standalone webui / im：`src/core_channel/client.rs::CoreChannelBackend::with_secret`；
router 订阅：`sebas-router/src/core_channel.rs::channel_secret`）按
**env 优先 → secret 文件**顺序发现密钥，且**每次连接前重读文件**——core 重启
换钥后，客户端靠重连退避天然自愈，无需通知通道。被拒的 secret 不会原样无限
重试：下一次尝试重新解析（`client.rs::handshake`）。两者皆缺 → warn 一次 +
空 secret 尝试，握手被拒、如实上报，不静默。

### 2.3 各通道细节

**pipe readiness（§1.1 已述）**：协议只剩 `{"cmd":"ready"}`（`src/ipc.rs`）。
控制操作一律走 control RPC。

**control RPC**：命令面 = `RpcControlRequest` 的 11 个变体
（`src/watchdog/control_rpc.rs`）：

| 变体 | 语义 |
|---|---|
| `Status` | 控制面状态快照（附带最近启动失败摘要） |
| `EventsSince { seq }` | 按 seq 拉控制事件 |
| `Update { dev, dry_run }` | 升级（dev 构建 / 仅计划） |
| `Rollback { dry_run }` | 回滚 |
| `RestartCore` | 重启 core 子进程（升级/回滚语义） |
| `ServiceStatus` | 受管服务状态快照（4 个服务） |
| `ServiceStatusFor { service }` | 单服务查询（/router status、/webui status） |
| `ServiceSet { service, desired, persist }` | 期望态 on/off；core 的启停只接受 CLI/WebUI actor（飞书 actor 拒绝——core 停止后确认卡片无法送达） |
| `ServiceRestart { service }` | 重启单个受管服务（core 除外） |
| `Confirm { token }` / `Cancel { token }` | 危险操作的确认/取消（仅 Feishu actor；动作真值只在 watchdog 的 pending 注册表里） |

actor 两类（`RpcActor`）：`Cli { uid }` 与 `Feishu { open_id, chat_id }`（后者由
core 以启动密钥做签名断言代理提交）。CLI 暴露其中 6 个动词
（`status/events/update/rollback/restart-core/services`，`src/cli.rs::ControlCmd`），
其余变体由 webui admin 面（`src/webui_cmd.rs::control_admin_adapter`）与 im
（`src/im_cmd.rs::WatchdogControl`）提交。

**core session channel 请求面**（`src/core_channel/protocol.rs::CoreChannelRequest`）：
一次性请求（Snapshot / Spawn / CreatePlaceholder / Message / EnsureMessage /
Cancel / Close / Turns / SetFocus / Focused / SetSessionModel / ApprovalAnswer /
StateSnapshot / StateMutation）+ 两条持久流（`Subscribe` 会话流、
`StateSubscribe` 状态流）。流的语义：先发一帧全量快照，再持续推增量；
**会话流**滞后（>256 帧）或写超时 → 断开连接、客户端重连重快照
（`server.rs::serve_subscription`）；**状态流**滞后 → 语义等价全域变更、服务端
直接重发快照（`server.rs::serve_state_subscription`）。

**不可达分类**（`src/core_channel/client.rs::reachability`）：通道客户端把失败
闩锁为三种机器可读 kind——`startup_failed`（socket 不存在）、`auth_rejected`
（握手被拒，secret 不匹配）、`disconnected`（连接被拒/中途断开/超时）——连同
人类可读 cause 一并出现在 `/api/summary` 的 `reachability` 里；若
`SEBAS_STARTUP_ERROR_FILE` 有 core 最近一次启动失败摘要，cause 会被富化为
`core startup failed: …`（`client.rs::enrich_with_startup_summary`）。webui 据此
渲染全局不可达横幅并把项目注册降级。

### 2.4 钥匙是谁发的、谁持有

| 密钥/标记 | 谁生成 | 谁持有 | 怎么传 | 落盘？ |
|---|---|---|---|---|
| `SEBAS_CONTROL_SECRET` | watchdog（`src/watchdog.rs::create_control_secret`，pid+时间戳） | watchdog 内存 + 四个子进程 env（core/webui/router/im 都注入） | 信封 `secret` 字段 | **不落盘**（重启即换，外部 CLI 需自行提供 `--secret`/env） |
| `SEBAS_CORE_SECRET` | watchdog（同上函数）注入四子进程；无 env 的裸 core 自己 `secret.rs::generate` | core（服务端）+ webui/router/im（客户端）；迟启动客户端经 secret 文件发现 | 握手行 `ChannelHandshake`（`src/core_channel/protocol.rs`） | **落盘**：`core.secret`（0600，原子写）；core 优雅退出**不删**——socket 文件才是"core 死了"的权威信号，残留 secret 无害（socket 不在走不到握手） |
| `SEBAS_IPC` | 硬编码 `"1"`（`src/watchdog.rs::CoreSpawner`） | 仅 core 子进程 | env 标记 | — |
| `SEBAS_CORE_SOCKET` | watchdog（`run_watchdog` 按 config 解析后注入） | 仅 router 子进程消费（webui/im/core 按 config 自算同一路径） | env 注入 | — |
| `SEBAS_ROUTER_CONFIG` | watchdog（与 `--config` 同值） | 仅 router 子进程消费（secret 文件发现的锚点） | env 注入 | — |

## 3. 子命令入口：三类分发表

`src/cli.rs::Cmd` 的全部变体按运行类别对号入座（注意：**枚举声明顺序与运行类别
不一致**——`Router` 在 `WebUi` 之前、`Run` 在 `Im` 之前、一次性命令穿插其间，
勿按行序理解）。分发全部在 `src/main.rs::main` 的 `match` 里。

### 3.1 常驻服务

| 子命令 | src/ 入口 | 委托 crate 实体（门后一跳） |
|---|---|---|
| `core` | `src/main.rs`（`Cmd::Core`）→ `src/run.rs::run` | 内嵌 `--router`：`sebas_router::server::serve_with_listener`；内嵌 `--webui`：`sebas_webui::run_with_admin_adapter_and_auth`；通道自动武装：`src/run.rs::arm_core_channel` → `src/core_channel/server.rs::serve_bound`；会话/适配器装配都在本函数内（ACP 驱动来自 `sebas_acp`，原生内核来自 `sebas_agent`，分发引擎来自 `sebas_dispatch`） |
| `webui` | `src/main.rs`（`Cmd::WebUi`）→ `src/webui_cmd.rs::run` | `sebas_webui::run_with_admin_adapter_and_auth`（与 core 内嵌形态同一函数）；会话数据经 `src/core_channel/client.rs::CoreChannelBackend`（纯客户端）；admin 面经 control RPC（`src/webui_cmd.rs::control_admin_adapter`） |
| `router` | `src/main.rs`（`Cmd::Router`）→ `src/router_cmd.rs::run` | `sebas_router::server::run`（HTTP/透传/admin 面）；状态订阅在 router crate 自带的客户端 `sebas-router/src/core_channel.rs::spawn_subscriber`（手工握手 + `StateSubscribe`，**不在** `src/core_channel/client.rs` 里）；`--debug` 注入 test provider（`sebas_router::debug::enable_debug_test_provider`） |
| `im` | `src/main.rs`（`Cmd::Im`）→ `src/im_cmd.rs::run` | `sebas_im::bootstrap::bootstrap`（飞书 WS/token/问候装配）；会话操作经 `src/core_channel/client.rs`（适配成 `sebas_im::port::CoreSessionPort`）；控制命令直发 control RPC（`src/im_cmd.rs::WatchdogControl`）；卡片配置形状来自 `sebas_feishu::cards` |
| `run` | `src/main.rs`（`Cmd::Run`）→ `src/watchdog.rs::run_watchdog` | 无 crate 委托——watchdog 本体（监督/派生/升级回滚/control RPC 服务端）都在根 crate `src/watchdog/` |

### 3.2 一次性命令

| 子命令 | src/ 入口 | 说明 |
|---|---|---|
| `service` | `src/main.rs` → `src/service.rs::run_install` / `run_uninstall` | 安装/卸载 systemd 单元；ExecStart 烘焙 `sebas::RUN_SUBCOMMAND`（即 `sebas run`） |
| `replay` | `src/main.rs` → `src/replay.rs::run` | 按目录里的 `.json` 事件回放 |
| `record` | `src/main.rs` → `src/record.rs::run` | 录制 ACP agent stdio 流量为 fixture |
| `update` | `src/main.rs` → `src/update.rs::run` | watchdog 的一次性升级/回滚实现 |
| `agent-kinds list` | `src/main.rs` → `src/agent_kinds.rs::run` | 配置的第三方 agent 可达性报告 |
| `webui-passwd` | `src/main.rs` → `src/webui_cmd.rs::run_passwd` | 建/改 WebUI 登录账户（PBKDF2 落盘，运行中的 webui mtime 热重载） |

### 3.3 控制面

| 子命令 | src/ 入口 | 说明 |
|---|---|---|
| `control <子命令>` | `src/main.rs::run_control` | 统一信封构造 → `control_rpc::request`；子命令 6 个（`src/cli.rs::ControlCmd`）：`status`/`events`/`update`/`rollback`/`restart-core`/`services`；socket 解析 `--socket` > `$SEBAS_CONTROL_SOCKET` > `default_socket_path`；secret `--secret` > `$SEBAS_CONTROL_SECRET`（缺失即报错——watchdog 故意不落盘） |
| `ctl` | `src/main.rs`（`Cmd::Ctl`）→ `run_control` | `sebas control` 的别名（clap 独立变体，非隐藏 alias） |
| `status` / `services` | `src/main.rs::run_control_status` | 分别等价 `control status` / `control services`（复用 `run_control`） |

### 3.4 SUBCOMMAND 常量现状（如实标注）

三个常量在 `src/lib.rs`，现状参差：

- `CORE_SUBCOMMAND`（="core"）：`CoreSpawner` 使用，且有 clap 名同步测试
  （`src/main.rs::tests`，`run_subcommand_name_matches_core_subcommand_const`）。
- `RUN_SUBCOMMAND`（="run"）：仅 `src/service.rs`（systemd ExecStart）使用，无测试。
- `ROUTER_SUBCOMMAND`（="router"）：**死常量**——`RouterSpawner` 的 argv 构造处
  硬编码 `"router"` 字面量（`src/watchdog.rs`；`WebUiSpawner`/`ImSpawner` 同样
  硬编码 `"webui"`/`"im"`）。改子命令名时这里没有常量保护。

另注：`src/lib.rs` 中 `RUN_SUBCOMMAND`/`ROUTER_SUBCOMMAND` 的 doc 注释曾声称
存在 `watchdog` / `gateway` 隐藏 clap 别名，与 `src/cli.rs` 不符（remove-gateway-residue
已修正该注释；`src/cli.rs` 无任何隐藏别名，本文以 `cli.rs` 为准）。

### 3.5 workspace crate 速查

`src/` 里的 `*_cmd.rs` 只是进程的门；实体多在门后一跳的 crate 里。10 个成员
（`Cargo.toml` `members`）：

| crate | 一句话职责 |
|---|---|
| `sebas`（根） | 二进制与进程入口：watchdog 派生/监督（`src/watchdog/`）、三通道实现（`src/ipc.rs`、`src/watchdog/control_rpc.rs`、`src/core_channel/`）、core 编排（`src/run.rs`）、各子命令门（`src/*_cmd.rs`） |
| `sebas-router` | LLM provider 透传代理：Anthropic/OpenAI 双协议、admin 面、热重载、自带 core channel 状态订阅客户端 |
| `sebas-webui` | WebUI dashboard 服务端：axum 路由/SSE/登录鉴权；会话读写全部走 `SessionBackend` seam（进程内或通道客户端），不依赖根 crate |
| `sebas-im` | IM 服务层：飞书装配入口（`bootstrap`）与交互前端/端口抽象（`frontend`/`port`），经核心会话通道驱动会话 |
| `sebas-feishu` | 飞书接入：WS 事件、卡片渲染（`sebas_feishu::cards`）、出站 API 客户端 |
| `sebas-acp` | ACP agent 驱动：claude 专用驱动 + 通用 ACP 驱动（`SessionManager`/`AgentDriver`） |
| `sebas-agent` | 原生 in-process coding agent 内核：LLM 客户端、工具、权限策略、turn 状态机 |
| `sebas-dispatch` | core 内会话分发领域层：`DispatchHandle`、状态库（`state_store`）、卡片状态机/命令/表单 |
| `sebas-channels` | 通道抽象：`ChannelAdapter` trait + `AdapterRegistry`（core 只依赖这里的类型） |
| `sebas-ipc` | 跨平台本地 IPC 传输原语（`IpcListener`/`IpcStream`；Unix socket / Windows named pipe），三通道共用 |
| `xtask` | 构建期工具（模型表更新、文档检查） |

## 4. 排查指路（走错通道的三种典型症状）

- **改了 provider/服务不生效、`ctl` 报 socket not found** → 你要找的是管理面：
  watchdog 在跑吗？`$XDG_RUNTIME_DIR` 两端一致吗？（§2.1 control 行）
- **webui 页面显示 unreachable / 项目注册降级** → 数据面断了：看
  `/api/summary` 的 `reachability.kind`（§2.3）——`startup_failed` 是 core 没起来，
  `auth_rejected` 是密钥不匹配（查 secret 文件与 env），`disconnected` 是运行中断连。
- **手动起 webui/router 调试却连不上 core** → 别忘了 core channel 的位置是
  config 同源解析或 `SEBAS_CORE_SOCKET` 注入，不是写死的默认路径；secret 按
  env → `<config 目录>/core.secret` 发现（§2.2）。
