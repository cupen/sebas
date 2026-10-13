## Context

pi 安装后不能用的根因是 sebas 喂给 pi 的通路断了。`pi-agent` spec 仍要求"sebas 不喷 env 给 pi"，但 `make-pi-the-builtin-agent` 在 `src/spawn_env.rs::compute_pi_spawn_env` 把 `ANTHROPIC_API_KEY` / `ANTHROPIC_BASE_URL` 注入 pi 子进程——而 pi 的内置 Anthropic 模型把 `baseUrl` 写死为 `https://api.anthropic.com`，`createClient` 显式传 `baseURL: model.baseUrl` 压掉了 env 默认值（实测：`ANTHROPIC_BASE_URL` 设到假上游→零命中，pi 仍拨 api.anthropic.com）。所以"喷 env"这条对端点重定向无效、对密钥也只搭一半桥，sebas 的 provider 注册表实际上从未真正驱动过 pi。

detached 拓扑（`sebas run` 起 core + im + webui 为独立进程）下，独立 `sebas webui` 的 `CoreChannelBackend` 不实现 `execution_bodies()`，落到 trait 默认 `None`→`/api/agents` 的 native 行 `reachable=false`、前端 `agentUnavailableLabel` 对 native 硬编码"未安装 pi"——即使 pi 已装且 provider 已配，dropdown 仍恒置灰。

**已实测结论（驱动本设计）**：
- pi 只认 `models.json`（或 extension 的 `registerProvider`）作为端点声明；`ANTHROPIC_*` / `OPENAI_*` 之外的 baseUrl env 不被内置 provider 尊重。models.json 的 provider 级 `baseUrl` 覆盖保留 pi 内置 17 个 claude 模型，密钥经 `${SEBAS_PI_KEY_n}` 自命名 env 插值，真值不落盘。
- `pi auth check` 不带 `--provider`/`--model` 必报错（exit 0 但 stderr 错），导致 reachability cause 漏出"pi auth check: "半截文案。

## Goals / Non-Goals

**Goals:**
- sebas 把 provider 注册表映射成 pi 的 `models.json` 桥，文件在 sebas 自己的目录，操作员 `~/.pi/agent` 不被读写。
- detached 拓扑下独立 webui 能拿到 `native` 行的真实可达性；前端按服务端 cause 渲染不可用文案。
- 删 `compute_pi_spawn_env` 这条"喷 env"路径；密钥改经 `${VAR}` 插值。
- 注册表为空时，pi 端如实报"未配 provider"。

**Non-Goals（设计级，仅约束不实现）**：
- 不读操作员 shell env 兜底（spec 层 s1 拍定，详见 Non-goals of proposal）。
- 不做凭据运行中热更新（spawn 期语义，切 provider 需新会话）。
- 不动 `sebas-agent` crate、legacy native resume 路径、`add-pi-driver` 自身 105 处 main-spec 提及。
- 不解决 pi 子进程仍继承父进程 `ANTHROPIC_*` 这件事（a1+ 已 deferred；本 change 不调 env_clear）。

## Decisions

### D1 — 桥文件形态：`<SEBAS_HOME>/pi-agent/models.json`

**选**：一份纯数据 `models.json`（`models.json` 形态），**不**走 extension `registerProvider` 的代码接口。

**理由**：实测验证两者能力等价（都能保内置模型、密钥经 env 插值），但 models.json 是数据、好审查、pi 的"配置"语义；extension 是代码、`ExtensionAPI` 演化时易碎。**关键差异**只有一条：models.json 必须落在 `<PI_CODING_AGENT_DIR>/`，extension 可经绝对路径 `--extension` 加载、不必劫持 agent 目录。代价是 sebas 必须设 `PI_CODING_AGENT_DIR`（与既有 `--session-dir` 钉子并列）。这是可接受的对齐——spawn 期一次性。

**否决**：extension（a1-e）。代码生成对本任务是过度设计、且越界 pi 私有 API 表面；与"一个文件一个写入者"的章程配合度反而下降。

### D2 — `PI_CODING_AGENT_DIR` 设为 `<SEBAS_HOME>/pi-agent/`（sebas 私有）

**选**：sebas 在自己的 SEBAS_HOME 下建 `pi-agent/` 子目录，spawn 时把 `PI_CODING_AGENT_DIR` 指向它。

**理由**：隔离 pi 自己的状态（sessions 已经在 driver 经 `--session-dir` 钉走；auth.json、models.json、settings.json、extensions/、skills/ 在 sebas 目录下是空的或受控的）。`add-pi-install-recipe` 把 pi 二进制装在 `<SEBAS_HOME>/agent-tools/pi/bin/pi`；agent-tools 与 pi-agent 平行，各管各的：tools=二进制，agent=桥。

**否决**：① 沿用操作员 `~/.pi/agent`（会写操作员的文件，越界）；② `<SEBAS_HOME>/.cache/...` 之类的暗目录（不可见，不便排障）。

### D3 — models.json 内容：`provider-level baseUrl` 覆盖，保 pi 内置模型

