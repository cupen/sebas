# tasks — document-process-ipc-subcommands

> 纯文档 change：只读梳理 + 新增 `docs/architecture/process-ipc-subcommands.md`，零生产代码变更。

## 1. 现状核对（只读）

- [x] 1.1 核对进程派生：通读 `src/watchdog.rs` 四个 Spawner 与 `src/lib.rs` 三个 SUBCOMMAND 常量，记录派生 argv 与 env；**通读 `src/config.rs` 各 `enabled` 字段的 serde default 与回退逻辑**（符号存在性 rg 拦不住默认值写错），验证：`rg "CoreSpawner|WebUiSpawner|RouterSpawner|ImSpawner|SEBAS_IPC|SEBAS_CORE_SECRET|SEBAS_CONTROL_SECRET" src/` 均有命中，且 core 关/webui 开/router 关/im 随 feishu 四个默认值与 design D2 一致
  - 核对通过：core 默认关（`WatchdogCoreConfig` derive Default → `enabled: false`）、webui 默认开（`default_webui_enabled` → true，host 127.0.0.1 / port 9797）、router 默认关（`WatchdogRouterConfig` Default；`run --debug` 经 `config.router.enabled || debug` 强制开）、im 缺省随 `FeishuConfig::is_enabled()`（显式 `[watchdog.im] enabled` 优先）。四 Spawner 的 argv/env 逐一记录进新文档 §1.1。
  - 漂移观察（设计写就后代码演进，文档以代码为准 D5）：core session channel 的 secret 不再只靠 watchdog 注入 env——harden-core-channel-deployment 后 core **无条件自动武装**：env 缺失时现场生成并原子写 secret 文件（`[watchdog.core] secret_file`，缺省 `<config 目录>/core.secret`，0600；`config.rs::core_secret_file_path` + `run.rs::arm_core_channel`）；且 ready 打点后移到 bind+落盘之后（bind 失败 → 75 退出）。已写入新文档 §1.4/§2.2。
  - 漂移观察（代码注释 vs 实现）：`src/lib.rs` 的 `RUN_SUBCOMMAND`/`ROUTER_SUBCOMMAND` doc 注释声称存在 `watchdog`/`gateway` 隐藏 clap 别名，`src/cli.rs` 里**并不存在**（design D5④ 对 cli.rs 的判断仍成立；出入门的是 lib.rs 注释）。纯文档 change 不改代码，留待 follow-up 修正注释（新文档 §3.4 已如实标注）。
- [x] 1.2 核对三通道：通读 `src/ipc.rs`、`src/watchdog/control_rpc.rs`、`src/core_channel/{mod,protocol,server,client}.rs`，记录职责/路径发现/认证/生命周期。注意 client 归属：`src/core_channel/client.rs` 只服务 webui+im 两个视角；**router 用的是 `sebas-router/src/core_channel.rs` 里自带的订阅客户端**（手工握手 + StateSubscribe），不在 src client 里。验证：对照表三行均能指到具体函数（`ChildIpc::ready`、`control_rpc::serve/request`、`core_channel::server::serve`），control 的 `default_socket_path`、core 的 `SEBAS_CORE_SOCKET` 注入点、两通道认证差异（control 仅 secret 比对、core channel 叠加 `SO_PEERCRED`）均有出处
  - 核对通过：`ipc.rs::ChildIpc::ready`、`control_rpc::{serve,request,default_socket_path}`（XDG → per-uid 回退，0600，11 个 `RpcControlRequest` 变体逐一清点）、`core_channel/server.rs::{serve,serve_bound,bind_channel_socket,peer_uid_ok,read_handshake,default_socket_path,socket_path}` 全部到位；router 订阅客户端确认在 `sebas-router/src/core_channel.rs::spawn_subscriber`（socket 只认 `SEBAS_CORE_SOCKET`，缺省降级文件监听）。
  - 漂移观察：① 通道客户端 secret 发现改为 env 优先 → secret 文件、每次连接前重读（`secret.rs::ChannelSecret::from_env_or_file`；router 侧 `sebas-router/src/core_channel.rs::channel_secret` 走 `SEBAS_ROUTER_CONFIG` 同目录推导）——core 重启换钥自愈；auth_rejected 不会原样无限重试（cover-core-channel-test-gaps）。② `/api/summary` reachability 携带机器可读 `kind`（startup_failed | auth_rejected | disconnected，`client.rs::FailKind` → `reachability()`）。③ 设计 D3「滞后订阅者被 drop」实为分流：**会话流**滞后即断开重快照，**状态流**滞后由服务端重发快照。均已写入新文档 §2.2/§2.3。
