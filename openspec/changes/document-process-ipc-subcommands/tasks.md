# tasks — document-process-ipc-subcommands

> 纯文档 change：只读梳理 + 新增 `docs/architecture/process-ipc-subcommands.md`，零生产代码变更。

## 1. 现状核对（只读）

- [ ] 1.1 核对进程派生：通读 `src/watchdog.rs` 四个 Spawner 与 `src/lib.rs` 三个 SUBCOMMAND 常量，记录派生 argv 与 env；**通读 `src/config.rs` 各 `enabled` 字段的 serde default 与回退逻辑**（符号存在性 rg 拦不住默认值写错），验证：`rg "CoreSpawner|WebUiSpawner|RouterSpawner|ImSpawner|SEBAS_IPC|SEBAS_CORE_SECRET|SEBAS_CONTROL_SECRET" src/` 均有命中，且 core 关/webui 开/router 关/im 随 feishu 四个默认值与 design D2 一致
- [ ] 1.2 核对三通道：通读 `src/ipc.rs`、`src/watchdog/control_rpc.rs`、`src/core_channel/{mod,protocol,server,client}.rs`，记录职责/路径发现/认证/生命周期。注意 client 归属：`src/core_channel/client.rs` 只服务 webui+im 两个视角；**router 用的是 `sebas-router/src/core_channel.rs` 里自带的订阅客户端**（手工握手 + StateSubscribe），不在 src client 里。验证：对照表三行均能指到具体函数（`ChildIpc::ready`、`control_rpc::serve/request`、`core_channel::server::serve`），control 的 `default_socket_path`、core 的 `SEBAS_CORE_SOCKET` 注入点、两通道认证差异（control 仅 secret 比对、core channel 叠加 `SO_PEERCRED`）均有出处
- [ ] 1.3 核对子命令到 workspace crate 的一跳：验证四个常驻子命令的"src/ 入口 → 委托 crate"链路（`run.rs::run` 内嵌 `sebas_router::server::serve_with_listener` / `sebas_webui::run_with_admin_adapter_and_auth`；`webui_cmd.rs::run` → `sebas_webui::run_with_admin_adapter_and_auth`；`router_cmd.rs::run` → `sebas_router::server::run` + `sebas-router/src/core_channel.rs`；`im_cmd.rs::run` → `sebas_im::bootstrap::bootstrap`），验证：`rg "sebas_webui::|sebas_router::|sebas_im::" src/*_cmd.rs src/run.rs` 每条链路均有命中

## 2. 总览文档

- [ ] 2.1 起草 `docs/architecture/process-ipc-subcommands.md` **并取代 `docs/architecture.md`**（进程树 + 三通道对照表含"位置发现/认证"列 + 子命令三类分发表含"委托 crate"列 + in-process 内嵌附注）；收编旧文档中仍正确的结论（如默认启动策略"仅 WebUI 默认启用"），旧文档过时陈述（Gateway 命名、三服务、5 crate、隐藏别名）不得复现；删除 `docs/architecture.md`、`README.md` 架构指针改指新文档，验证：新文件三段齐全、ASCII 图可渲染、`rg "architecture\.md" --glob '!openspec/**'` 输出中旧路径零残留
- [ ] 2.2 补调用链锚点：每个表格条目附 `file::fn` 锚点（如 `main.rs::run_control`、`run.rs::run`、`webui_cmd.rs::run`、`router_cmd.rs::run`、`im_cmd.rs::run` 及其委托 crate 入口），不锚行号（行号腐烂最快），验证：抽查 5 处锚点的函数在当前代码中存在且名字一致

## 3. 校验收尾

- [ ] 3.1 跑 `openspec validate document-process-ipc-subcommands` 通过且 `git status` 恰为：新增 1 份 docs + 删除 `docs/architecture.md` + 修改 `README.md`（指针一行）+ 3 份 planning artifacts，验证：validate 输出 pass、无生产代码 diff
