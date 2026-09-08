## Context

见 proposal.md Why。现状（`src/` 实测）：单 binary 多子命令，`main.rs::main` 经 `cli.rs::Cmd` 分发；`sebas run`（watchdog）是唯一监督者，以 `current_exe()` 派生 `core/webui/router/im` 子进程；进程间三通道并存：pipe Ready-only（`ipc.rs`）、control RPC socket（`watchdog/control_rpc.rs`）、core session channel（`core_channel/`）。`run.rs::run` 是 core 内编排点。

注意 workspace 有 10 个成员 crate（`sebas-router/sebas-webui/sebas-im/sebas-feishu/sebas-acp/sebas-dispatch/sebas-agent/sebas-channels/sebas-ipc/xtask`）：`src/` 里的 `*_cmd.rs` 只是进程的门，各子进程的实体多在门后一跳的 crate 里（如 router 的通道订阅在 `sebas-router/src/core_channel.rs`、im 宿主逻辑在 `sebas_im::bootstrap`）。文档必须覆盖到那一跳，否则只画门不画房间。

## Goals / Non-Goals

**Goals:**

- 给出一张可挂墙的进程拓扑 + 三通道对照表 + 子命令分发表，定位到文件/函数。
- 约定文档落点 `docs/architecture/process-ipc-subcommands.md`，后人改进程/通道时有地方同步。

**Non-Goals:**

- 不定义新协议、不改任何调用路径；文档与代码不一致时以代码为准并提 follow-up，不顺手修。

## Decisions

### D1：文档按"进程 → 通道 → 入口"三段组织，而非按文件罗列

- Rationale：用户提问原话就是两问（进程与通信 / 子命令入口调用），三段一一对应；按文件罗列会淹没拓扑。
- Alternative：按 crate 罗列（sebas/sebas_router/sebas_webui…）—— rejected，跨进程调用链会被拆散。注意 rejected 的是"组织维度"，不是"覆盖范围"：各入口条目仍须点到其委托的 crate 实体（见 D4 委托列）。

### D2：进程拓扑以 watchdog 为根画一棵派生树，标注常驻/条件启动

```
            sebas run (watchdog, watchdog.rs::run_watchdog)
             ├─ sebas core ── sessions/adapters/router-in-process
             │    (CoreSpawner, SEBAS_IPC=1, stdout=pipe)
             ├─ sebas webui ── standalone dashboard (WebUiSpawner, 无readiness门)
             ├─ sebas router ── LLM proxy (RouterSpawner, --debug可选)
             └─ sebas im ── 飞书WS宿主 (ImSpawner, [watchdog.im]enabled)
```

- Rationale：与 `CoreSpawner/WebUiSpawner/ImSpawner/RouterSpawner` 四个 spawner 一一对应，可验证。
- 常驻 vs 条件：**core 默认关**（`[watchdog.core] enabled` serde default=false，注释原话"默认关：feishu 是可选项，`sebas watchdog` 默认只启动 WebUI"，`config.rs` + `watchdog.rs:378`），webui 默认开（`default_webui_enabled`），router 默认关（`--debug` 强制开），im 跟随 `[feishu]` 启用判定；注册机制均为 `services.register(..., config.*.enabled)`（`watchdog.rs:399-450`），初值停用的服务仍注册以便服务页重启。

### D3：三通道对照表钉死"职责/认证/生命周期"三列

| 通道 | 方向 | 位置发现 | 认证 | 生命周期 |
|---|---|---|---|---|
| pipe `{"cmd":"ready"}`（`ipc.rs::ParentIpc/ChildIpc`） | core→watchdog 单向 | 无 socket——继承的 stdout fd | `SEBAS_IPC=1` 环境标记 | spawn→ready→持续排空（日志转发），EOF 即死亡 |
| control RPC socket（`control_rpc.rs::serve/request`） | cli/webui/feishu→watchdog | `XDG_RUNTIME_DIR/sebas/control.sock`，未设时 per-uid 回退（`control_rpc.rs::default_socket_path`），客户端同一解析 | **仅 secret 比对 + socket mode 0600，无对端 uid 校验**（`SEBAS_CONTROL_SECRET` watchdog 随机生成、不落盘；actor 里的 uid 只是审计元数据） | watchdog 存活期；命令面 = `RpcControlRequest` 全部 11 个变体：Status/EventsSince/Update/Rollback/RestartCore/ServiceStatus/ServiceStatusFor/ServiceSet/ServiceRestart/Confirm/Cancel（重操作异步受理 operation_id） |
| core session channel（`core_channel/server+client`） | webui/router/im→core | `SEBAS_CORE_SOCKET`（watchdog 按 config `core.channel_path` 解析、缺省 `core_channel::default_socket_path` 后注入 router；webui/im 经 `core_channel::socket_path(&cfg)` 与 core 同一解析） | `SEBAS_CORE_SECRET`（watchdog 注入 core+webui+router+im 四个进程，`ImSpawner` 亦持有）+ `SO_PEERCRED` 同 uid 校验（仅此通道有，`core_channel/server.rs::peer_uid_ok`） | core 存活期；snapshot-then-subscribe；滞后订阅者被 drop |