- [x] 1.3 核对子命令到 workspace crate 的一跳：验证四个常驻子命令的"src/ 入口 → 委托 crate"链路（`run.rs::run` 内嵌 `sebas_router::server::serve_with_listener` / `sebas_webui::run_with_admin_adapter_and_auth`；`webui_cmd.rs::run` → `sebas_webui::run_with_admin_adapter_and_auth`；`router_cmd.rs::run` → `sebas_router::server::run` + `sebas-router/src/core_channel.rs`；`im_cmd.rs::run` → `sebas_im::bootstrap::bootstrap`），验证：`rg "sebas_webui::|sebas_router::|sebas_im::" src/*_cmd.rs src/run.rs` 每条链路均有命中
  - 核对通过：四条链路 rg 全命中并逐一读源确认；`ROUTER_SUBCOMMAND` 仍为死常量（`RouterSpawner` 硬编码 `"router"` 字面量，WebUi/Im Spawner 同样硬编码字面量），design D4 的 follow-up 提醒已落入新文档 §3.4。

## 2. 总览文档

- [x] 2.1 起草 `docs/architecture/process-ipc-subcommands.md` **并取代 `docs/architecture.md`**（进程树 + 三通道对照表含"位置发现/认证"列 + 子命令三类分发表含"委托 crate"列 + in-process 内嵌附注）；收编旧文档中仍正确的结论（如默认启动策略"仅 WebUI 默认启用"），旧文档过时陈述（Gateway 命名、三服务、5 crate、隐藏别名）不得复现；删除 `docs/architecture.md`、`README.md` 架构指针改指新文档，验证：新文件三段齐全、ASCII 图可渲染、`rg "architecture\.md" --glob '!openspec/**'` 输出中旧路径零残留
  - 完成：新文档按 D1 三段组织（§1 进程 → §2 通道 → §3 入口），外加 §0 两平面一页心智模型（管理面 control RPC → watchdog；数据面 core session channel → core；pipe = readiness）；`docs/architecture.md` 已删除；`README.md` 指针一行改指新文档；旧路径字符串全仓（openspec 除外）零残留。旧文档正确结论（仅 WebUI 默认启用）收编为 §1.3；过时陈述零复现（经核：`[watchdog.gateway]` 兼容解析在 `config.rs` 也已不存在，一并未收编）。
- [x] 2.2 补调用链锚点：每个表格条目附 `file::fn` 锚点（如 `main.rs::run_control`、`run.rs::run`、`webui_cmd.rs::run`、`router_cmd.rs::run`、`im_cmd.rs::run` 及其委托 crate 入口），不锚行号（行号腐烂最快），验证：抽查 5 处锚点的函数在当前代码中存在且名字一致
  - 完成：全文 `file::fn` 锚点、零行号；成文后对 35+ 锚点逐一批量 rg 核验存在且名字一致（含 `run.rs::arm_core_channel`、`core_channel/server.rs::serve_bound`、`core_channel/secret.rs::from_env_or_file`、`sebas-router/src/core_channel.rs::spawn_subscriber`、`sebas_webui::run_with_admin_adapter_and_auth`、`sebas_im::bootstrap::bootstrap` 等），一处笔误（`reachability` 非 pub）已按实际签名修正。

## 3. 校验收尾

- [x] 3.1 跑 `openspec validate document-process-ipc-subcommands` 通过且 `git status` 恰为：新增 1 份 docs + 删除 `docs/architecture.md` + 修改 `README.md`（指针一行）+ 3 份 planning artifacts，验证：validate 输出 pass、无生产代码 diff
  - 完成：validate pass；`git diff main --stat` 仅 docs/README/openspec 路径，无生产代码。