**选**：仅写 provider 级 baseUrl 与密钥（`apiKey: "${SEBAS_PI_KEY_n}"`），**不**列 `models`。pi 文档明说"只注册 baseUrl/headers 保留内置模型"，实测确认 17 个内置模型全部保留，端点改为新值。

**格式示例**（一个 anthropic 协议的 provider）：

```json
{
  "providers": {
    "anthropic": {
      "baseUrl": "https://gateway.example/v1",
      "apiKey": "${SEBAS_PI_KEY_0}"
    }
  }
}
```

多 provider（不同 protocol 家族）则多块：anthropic 用 `base_url_anthropic`，openai 家族用 `base_url_openai_chat`/`openai_responses`；同协议族不重复注入。**默认选择**为 `default_selection.provider`；同协议族默认赢，其余跳过并写诊断。

**否决**：为每个 provider 完整列 `models` 数组（要造 context/cost 元数据，sebas 没真值）。

### D4 — 密钥传输：`${SEBAS_PI_KEY_n}` env 插值，spawn 期

**选**：sebas 在 `models.json` 写 `${SEBAS_PI_KEY_n}`，spawn 时把真值放在 env（`SEBAS_PI_KEY_0=<key>`）。pi 的 `apiKey` 解析阶段读 env、真值不落盘。

**否决**：
- `--api-key` argv（pi 支持，但 `/proc/*/cmdline` 可见、密钥明文上进程列表）。
- `auth.json` 写盘（sebas 写操作员 `~/.pi/agent` 越界）。
- `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` 直传（用 claude 家族 env 名字空间——你已点过"别乱发 ANTHROPIC_*"；且 pi 内置 anthropic provider 对 baseUrl 重定向无效，喷了也没用）。

### D5 — `core_session_channel::StateSnapshot` 域扩展 `execution_bodies`

**选**：扩 `StateSnapshot { domain: "agents" }` 响应 payload，加 `execution_bodies: { native: { ok: bool, cause: Option<String> }, ... }` 字段。`subscribe` 流仍用 `Changed { scope: "agents" }` 触发，客户端据此 re-fetch。

**理由**：`StateSnapshot` 已是 agents 注册表/配置的传输面（`GET /api/agents` 路径已有）；复用一个域、增量扩字段，比另开新域或新 op 变更面更小，且不破坏既有 `StateSnapshot` 协议（未读 `execution_bodies` 的客户端忽略新字段）。

**协议层改动**：
- `sebas-ipc/src/protocol.rs` 的 `StateSnapshot` 响应帧 payload schema 加 `execution_bodies: Option<HashMap<String, ExecutionBodyStatus>>`（默认 `None`、现有客户端不变）。
- core 端在每次 `StateSnapshot agents` 响应中读 `DualSessionBackend::execution_bodies()` 填字段。

**否决**：① 另起新 RPC op（如 `ExecutionBodiesGet`）——变更面更大；② 让 webui 走 `/api/agents` 端直接连 core——违反"webui 不直连 core"既定架构。

### D6 — `CoreChannelBackend` 覆写 `execution_bodies()`

**选**：standalone webui 的 `CoreChannelBackend`（`src/core_channel/client.rs`）实现 `SessionBackend::execution_bodies()`，从最近一次 `StateSnapshot agents.execution_bodies` 缓存里取；缓存随 `Changed { scope: "agents" }` 推送刷新。

**理由**：现有 trait 默认 `None`→`unwrap_or(false)`→`/api/agents` 的 native 行恒 `reachable=false`。缓存+刷新避免每次 `GET /api/agents` 都做一次同步 RPC。

**否决**：trait 改默认值为 `Some(vec![])`——语义错（"都不可用"≠"未知"），且仍拿不到核心的真值。

### D7 — 前端 `agentUnavailableLabel` 改读服务端 cause

**选**：`sebas-webui/frontend/src/views/new-session-dialog.ts` 的 `agentUnavailableLabel(a)` 删除 `id === 'native'` 硬编码分支，按 a.cause 直渲染（可经 agent-workbench 的 "unavailable cause speaks operator language" 规则做友好映射，但不得写与 a.cause 矛盾的字面量）。

**理由**：把"原因从哪来"的决定权交回服务端。`make-pi-the-builtin-agent` 4.1 当时的硬编码是占位文案；`make-pi-ride-sebas-provider` 把"未配 provider"/"pi 不可达"两条原因从 server 端区分后，UI 就能如实显示。

**否决**：保持硬编码（不可接受——已与 server cause 矛盾）；在 `sebas-webui` 服务端 agent_kinds 查表时直接把 `cause` 替换为前端文案（耦合 + 跨包分散）。

### D8 — `pi auth check` 调用补 `--provider`/`--model`

**选**：`sebas-acp/src/pi/driver.rs` 调 `pi auth check <args>` 时读当前会话的 provider/model 拼上（provider=`anthropic`，model=pi 的默认 `claude-opus-4-8`），如 `args --provider anthropic --model claude-opus-4-8`。

