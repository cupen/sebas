# design — add-opencode-acp

## Context

动机见 proposal.md——opencode 是标准 ACP v1 agent，接入是"配置即用"；真正的实现缺口是 `AcpDriver` 的 resume（当前对 `resume=true` 只 `warn` + fresh）。设计以 spec（`opencode-agent` 新能力 + `acp-driver` resume delta）为需求面。

关键结构事实（已核实源码）：

- **接入面已经就绪**：`run.rs::build_agent_registry` 把 `AgentConfig::Acp{..}` 一律绑定 `AcpDriver`；`sebas-webui/src/agent_kinds.rs::discover_agent` 用 `<exe> --version` 探测——opencode 的 `--version` 输出裸版本号（如 `1.18.25`），两条路径都天然兼容，**零代码改动**即可接入。
- **ACP `session/load` 已确认**：`agent-client-protocol`（2.0.0）提供 `LoadSessionRequest::new(session_id, cwd)`，响应 `LoadSessionResponse` 无 not-found 字段，失败以 `RpcError` 形式返回；initialize 响应带 `capabilities.load_session: bool`。
- **opencode 的 ACP 能力已验证**：`opencode acp` 子命令（非 `--acp` flag）、`session/load` 完整（`loadSession: true`、sessionCapabilities 含 resume）、`session/new` 的 cwd 被 honor、权限请求映射 allow_once/allow_always/reject、流式 text/thinking delta、stdin 关闭即进程退出。

## Goals / Non-Goals

**Goals:**
- opencode 配置即用（不新增驱动分支、不改注册表）
- ACP 通用 resume 落地，opencode 为首个受益者
- 验收流程设计：自动化（mock ACP 回归）与真实 opencode 冒烟两档，为"配置即可用"设质量门槛

**Non-Goals:**
- 不扩展 ACP 协议面（fork/list/close/usage/plan 维持现状）
- 不新增 opencode 专属配置注入（`--model`、provider 透传走现有 `extra_env`）
- 不改飞书侧、不做数据迁移

## Decisions

### D1 接入面：零代码，配置 + 验证

opencode 走现有 `driver = "acp"` 通道。实现侧只需要：文档/配置示例加一个 `[acp.agents.opencode]` 条目 + 测试确认 `agent-kinds list` 探测兼容（`--version` 裸版本号）。**不做**任何 opencode 专属分支。

### D2 ACP resume：异步握手 + 握手完成通知（核心）

**问题**：`AgentDriver::spawn` 同步返回 `DriverHandle { session_id, resumed, run }`；Claude 驱动在 spawn 内同步握手，所以 `resumed` 同步可知。但 `agent-client-protocol` 的 `connect_with(agent, |cx| async move { … })` 是**闭包式**的，连接句柄无法移出闭包——ACP 驱动的 initialize + load/new 只能在 `run` future 内异步执行。若沿用 `DriverHandle.resumed: bool` 同步字段，spawn 返回时 resume 结果未知；更糟的是**fallback fresh 需要换 session_id**（load 失败 → 新 uuid），而 manager 用 `handle.session_id` 作为会话表 key——若 id 在 run 内才变，表 key 与真实会话错乱，`kill(sid)` 会 miss。

**决策**：`DriverHandle` 增加一个可选握手信号，把"最终 routing id + resumed"的确定推迟到握手完成：

```rust
pub struct DriverHandle {
    pub session_id: String,                        // 已知值（Claude）；ACP 以握手结果为准
    pub resumed: bool,                             // Claude 用；ACP 忽略
    pub handshake: Option<oneshot::Receiver<(String, bool)>>, // 新增：ACP 握手完成信号
    pub run: BoxFuture<'static, ()>,
}
```

- **driver 侧（AcpDriver）**：`run` 内握手完成后 `handshake.send((final_sid, resumed))`——load 成功 → `(原 sid, true)`；load 失败/无能力 → `(新 uuid, false)` 并继续 `session/new`；普通 new → `(sid, false)`。Claude 驱动不产生该信号（`None`）。
- **manager 侧（spawn）**：`let handle = entry.driver.spawn(cfg).await?;` 之后、插入会话表之前：

```rust
let (sid, resumed) = match handle.handshake {
    Some(rx) => match timeout(entry.startup_timeout, rx).await {
        Ok(Ok(p)) => p,
        Ok(Err(_)) | Err(_) => return Err(/* driver 已终止或握手超时 */),
    },
    None => (handle.session_id.clone(), handle.resumed),
};
```

- 表插入、`SpawnOutcome` 全部使用握手后的 `(sid, resumed)`；`startup_timeout` 语义对 ACP 变为"initialize + load/new 完成等待"，与 Claude 的握手超时一致。
- `AgentDriver` trait 不变，Claude 路径行为完全不变。

**备选（否决）**：两次连接（spawn 内握手后断开，run 再连）——对持久化 session 的 agent 可行，但对内存态 ACP agent 不成立，且 double-handshake 浪费；"不换 id 直接 terminal error"——丢掉 `resumed=false` 的 fallback 语义，与 Claude 的 ResumeRejected 语义不一致。

### D3 失败路径统一：无能力 / load 失败 → fresh fallback

