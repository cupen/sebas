# sebas 架构总览

> **定位与维护方式**（贡献者视角）：本文是 sebas 进程结构与通信语义的单一事实来源，
> 回答"哪些进程、谁拉起谁、控制走哪条通道、core 为什么默认不启动"。
> 内容以 **2026-08 的代码实现快照**为准；关键结论均标注来源文件（`文件路径 + 模块名`），
> 便于日后改动代码时逐一核对。行为变化时请同步更新对应章节与标注。
> 不含未来计划或未实现设想——设计意图请查 `openspec/`（specs 为行为规格，
> changes 为变更提案）；面向使用者的安装与使用说明见 `README.md`。

## 1. 进程全景

**单一 `sebas` 二进制，子命令决定进程角色。** 不存在第二个常驻二进制：
run(watchdog 守护)、core、webui、router 都是同一个可执行文件在不同命令行下的不同"人格"。
子命令与 `src/cli.rs` 中 `Cmd` 枚举一一对应（枚举定义：`src/cli.rs`）：

```
                        ┌─────────────────────────────────────────────┐
                        │              sebas（单一二进制）              │
                        └─────────────────────────────────────────────┘
                                              │ 子命令决定角色（src/cli.rs::Cmd）
   ┌──────────────┬───────────────┬──────────┼──────────────┬─────────────┐
   │              │               │          │              │             │
┌──▼───┐      ┌───▼───┐      ┌────▼───┐  ┌───▼────┐  ┌─────▼─────┐  ┌────▼────┐
│ core │      │  run   │      │ webui  │  │ router │  │  ctl /    │  │ record/ │
│      │      │        │      │        │  │        │  │  status/  │  │ replay/ │
│      │      │        │      │        │  │        │  │  services │  │ update/ │
└──┬───┘      └───┬───┘      └───┬────┘  └───┬────┘  └─────┬─────┘  └────┬────┘
   │              │              │           │             │             │
core 进程      watchdog 守护   独立 WebUI   独立模型路由  控制 CLI      一次性工具
(飞书 bot     （拉起并监督    （连 core     （provider  （经 control  （fixture 录制/
 + ACP +      core/webui/    socket 或     透传代理）   socket 与     回放/系统服务
 通道服务）    router 子进程) 进程内后端                 watchdog 对话)  安装/升级实现）
```

各角色一句话：

| 子命令 | 进程角色 | 说明 |
|---|---|---|
| `sebas core` | **core 进程** | 长驻服务本体：飞书 WebSocket 长连接 + ACP 会话驱动 + core session channel socket。`--webui` 可顺带在进程内起 WebUI（`InProcessBackend`），`--router` 可顺带起内置 router（`src/run.rs`） |
| `sebas run` | **watchdog 守护** | 父进程：按配置拉起 core / webui / router 子进程并监督（重启/退避/升级/回滚）（`src/watchdog.rs`；旧名 `watchdog` 仍为隐藏别名） |
| `sebas webui` | **独立 WebUI** | 独立的 dashboard 进程，经 core session channel socket 观察与驱动会话（`src/webui_cmd.rs`）。由 watchdog 拉起，或手动指向同一 socket |
| `sebas router` | **独立模型路由** | provider 透传代理独立进程（`src/router_cmd.rs`；旧名 `sebas gateway` 仍为隐藏别名） |
| `sebas ctl` / `status` / `services` | **控制 CLI** | 向 watchdog 控制面发 RPC 的薄客户端（`src/main.rs` 中 `Cmd::Ctl/Status/Services` → `run_control*`） |
| `sebas record` / `replay` | **一次性工具** | ACP stdio 流量录制为 fixture / 按 fixture 回放（`src/record.rs`、`src/replay.rs`，规格见 `openspec/specs/replay-debug/spec.md`） |
| `sebas update` | **一次性工具** | watchdog 调用的一次性升级实现（`src/update.rs`、`src/upgrade.rs`） |
| `sebas service` | **一次性工具** | 安装/卸载 systemd 系统单元（`src/service.rs`） |

来源标注：进程拓扑对照 `src/cli.rs`（`Cmd` 枚举分支）与 `src/main.rs`（各分支的 `run_*` 分发）。

## 2. watchdog 监督模型

`sebas run`（watchdog 守护）是唯一会拉起其他进程的角色。它管理三个受管服务
（`ServiceName::Core / WebUi / Gateway`，来源：`src/watchdog/supervisor.rs`）：

- **core**：飞书 bot + ACP 会话驱动 + core session channel socket。
  是"会话状态的单一权威"——standalone WebUI 的一切会话读写都经 socket 到 core。
- **webui**：dashboard 进程。自身不持有会话状态。
- **router**：provider 透传代理（模型路由）。

### 2.1 生命周期状态机

每个受管服务由一个监督 task 全权负责（来源：`src/watchdog/supervisor.rs` 模块注释：
"每服务一个监督 task：spawn、readiness 门、崩溃退避、命令处理"）。
观测状态机（`ServiceState`，来源：`src/watchdog/supervisor.rs`）：

