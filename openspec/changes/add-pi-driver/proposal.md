# add-pi-driver

## Why

sebas 的 agent 体系目前只有两个一等 driver：`claude`（专有 stream-json）与 `acp`（通用 ACP v1）。[pi](https://github.com/earendil-works/pi) 是全开源（MIT）、多 provider、自带完整 headless RPC 的 coding agent，官方 ACP 支持未立项（讨论 #4444），通用 `acp` driver 接不进它。接入 pi 给操作员一个不绑定订阅的一等 agent 选项，并按产品决策将默认 agent 从 claude 切换为 pi。

## What Changes

- `sebas-acp` 新增第三个 `AgentDriver` 实现 `PiDriver`：驱动 `pi --mode rpc` 子进程（stdio 严格 JSONL、JSON-RPC 2.0 超集），完整映射回合生命周期（text/thinking 增量、tool 生命周期、`agent_settled` 终态、usage、`set_model`、`abort` 取消、compaction 事件如实呈现）。
- 会话持久化与恢复：pi 会话落 `--session-dir`，pi session id 经握手上报（对齐 AcpDriver 的 `acp_session_id` 语义），恢复用 `--session <id>`。
- 权限 v1 如实不支持：pi 无权限系统，工具以进程权限直跑；`SetMode` 返回非终态不支持（对齐 AcpDriver 口径），不产生审批卡。
- 凭据 v1 零翻译：pi 自管 provider（`/login`、`models.json`、env 继承）；reachability 以 `pi auth check`（ready/not_ready/invalid）+ 二进制在场探测。
- 配置面：`[acp.agents.pi] driver = "pi"`（path/sessions_dir/work_dir/args 键值形式/超时，镜像 claude 形态）；装配点（`run.rs`、`agent_store.rs`）、模型层校验（`is_valid_driver`）、dispatch 写入口同步放行 `pi` 标签。
- **BREAKING**：产品默认 agent 由 claude 改为 pi——`[acp] default` 缺省与单 agent 隐式默认的硬编码回退从 `"claude"` 换为 `"pi"`（config.rs 三处）；`config.toml.example` 种子改为 pi 默认 + claude 保留可切回；pi 二进制缺失时默认 agent 诚实报不可达（不硬失败）。
- WebUI：Settings → Agents 表单新增 pi 形态；新会话目录自然含 pi 行。

## Capabilities

### New Capabilities

- `pi-agent`: pi 作为一等 agent 的驾驶行为契约——RPC 协议映射、会话恢复语义、无权限系统的如实呈现、模型选择、reachability 口径。

### Modified Capabilities

- `agent-driver`: 抽象从两个实现扩为三个；隐式/缺省默认 agent 回退由 claude 改为 pi（含 pi 缺失时的诚实退化场景）。
- `agent-settings`: agent 表单与驱动标签封闭集新增 `pi` 形态。

## Impact

- `sebas-acp`（新模块 `src/pi/`，协议编解码 + driver）、`src/config.rs`（AgentConfig 变体与回退）、`src/run.rs` / `src/agent_store.rs`（装配）、`sebas-models` / `sebas-dispatch`（标签校验）、`sebas-webui` 前后端（表单与目录）。
- 测试：协议翻译层离线 fixture 单测；进程级 journey skip-if-absent（pi 不在场跳过并如实报告）；沙箱验收配方经 fake-provider 做自定义 provider。
- 运行前提：Node ≥22.19 + `pi` 二进制；与 in-flight 的 add-agent-auto-install 协调（其封闭配方表后续加 pi 行，不在本 change）。

## Non-goals

- 不做权限审批桥（pi 扩展拦截 tool_call + extension UI → 审批卡，另立 change）。
- 不做 sebas-node 远端执行体的 pi 支持（触点已盘点，另立 change）。
- 不代写 pi 的 `models.json`（凭据与自定义端点由操作员在 pi 侧配置）。
- 不做 pi 安装配方自动化（挂靠 add-agent-auto-install 落定后加行）。
- 不动 claude / acp 两个既有 driver 的行为。