load 前先查 `capabilities.load_session`：`false` → 直接 fresh（省一次注定失败的请求，且"不假装成功"）。flag 为 `true` 但 load 抛 `RpcError`（会话不存在等）→ fresh fallback。两条路径都 `resumed=false` 并 `warn`，与 Claude 的 resume-rejection fallback（sebas-dk8.4）语义对齐。**不允许**把 RpcError 原样上抛到调用方。

### D4 验收流程：自动化回归 + 真实冒烟两档（本 change 的落地质量门）

**A 档——自动化（无 opencode 依赖，进 CI/常规测试）**：

- **mock ACP agent 夹具**：用 `agent-client-protocol` 的服务端角色（或最小 stdio JSON-RPC server）起一个可编程的 fake ACP agent，能按场景脚本化：① 声明 `load_session=true` 且 load 成功；② 声明 `load_session=true` 但 load 抛错（会话不存在）；③ 声明 `load_session=false`；④ 普通 new。每个场景断言 `SpawnOutcome` 的 `session_id`/`resumed` 与后续事件流。
- **新增单测/集成测试**（挂在 `sebas-acp/tests/`）：
  - resume 成功：`resumed=true`、routing id 不变、后续 prompt 走同一会话；
  - load 失败：`resumed=false`、id 为新 uuid、会话照常可对话；
  - 无 load 能力：同上 fresh fallback；
  - 握手超时：`startup_timeout` 到期 → spawn 报错、run 终止（子进程被杀）。
- **回归**：现有 `sebas-acp/tests/` 全绿（spawn/lifecycle/permission_roundtrip/no_duplicate_prompt/canned 等）。
- **探测兼容**：`discover_agent("opencode", &["opencode","acp"])` 的单元验证（有二进制时 reachable+version；无二进制时 unreachable+cause）。

**B 档——真实 opencode 冒烟（需 opencode 二进制 + 模型凭据，人工/半自动）**：

1. 配置 `[acp.agents.opencode] driver="acp", command=["opencode","acp"]` → `sebas agent-kinds list` 报 `opencode reachable <version>`；
2. webui 创建 opencode 会话 → 提示 → 流式 text/tool 事件 → 权限卡往返（如遇工具权限）；
3. `/cancel` 中断 → 会话可继续；
4. 结束会话后 **resume** → `resumed=true`、同一会话继续；
5. 观察 resume 后是否重放历史消息（见 R1）并记录现象。

冒烟步骤固化为 `docs/` 下的验收清单（或 tasks 内嵌清单），B 档通过才宣告"接入验收完成"。

## Risks / Trade-offs

- **[R1] resume 历史重放**：opencode load 后会把历史消息作为 session/update 重放给客户端（source-confirmed），下游 webui 可能重复渲染旧消息。→ 冒烟验收项 5 必须观察并记录；若确认是问题，后续 change 在 load 握手后抑制重放事件（本期不预实现，避免猜测）。
- **[R2] 握手超时语义**：ACP spawn 现在把 initialize+load 放进 `startup_timeout` 等待窗口，opencode（Node 进程）启动偏慢，默认 30s 应足够。→ 超时即明确报错并终止子进程（沿用现有 Timeout 错误语义），配置可调。
- **[R3] 权限映射**：opencode 的 option kind 为 allow_once/allow_always/reject，`map_decision` 的 AllowOnce→allow_once、AllowSession→allow_always、Deny→reject 应直接匹配；Deny 兜底 Cancelled。→ B 档冒烟项 2 验证真实往返，防呆不防实测。
- **[R4] 驱动层结构改动波及 Claude 路径**：`DriverHandle` 加字段、manager 加 await，是共享代码。→ 保持 `handshake=None` 时走原路径，Claude 相关测试全绿作回归门。

## Migration Plan

- 部署：无数据迁移、无破坏性变更。新增配置条目即启用；`driver="acp"` 的既有 agent（如 gemini）升级后获得 resume 能力（若其支持 load）。
- 回滚：移除 opencode 配置条目即回到现状；驱动层改动随二进制整体回滚，旧版本无 resume 的 fresh fallback 行为与新版本对不支持 load 的 agent 一致。

## Open Questions

- [R1] **（已实证）** opencode `session/load` 拒绝 sebas 的 uuid 路由 id。真实验收
  （沙箱 core + 真实 opencode，2026-09-04）：fresh 会话正常（`4d512724-...`），
  优雅退出 dump state 后重启，对 dormant 会话发消息触发 `SpawnResume` → driver
  发出 `LoadSessionRequest(4d512724-...)` → opencode 回 `Internal error: OpenCode
  service failure` → 正确 fresh fallback（`f3526dc2-...`，`resumed=false`）。
  这说明 opencode 的 `loadSession` 按其自身 session store / id 格式解析，不认
  sebas 的 uuid。修复方向（后续 change）：sebas 需持久化 opencode 返回的**真实
  ACP session id** 并与路由 session_id 映射，resume 时用真实 id 发 load——这超出
  本 change「把 ACP load 接上 + 诚实回退」的边界，记为 next-step。
- [R1 重放] resume 成功后是否重放历史消息：同上一项，待 resume 真正打通后观察。
- ACP 会话的 `work_dir` 透传（`session/new` 的 cwd）：现有 `DriverConfig.work_dir`
  已覆盖（None 时 current_dir 兜底），本期不扩大配置面。