```
              ┌──────────┐  spawn 成功
   Disabled ──►  Starting │────────────┐   readiness 门：
   (配置层未启用)└──────────┘            │    core 等管道 {"cmd":"ready"}
                    ▲                   ▼    webui/router 无门：spawn 即 Running
                    │              ┌─────────┐
        ServiceSet/ │              │ Running │
        Restart 复位│              └────┬────┘
                    │            child 退出│(非 Stop)
                    │                   ▼
   ┌──────────┐  spawn 失败    ┌────────────┐  1s 后重启
   │Degraded  │◄───────────────│ Restarting │──────────► Starting
   │(bind 等外 │  (不自动重试，  └────────────┘
   │ 部原因)  │   等 Restart)        │ 1h 窗口内崩溃 >3 次
   └──────────┘                      ▼
        ▲                        冷却 30s（计数重置后继续监督）
        │                        （watchdog 绝不因 child 崩溃而退出）
   Stopped ◄──── ServiceSet off / Stop：child 已停，不再重启
```

关键参数（来源：`src/watchdog/supervisor.rs` 常量区）：

| 常量 | 值 | 语义 |
|---|---|---|
| `CRASH_WINDOW` | 3600s | 崩溃计数窗口，超窗未崩则计数重置 |
| `MAX_CRASHES` | 3 | 窗口内连续崩溃上限，超过进入冷却 |
| `OVER_LIMIT_COOLDOWN` | 30s | 超限冷却；冷却后重置计数继续监督（watchdog 不退出） |
| `RESTART_DELAY` | 1s | 崩溃后重启前的固定等待 |
| `SPAWN_RETRY_DELAY` | 5s | spawn 失败（缺二进制等）后的重试等待 |
| `STOP_GRACE` | 5s | 优雅停止宽限：SIGTERM → 宽限 → SIGKILL |

注意：`ServiceRestart` 命令触发的重启不计入崩溃计数
（来源：`src/watchdog/supervisor.rs`，`register_crash` 注释）。

### 2.2 默认启动策略

**仅 WebUI 默认启用；core 与 router 默认停用。**
（来源：`src/config.rs` `WatchdogCoreConfig` / `WatchdogWebUiConfig` /
`WatchdogGatewayConfig` 的 doc 注释与 `default_webui_enabled()`）

- `watchdog.webui.enabled` 默认 `true`——watchdog 唯一默认启动的服务。
- `watchdog.core.enabled` 默认 `false`——feishu 是可选项（不配 app_id/secret 即不接飞书），
  需要时在 WebUI 服务页启用，或配置里显式 `enabled = true`。
- `[watchdog.router] enabled` 默认 `false`——生产 router 的默认形态仍是
  `sebas core --router` 的进程内模式；显式开启后才由 watchdog 作为受管子进程监督
  （配置节旧名 `[watchdog.gateway]` 仍被兼容解析并告警）。

期望态的三层合成（来源：`src/watchdog/services.rs`，模块注释 "三层（design.md D5）"）：

```
config 默认（config.toml [watchdog.*].enabled）
    → ~/.sebas/services.json 覆盖（WebUI 服务页/CLI 写入的持久层）
        → 运行时 ServiceSet（未 persist 时仅本进程生命周期内有效）
```

WebUI 服务页或 CLI 设置 `ServiceSet { persist: true }` 时同步写 `services.json`
（`ServiceName` ∈ {core, router, webui} × `DesiredState` ∈ {enabled, disabled}），
下次 watchdog 启动时经 `initial_desired()` 读回
（来源：`src/watchdog/services.rs::initial_desired` / `persist` 写入路径）。

## 3. IPC 语义

两条通道，职责严格分离（来源：`src/ipc.rs` 模块注释）：

### 3.1 管道 = 仅 readiness 握手

watchdog（父）与 core（子）之间的管道**只承载一件事**：core 完成启动后向 stdout
写一行 `{"cmd":"ready"}`（`ChildMsg::Ready`，来源：`src/ipc.rs`），父进程读到即把
该服务从 `Starting` 翻到 `Running`。管道不承载任何控制操作，也不承载生命周期语义
之外的信息（子进程存活检测由父进程 `child.wait()` 承担）。
webui / router 无 readiness 概念：spawn 即视为 Running
（来源：`src/watchdog/supervisor.rs`，"无 readiness 门的进程（webui/router）：spawn 即 Running"）

### 3.2 控制 = Unix socket control RPC

一切控制操作走 control RPC Unix socket（默认 `$XDG_RUNTIME_DIR/sebas/control.sock`，
回退 per-uid 临时目录；来源：`src/watchdog/control_rpc.rs::default_socket_path`）。
CLI（`sebas ctl/status/services`）与 WebUI 服务页共用同一通道、同一组操作。

**RPC 操作列表**与 `RpcControlRequest` 枚举成员完全一致
（来源：`src/watchdog/control_rpc.rs::RpcControlRequest`，serde `tag = "type"`）：