**理由**：pi 1.1.0 的 `pi auth check` 不带 provider/model 必报错，stdout 出现 `Error: Auth checks require --provider <provider> or --model <model>`——`pi_auth_cause` 解析后 cause 落成半截 `pi auth check: `；补齐后正常返回 `ready` / `not_ready` / `invalid`。

**否决**：调用 `pi auth print-api-key --provider anthropic`——更重，且 pi 1.1.0 文档明说 print-api-key 用于把 key 打到 stdout 给外部用；与 reachability 探针无关。

### D9 — 删除 `compute_pi_spawn_env`（及下游引用）

**选**：`src/spawn_env.rs::compute_pi_spawn_env` 及其调用点（`src/session_boot.rs::spawn_overrides` 的 pi 分支、`make_pi_provider_env` 测试、e2e `native_slot_runs_pi_with_provider_env_and_model_switch`）整体移除。`resolve_pi_spawn_env` 留一个 `#[deprecated]` shim，调用点改为由 D1–D3 的桥文件路径处理。

**理由**：你已经点过"ANTHROPIC_* 仅用于 claude，别乱发"；这条喷 env 路径既没给 pi 端点重定向带来收益（实测），又对密钥只搭了半个桥，删除没有语义损失。e2e 用 fake-pi 桩的测试不依赖 env 注入；改为从 settings.db 配 fake provider 走桥即可。

**否决**：保留为可空路径（保留即默认走它、反而拖累桥文件路径被忽略）。

## Risks / Trade-offs

- **R1 桥文件重生 race**：provider 改了之后，正在跑的 pi 会话仍用 spawn 期的 `SEBAS_PI_KEY_n`（spawn 期语义，已在 spec 写明）。**缓解**：sebas 在 provider 变更时把变更 `default_selection` 记在事件流上；webui 侧对活跃 native 会话做"切换 provider"提示，归 `/api/sessions/{key}/switch` + 启动新会话。
- **R2 桥目录被改动**：sebas 的 `pi-agent/` 目录是 sebas 写的，操作员误删 → 下一会话 spawn 时找不到 models.json、pi 退到内置 catalog。**缓解**：在 core 启动时若该目录缺失，重建空 models.json；并把目录所有权写进 AGENTS.md 运维段。
- **R3 inheritance 残留**：本 change 不动 env_clear，pi 子进程仍会继承 core 进程 env 中的 `ANTHROPIC_*` 等；这对"装完即用"无碍（pi 内置模型只用 env 默认的 key，不读 baseUrl env），但与"零喷 ANTHROPIC_*"口号不一致。**缓解**：在 proposal 与 spec 明确"本 change 不动 env_clear"；该清理进 a1+ 的独立 change。
- **R4 e2e 迁移**：现有 `native_slot_runs_pi_with_provider_env_and_model_switch` 走 fake-pi 桩 + env 注入，迁移到桥文件路径要改 3 处（fake-pi 桩仍可用、env 注入改为"先建 settings.db fake provider 写入 base_url_anthropic，再 spawn"）。**缓解**：tasks 标 1.5 估时、留一个回归回归（fake-pi 桩仍能答 `get_available_models` 等）。
- **R5 协议前向兼容**：StateSnapshot 响应 payload 新增 `execution_bodies` 字段，旧客户端忽略（serde 忽略未知字段 + `Option<...>` 包裹默认 None）。**缓解**：在 `sebas-ipc` 加 `execution_bodies: Option<HashMap<String, ExecutionBodyStatus>>` 并在 `ipc_protocol_home` 契约里登记；protocol-contract 测试 fixture 加新行。

## Migration Plan

1. **阶段一（核心桥）**：实现 D1–D4 + D9。新增 `sebas-agent-bridge` 内部模块（嵌入 `sebas` 二进制 crate），在 `src/run.rs` 装配点：读 settings.db providers，生成 `models.json`，spawn pi 时注入 `PI_CODING_AGENT_DIR` + `SEBAS_PI_KEY_n`。删除 `compute_pi_spawn_env`。
2. **阶段二（通道）**：实现 D5–D6。扩 `StateSnapshot` 协议、core 端填字段、`CoreChannelBackend` 缓存+刷新。
3. **阶段三（前端）**：实现 D7 + D8。`agentUnavailableLabel` 改读 cause；`pi auth check` 调用补参数；catalog `pi_auth_cause` 解析不再出现半截文案。
4. **回归**：现有 `native_slot_runs_pi_with_provider_env_and_model_switch` 迁到桥文件路径；新增 `detached_topology_native_reachability_transmitted`（detached webui + core 双进程 + 验证 catalog native 行可达性与 core 一致）；`ipc_protocol_home_test` 增 execution_bodies 字段 fixture。
5. **回滚**：D9 路径切回 `#[deprecated]` shim 一次性（设置 `SEBAS_LEGACY_PI_ENV=1` 触发）；阶段二协议字段 `Option<...>` 默认 None，旧 core 与新 webui 兼容。

## Open Questions

无（Q1–Q5 已收口，注册表数据源 / 桥目录 / 端点格式 / 密钥传输 / 范围五个决定都成定局；a1+ 的 env_clear 治理明确 deferred 到独立 change）。
