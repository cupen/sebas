# tasks — document-process-ipc-subcommands

> 纯文档 change：只读梳理 + 新增 `docs/architecture/process-ipc-subcommands.md`，零生产代码变更。

## 1. 现状核对（只读）

- [ ] 1.1 核对进程派生：通读 `src/watchdog.rs` 四个 Spawner 与 `src/lib.rs` 三个 SUBCOMMAND 常量，记录派生 argv 与 env，验证：`rg "CoreSpawner|WebUiSpawner|RouterSpawner|ImSpawner|SEBAS_IPC|SEBAS_CORE_SECRET|SEBAS_CONTROL_SECRET" src/` 均有命中且与 design D2 一致
- [ ] 1.2 核对三通道：通读 `src/ipc.rs`、`src/watchdog/control_rpc.rs`、`src/core_channel/{mod,protocol,server}.rs`，记录职责/认证/生命周期，验证：对照表三行均能指到具体函数（`ChildIpc::ready`、`control_rpc::serve/request`、`core_channel::server::serve`）

## 2. 总览文档

- [ ] 2.1 起草 `docs/architecture/process-ipc-subcommands.md`（进程树 + 三通道对照表 + 子命令三类分发表 + in-process 内嵌附注），验证：文件存在且三段齐全、ASCII 图可渲染
- [ ] 2.2 补调用链行号：每个表格条目附文件与函数名（如 `main.rs::run_control`、`run.rs::run`、`webui_cmd.rs::run`、`router_cmd.rs::run`、`im_cmd.rs::run`），验证：抽查 5 处行号与当前代码一致

## 3. 校验收尾

- [ ] 3.1 跑 `openspec validate --change document-process-ipc-subcommands` 通过且 `git status` 仅新增 1 份 docs + 3 份 planning artifacts，验证：validate 输出 pass、无生产代码 diff
