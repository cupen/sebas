## Context

见 proposal.md Why。现状（`src/` 实测）：单 binary 多子命令，`main.rs::main` 经 `cli.rs::Cmd` 分发；`sebas run`（watchdog）是唯一监督者，以 `current_exe()` 派生 `core/webui/router/im` 子进程；进程间三通道并存：pipe Ready-only（`ipc.rs`）、control RPC socket（`watchdog/control_rpc.rs`）、core session channel（`core_channel/`）。`run.rs::run` 是 core 内编排点。

## Goals / Non-Goals

**Goals:**

- 给出一张可挂墙的进程拓扑 + 三通道对照表 + 子命令分发表，定位到文件/函数。
- 约定文档落点 `docs/architecture/process-ipc-subcommands.md`，后人改进程/通道时有地方同步。

**Non-Goals:**

- 不定义新协议、不改任何调用路径；文档与代码不一致时以代码为准并提 follow-up，不顺手修。

## Decisions

### D1：文档按"进程 → 通道 → 入口"三段组织，而非按文件罗列

- Rationale：用户提问原话就是两问（进程与通信 / 子命令入口调用），三段一一对应；按文件罗列会淹没拓扑。
- Alternative：按 crate 罗列（sebas/sebas_router/sebas_webui…）—— rejected，跨进程调用链会被拆散。

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
- 常驻 vs 条件：core/webui 默认开（`services.register(..., config.*.enabled)`），router 默认关（`--debug` 强制开），im 跟随 `[feishu]` 启用判定。

### D3：三通道对照表钉死"职责/认证/生命周期"三列

| 通道 | 方向 | 认证 | 生命周期 |
|---|---|---|---|
| pipe `{"cmd":"ready"}`（`ipc.rs::ParentIpc/ChildIpc`） | core→watchdog 单向 | `SEBAS_IPC=1` 环境标记 | spawn→ready→持续排空（日志转发），EOF 即死亡 |
| control RPC socket（`control_rpc.rs::serve/request`） | cli/webui/feishu→watchdog | `SEBAS_CONTROL_SECRET`（watchdog 随机生成，不落盘）+ uid/mode 0600 | watchdog 存活期；`control RestartCore/Update/Rollback/Services` 唯一命令面 |
| core session channel（`core_channel/server+client`） | webui/router→core | `SEBAS_CORE_SECRET`（watchdog 注入 core+webui+router）+ 同 uid | core 存活期；snapshot-then-subscribe；滞后订阅者被 drop |

- Rationale：三者最易混（都是 Unix/pipe + secret），表格强制回答"找谁、带什么钥匙、挂了找谁"。
- Alternative：三通道各写一节散文 —— rejected，对照阅读成本高。

### D4：子命令表分"常驻服务 / 一次性命令 / 控制面"三类

- 常驻：`core`（`run.rs::run`）、`webui`（`webui_cmd.rs::run`）、`router`（`router_cmd.rs::run`）、`im`（`im_cmd.rs::run`）、`run`（`watchdog.rs::run_watchdog`）。
- 一次性：`service`、`replay`、`record`、`update`、`agent-kinds list`、`webui-passwd`。
- 控制面：`control/*` + 顶层 `status/services` + 别名 `ctl`（`main.rs::run_control` 统一信封构造）。
- Rationale：与 `cli.rs::Cmd` 枚举顺序一致，读者可逐行对号入座；`CORE_SUBCOMMAND/RUN_SUBCOMMAND/ROUTER_SUBCOMMAND` 常量与 clap 名的同步测试（`main.rs::tests`）单列一行提醒。

### D5：落点选 `docs/architecture/` 而非 openspec 常驻 spec

- Rationale：梳理是现状快照，不承诺行为契约；openspec specs 只收行为需求，架构图进 docs 避免 `validate` 负担。后人改拓扑只改一份 md。

## Risks / Trade-offs

- [Risk] 文档快照易过期（spawner/secret 语义常变）→ Mitigation：文档头标注"以代码为准 + 生成日期 + 关键文件链接"；tasks 含 `openspec validate` 与 grep 复核步骤。
- [Risk] 三通道 secret 名字相近（CONTROL vs CORE）易抄错 → Mitigation：对照表单列一节"钥匙是谁发的、谁持有"，并引用 `watchdog.rs` 注入点行号。
- [Risk] `core --webui/--router` 内嵌形态 vs 独立子进程形态混淆 → Mitigation：拓扑下加注"in-process 内嵌（`run.rs` 内 `serve_with_listener`/`run_with_admin_adapter`）vs 独立进程"。

## Migration Plan

纯新增文档，无部署、无迁移、无回滚。合并后检查 `docs/architecture/process-ipc-subcommands.md` 存在且 `openspec validate --change document-process-ipc-subcommands` 通过即可。

## Open Questions

- 无。唯一不确定（是否要把飞书卡片/WS 细节画进附录）已在 Non-goals 排除，后续 change 另议。