| 操作 | 语义 |
|---|---|
| `Status` | 控制面状态快照 |
| `EventsSince { seq }` | 按 seq 打印控制事件 |
| `Update { dev, dry_run }` | 升级操作（dev 构建目标 / 仅计划） |
| `Rollback { dry_run }` | 回滚操作（仅计划） |
| `RestartCore` | 重启 core 子进程 |
| `ServiceStatus` | 受管服务状态快照 |
| `ServiceStatusFor { service }` | 单服务状态查询（`/router status`、`/webui status`） |
| `ServiceSet { service, desired, persist }` | 设置服务期望态（on/off；core 仅接受 CLI/WebUI actor） |
| `ServiceRestart { service }` | 重启单个受管服务 |
| `Confirm { token }` | 用不透明 token 确认危险操作（token 对应动作只在 watchdog 的 pending 注册表里） |
| `Cancel { token }` | 取消 pending 危险操作并记录 Canceled 事件 |

操作者（actor）分两类（`RpcActor`）：`Cli { uid }` 与 `Feishu { open_id, chat_id }`——
后者由 core 以启动密钥做签名断言代理提交；core 的启停只接受 CLI/WebUI actor
（来源：`src/watchdog/control_rpc.rs::RpcActor` 与 `ServiceSet` 字段注释）。

### 3.3 core session channel（会话数据面）

控制面之外，core 还开一条**会话数据面** Unix socket
（默认 `$XDG_RUNTIME_DIR/sebas/core.sock`，可用 `[watchdog.core] channel_path` 覆盖；
来源：`src/core_channel/server.rs::default_socket_path` / `src/config.rs::WatchdogCoreConfig`）：
standalone WebUI 经它做快照/订阅/创建/发消息/关闭/取转写，是"core 是会话单一权威"的落地。
安全：socket 0600 + `SO_PEERCRED` uid 相等校验 + `SEBAS_CORE_SECRET` 握手行
（来源：`src/core_channel/server.rs`，peer_uid_ok / read_handshake）。
与 control RPC 的分工：**控制走 control.sock，会话走 core.sock**。

## 4. CLI 控制面与 crate 职责速查

### 4.1 CLI 控制面

（来源：`src/cli.rs::ControlCmd` 与 `src/main.rs` 分发）

- `sebas control <子命令>`（别名 `sebas ctl`）：向 watchdog 控制面发 RPC。
  子命令：`status` / `events` / `update` / `rollback` / `restart-core` /
  `services`（来源：`src/cli.rs::ControlCmd`）。
- `sebas status`：`control status` 的快捷方式。
- `sebas services`：`control services` 的快捷方式（受管服务状态快照）。

### 4.2 crate 职责速查

workspace 成员与 `Cargo.toml` 的 `members` 一致
（来源：`Cargo.toml`）——根 crate 加 5 个成员 crate，另有 `xtask` 构建工具：

| crate | 一句话职责 |
|---|---|
| `sebas`（根） | 二进制装配与各子命令入口：core / run / webui / router / ctl / record / replay / update / service；出站分发与 session 装配 |
| `sebas-router` | 会话路由核心：SessionMap/FSM、卡片状态机、命令解析、CRUD 表单、会话事件广播（webui 与通道的数据源） |
| `sebas-feishu` | 飞书接入：WebSocket 事件、卡片渲染、出站 API 客户端（send_card / react / 话题感知发送） |
| `sebas-acp` | ACP 引擎适配：claude stream-json + 控制协议子进程驱动（spawn / prompt / 事件映射 / 崩溃与挂起看护） |
| `sebas-router`（原 sebas-gateway） | 模型路由：LLM provider 透传代理，Anthropic/OpenAI 双协议嗅探、路由、鉴权、用量统计 |
| `sebas-dispatch`（原 sebas-router） | core 进程内的会话分发领域层：会话映射、入站 dispatch、slash 命令、权限处理 |
| `sebas-webui` | WebUI dashboard：axum 路由、模板渲染、SSE 推送；会话读写全部经 `SessionBackend` seam（进程内或 socket），不依赖 sebas 二进制 crate |
| `xtask` | 构建期工具：update-models（拉取 models.dev 生成模型表）、check-docs（幽灵引用检查） |

依赖方向：根 crate 依赖全部成员；成员之间互不依赖
（`sebas-webui` 只依赖 `sebas-router` 的事件类型与 `sebas-feishu` 的 SessionKey，
通过 `SessionBackend` trait 与二进制 crate 解耦——
来源：`sebas-webui/src/session_backend.rs`）。

## 5. 校对结论

全文对照 `openspec/changes/add-architecture-doc/proposal.md` 的 What Changes 五项：
进程全景（§1）、watchdog 监督模型（§2）、IPC 语义（§3）、默认启动策略（§2.2）、
CLI 控制面与 crate 职责速查（§4）——已逐项覆盖；无未来计划或未实现设想。
