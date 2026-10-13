## Why

pi 装完"用不了"——新建会话里 native 槽位不可选、选了 pi 之后回话因凭据/端点失败。根因不是 pi 本身，而是 sebas 把 provider 配置喂给 pi 的通路是断的：把 claude 用的 `ANTHROPIC_*` env 喷到 pi（pi 对 baseUrl 重定向无效），detached 拓扑下独立 webui 拿不到 native 可达性（`execution_bodies()` 走 trait 默认 `None`），前端 `agentUnavailableLabel` 把 native 文案硬编码成"未安装 pi"——三处让"装完即用"始终不成立。

## What Changes

- **pi 的端点/密钥只从 sebas provider 注册表走**，不再喷 `ANTHROPIC_*` env。sebas 在 `<SEBAS_HOME>/pi-agent/` 生成 `models.json`（provider 级 baseUrl 覆盖 + 保留 pi 内置模型），spawn 时注入 `PI_CODING_AGENT_DIR` 与自命名 `SEBAS_PI_KEY_n`（密钥经 `${VAR}` 插值，不落盘）。`compute_pi_spawn_env` 等"塞 env"路径废弃。
- **detached 拓扑下独立 webui 能拉到执行体可达性**。`core_session_channel::StateSnapshot` 域扩 `execution_bodies` 载荷；`CoreChannelBackend` 覆写 `execution_bodies()`。native 槽位据此如实报"未配 provider / 二进制不可达 / 凭据无效"。
- **native 不可用文案按 cause 渲染**。前端删硬编码"未安装 pi"；`pi auth check` 调用补 `--provider`/`--model`。

## Capabilities

### New Capabilities

- （无）

### Modified Capabilities

- `pi-agent`（add-pi-driver 内 delta 上修改）：sebas 写**自己目录**的 `models.json` 桥；不得触碰操作员 `~/.pi/agent`；`ANTHROPIC_*` env 不再下发；端点须经 `models.json` 桥或 pi 自管凭据。
- `agent-workbench`：内置 PI 槽位可达性从 core `execution_bodies` 派生，detached 拓扑不再恒置灰。
- `webui`：native 不可用文案按服务端 cause 渲染；新建会话对话框如实呈现"未配 provider"等具体原因。
- `core-session-channel`：`StateSnapshot` 域增加 `execution_bodies` 载荷（per-body reachable + cause），独立 webui 进程可取到。

## Impact

- `src/spawn_env.rs`：删 `compute_pi_spawn_env`（deprecated shim 留给 e2e 迁移）。
- `src/agent_store.rs` / `src/run.rs` / `sebas-webui`：sebas 私有 `<SEBAS_HOME>/pi-agent/` 目录维护；spawn 注入 `PI_CODING_AGENT_DIR` + `SEBAS_PI_KEY_n`。
- `src/core_channel/`：协议增 `StateSnapshot.execution_bodies`；`CoreChannelBackend` 覆写。
- `sebas-webui` 前端：native 行不可用文案删硬编码（`new-session-dialog.ts` / `settings-modal.ts`）；catalog 取服务端 cause。
- `sebas-acp/src/pi/driver.rs`：`spawn_overrides` 不收 `ANTHROPIC_*`；`pi auth check` 补 provider/model 参数。
- 测试：把 `native_slot_runs_pi_with_provider_env_and_model_switch`（fake pi 桩）迁到 models.json 桥；新增 detached 双进程可达性回归。

## Non-goals

- 不删 `sebas-agent` 与 legacy native resume 路径（与 make-pi-the-builtin-agent 一致）。
- 不给 pi 加权限/审批桥（另立 change）。
- **不**读操作员 shell env 兜底（仅 s1：只认 sebas provider 注册表；注册表空就如实报"未配"）。
- 不做凭据的运行中热更新（spawn 期语义；切 provider 需新会话）。
- 不动 `add-pi-driver` 的 105 处 main-spec 提及；只改行为契约。
- 不修独立 1-bug（sebas-node pi 远端、pi 安装配方扩展等），归各自 change。
