# ACP 驱动 opencode 复验记录（2026-09-05）

> 验收人：sebas-agent 会话；沙箱 `/tmp/sebas-acp-itest`（WSL2 kali-linux），webui 端口 9879。
> 背景：`b9a3160`（feat/agent 顶端，含 state-store 引擎 + P2/P3 修复）上的独立复验，
> 距上轮验收（[acp-opencode-accept-2026-09-04.md](acp-opencode-accept-2026-09-04.md)）一天，
> 期间新增 5.x 状态引擎系列提交。**结论：ACP↔opencode 全链路通过；另发现 P1 启动崩溃回归（非 ACP）。**

## 环境

- opencode **1.18.29** 原生 Linux 二进制（WSL 内 `~/.opencode/bin/opencode`）。GitHub 直连下载被
  网络重置，改从 npm 包 `opencode-linux-x64` 提取二进制；`opencode.ai/install` 脚本会被
  Windows interop shim（`/mnt/c/.../npm/opencode`）误判「已安装」，WSL 里装原生版需绕开 PATH 里的 shim。
- sebas：`target/debug/sebas`（WSL 内 `cargo build`，6m30s）。
- 沙箱配置与 09-04 相同（`[acp.agents.opencode] command=["~/.opencode/bin/opencode","acp"]`、
  `[log] level="debug"`），另注意新增 `SEBAS_STATE_DB`：**不设它（缺省 `~/.sebas/sebas.db`）会踩
  下文 P1**，验收时将其指向不存在路径使 DB 回退文件存储后才可启动。
- 模型凭据：宿主已有 `DEEPSEEK_API_KEY`（models.dev 内置 deepseek 供应商同名 env），core 进程
  env 继承即达 opencode 子进程，无需登录态。

## 验证结果（全部通过）

1. **可达性**：`GET /api/agent-kinds` → `opencode reachable:true 1.18.29`。
2. **spawn 全链路**：initialize → `session/new`（cwd 透传 project_dir）→ `session/prompt`
   用真实 ACP id（`ses_…`）。
3. **模型选择**：创建带 `deepseek/deepseek-v4-flash` → `session/set_config_option` 生效，
   `current_model` 同步；**resume 后 load 响应 `currentValue` 保持**。
4. **完整轮次**：流式 chunk 精确拼出 `PONG-WIRE-P2`，`stopReason:end_turn`，usage 7332/7 tokens。
5. **P2 复验（0-turn 占位）**：无 prompt 创建立即返回 key、`spawning` 态、无 opencode 子进程；
   首条消息才 spawn 且 model 经 wire 透传到位。
6. **P3 复验（close 收割）**：close 后子进程 pid 消失，无僵尸。
7. **优雅退出**：SIGTERM → `core.sock` 移除、`sessions.json` 落盘（`acp_session_id` +
   `current_model`）、端口释放、无 opencode 残留。
8. **resume**：重启后 dormant → 对原 key 发消息 → `session/load` 携带真实 ACP id →
   模型保持 → 回复轮 `end_turn`。

## 🐛 P1 回归：默认配置启动即崩（state-store 引擎同步包装，非 ACP）

- **现象**：`SEBAS_STATE_DB` 指向可写路径（含缺省 `~/.sebas/sebas.db`）时，`sebas run` 启动
  panic：`Cannot start a runtime from within a runtime`（`sebas-router/src/state_store.rs`
  `load` 处；`save`/`update` 同款）。
- **根因**：`run.rs`（async）初始化 `DbStateEngine` 后 → `provider::build_form` →
  `FileStore::load` → `state_store::load()` 的 engine 分支在 **tokio worker 线程上**
  `Handle::current().block_on(...)`。`load()` 内注释「core 始终在 tokio 中运行故 block_on
  安全」的论断不成立——在运行时 worker 上 block_on 本身即 panic。
- **影响**：DB 引擎可初始化即必崩；仅当 DB 初始化失败回退文件存储才幸免（本轮验收即在
  回退路径下完成）。DB 侧代码路径（engine 读写、订阅）因此**未获验收覆盖**。
- **修复建议**：engine 分支改 `tokio::task::block_in_place(|| Handle::current().block_on(..))`
  （注意 current_thread 运行时会 panic，需核对测试），或将 `load`/`save` 调用链异步化。
- 该提交（b9a3160 等）已在 main，**合并前必须修复**。

## 观察项（非阻塞）

1. **sessions.json 不持久化 project_dir**：重启后 resume 的子进程 cwd 落到 core 进程 cwd
   （`acp_driver` 的 `work_dir.unwrap_or(current_dir)` 兜底）。实测 opencode 1.18.29 能容忍
   （A/B 实验：错误 cwd 下 resume 仍完成），但编码类 agent 的工具会在错误目录执行。建议把
   project_dir 写入映射并在 resume 时透传。
2. **模型不可用时的静默回退**：请求的模型在子进程不可用时 `set_config_option` 仅 WARN
   （`Invalid params: model not found`）并回落默认模型；若首条 prompt 仍引用不可用模型则
   opencode 报 `OpenCode service failure`，用户侧只见无响应。排查时曾误判为 cwd 问题，已用
   A/B 实验排除（根因是重启 core 后 env 丢 key → deepseek 不可解析）。
3. **opencode 1.18.29 的 Zen 模型（big-pickle）无登录也可完成轮次**，与 09-04 记录的
   「big-pickle 挂起」不同，opencode 侧行为已变。
4. **一次 webui 在 core 重启后 HTTP 无响应**（端口仍监听）；当时存在多个测试 core 并存干扰，
   未复现、未定位，低置信度，仅记录。

## 复现要点（给后续会话/operator）

- 沙箱启动必须覆盖**全部**默认路径：`SEBAS_STATE_DB` 是新增的必设项（本轮踩坑后设为
  `/proc/no-such-dir/sebas.db` 强制文件回退；修复 P1 后应改指沙箱内真实路径并补验 DB 路径）。
- `pkill -f "sebas webui"` 会误杀承载它的外层 shell（MSYS/WSL interop 下模式匹配自身），
  用 `pgrep -f` 取 pid 后逐个 kill 更稳；多个测试 core 并存会让「socket 归属」与日志归属
  混乱，每轮实验前先清点 `pgrep -f "sebas run"`。

## 遗留

- P1（见上）待修复；修复后需补验：默认配置启动、DB 引擎读写路径（state_persistence /
  state_subscription 集成测试 + 沙箱启动冒烟）。
- bd 本地库在此 checkout 未物化（`.beads/` 仅配置，无 Dolt 数据），P1 以本文档承载，
  待在有库的机器补录 issue。
