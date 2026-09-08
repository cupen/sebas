# document-process-ipc-subcommands

## Why

进程划分（watchdog/core/webui/router/im）与三套通信机制（pipe readiness、control RPC、core session channel）分散在 `main/ipc/watchdog/run/core_channel` 等模块，新人难以建立统一心智模型，排查问题常走错通道。本变更用一张统一视图把进程、通信、子命令入口调用关系钉死为文档。

## What Changes

- 梳理进程模型：`sebas run`（watchdog 父进程）与 `core/webui/router/im` 子进程的派生关系、常驻/按需状态、监督与退出语义。
- 梳理进程间通信：pipe Ready-only 握手（`SEBAS_IPC`）、control RPC Unix socket（`SEBAS_CONTROL_SECRET`）、core session channel（`SEBAS_CORE_SECRET` + `SEBAS_CORE_SOCKET`）的职责边界、认证与生命周期。
- 梳理子命令入口：`src/main.rs` → `src/cli.rs` → 各 `*_cmd.rs`/`run.rs`/`watchdog.rs` 的分发表，标注常驻服务、一次性命令、控制面三类。
- **取代 `docs/architecture.md`**：该文档自称"进程结构与通信语义的单一事实来源"，但内容已过时（Gateway 旧命名、三受管服务漏 im、"5 个成员 crate"实为 10、不存在的 watchdog 隐藏别名），且与本 change 主题约八成重叠。新文档以当前代码实测接棒，旧文档删除，`README.md` 架构指针改指新文档。
- 产出 `docs/architecture/process-ipc-subcommands.md` 一份总览文档（含 ASCII 拓扑图与调用表），零生产代码变更。

## Capabilities

### New Capabilities

（无——纯文档梳理，不引入新行为）

### Modified Capabilities

（无——不改变任何现有 spec 的需求行为）

> 本 change 为纯文档/梳理性质，无 spec-level 行为变更，因此设置 `skip_specs: true`，不创建 delta spec。

## Impact

- 新增 `docs/architecture/process-ipc-subcommands.md` 1 份；**删除 `docs/architecture.md`**（内容由新文档接棒）；`README.md` 架构指针一行改指新文档。涉及代码仅作只读梳理（`main.rs`、`cli.rs`、`ipc.rs`、`run.rs`、`watchdog*.rs`、`core_channel/*`、`webui_cmd.rs`、`router_cmd.rs`、`im_cmd.rs` 及 `sebas-router`/`sebas-webui`/`sebas-im` 委托入口）。
- 对运行时、API、配置、依赖零影响。

## Non-goals

- 不修改进程行为、通信协议、子命令语义（只画图，不改线）。
- 不重构 `watchdog` 监督逻辑与 channel 鉴权。
- 不覆盖飞书 WS/卡片渲染与 router 转发细节（各归其 spec）。
- 不逐字迁移旧 `docs/architecture.md`——以代码实测为准重写；旧文档过时陈述（Gateway 命名、三服务、crate 数、隐藏别名）不予保留。