- Rationale：三者最易混（都是 Unix/pipe + secret，名字还相近），表格强制回答"去哪找、找谁、带什么钥匙、挂了找谁"——两条 socket 通道的**路径发现机制不同**（control 靠约定路径函数、core channel 靠注入 env），正是排查走错通道的高发区，必须单列。
- Alternative：三通道各写一节散文 —— rejected，对照阅读成本高。

### D4：子命令表分"常驻服务 / 一次性命令 / 控制面"三类

- 常驻（每行带"src/ 入口 → 委托 crate 实体"两跳，已实测）：
  - `core`：`run.rs::run` → 内嵌形态时 `sebas_router::server::serve_with_listener` / `sebas_webui::run_with_admin_adapter_and_auth`（`run.rs` 内）
  - `webui`：`webui_cmd.rs::run` → `sebas_webui::run_with_admin_adapter_and_auth`（standalone 形态，同一函数）
  - `router`：`router_cmd.rs::run` → `sebas_router::server::run`；通道订阅在 `sebas-router/src/core_channel.rs`
  - `im`：`im_cmd.rs::run` → `sebas_im::bootstrap::bootstrap`；飞书卡片配置 `sebas_feishu::cards`
  - `run`：`watchdog.rs::run_watchdog`（父进程本体，无 crate 委托）
- 一次性：`service`、`replay`、`record`、`update`、`agent-kinds list`、`webui-passwd`。
- 控制面：`control/*` + 顶层 `status/services` + 别名 `ctl`（`main.rs::run_control` 统一信封构造）。
- Rationale：覆盖 `cli.rs::Cmd` 全部变体，按三类对号入座——注意枚举声明顺序与运行类别**不一致**（`Router` 在 `WebUi` 之前、`Run` 在 `Im` 之前、一次性命令穿插其间），文档不要宣称"逐行对应"。委托列防止文档止步 `src/`——`*_cmd.rs` 只是门，门后的 crate 才是进程实体，缺了它新人排查照样断线。
- SUBCOMMAND 常量现状（如实标注，勿美化）：仅 `CORE_SUBCOMMAND` 有 clap 名同步测试（`main.rs::tests`）；`RUN_SUBCOMMAND` 仅 `service.rs` 使用、无测试；`ROUTER_SUBCOMMAND` 是**死常量**——`RouterSpawner` 硬编码 `"router"` 字面量（`watchdog.rs` argv 构造处），文档单列一行提醒并提 follow-up issue。

### D5：新文档**取代** `docs/architecture.md`，落点 `docs/architecture/process-ipc-subcommands.md`

- 现状：`docs/architecture.md` 自称"进程结构与通信语义的单一事实来源"，与本 change 主题约八成重叠，但内容已过时——① 三受管服务（Core/WebUi/`Gateway` 旧命名）漏 `Im`（代码实为四个，`watchdog.rs` 注册处）；② `WatchdogGatewayConfig` 已改名 `WatchdogRouterConfig`；③ "根 crate 加 5 个成员 crate"实为 10（`Cargo.toml` members）；④ "`watchdog` 隐藏别名"在 `cli.rs` 不存在；⑤ 唯独默认启动策略（"仅 WebUI 默认启用"）写对了，新文档收编该结论。
- Rationale：并存必然漂移（同一拓扑两份 md 各自腐烂）；旧文档反正已烂，接棒成本低于合流。`README.md` 架构指针一行改指新文档。openspec specs 只收行为需求，架构快照进 docs 避免 `validate` 负担——落点选 `docs/architecture/` 而非常驻 spec 的原由不变。
- Alternative：修订旧文档而非新建 —— rejected，本 change 的三段式结构（进程→通道→入口）与旧文档章节差异大，修订等于重写且丢掉子命令分发表这个增量；在旧路径上重写又会让文件名与内容失配。

## Risks / Trade-offs

- [Risk] 文档快照易过期（spawner/secret 语义常变）→ Mitigation：文档头标注"以代码为准 + 生成日期 + 关键文件链接"；tasks 含 `openspec validate` 与 grep 复核步骤。
- [Risk] 三通道 secret 名字相近（CONTROL vs CORE）易抄错 → Mitigation：对照表单列一节"钥匙是谁发的、谁持有"，并引用 `watchdog.rs` 注入点行号。
- [Risk] `core --webui/--router` 内嵌形态 vs 独立子进程形态混淆 → Mitigation：拓扑下加注"in-process 内嵌（`run.rs` 内 `serve_with_listener`/`run_with_admin_adapter_and_auth`）vs 独立进程"。
- [Risk] 文档锚点（函数名/表格条目）随代码漂移腐烂 → Mitigation：统一用 `file::fn` 锚点而非行号（行号腐烂最快），文档头标注"以代码为准 + 生成日期"。

## Migration Plan

纯文档变更，无部署、无迁移、无回滚。合并后检查三件事：`docs/architecture/process-ipc-subcommands.md` 存在、`docs/architecture.md` 已删除且仓库内无残留引用（`rg "architecture\.md" --glob '!openspec/**'` 应只剩指向新文档的指针）、`openspec validate document-process-ipc-subcommands` 通过。

## Open Questions

- 无。唯一不确定（是否要把飞书卡片/WS 细节画进附录）已在 Non-goals 排除，后续 change 另议。
