# Design: add-agent-mode-selection

## Context

mode 在 webui 创建链路上不存在：`CreateSessionRequest` 只有 `prompt/project_id/agent/model`（`sebas-webui/src/api.rs:476`），前端无选择器。但仓库已有两处 mode 事实：

- **节点链路**：`SessionMode`（ask/edit/allow/auto，`sebas-node-link/src/lib.rs:290`）+ `SessionOp::Spawn.mode` / `SessionOp::SetMode` wire 字段 + 节点内 `record_gate` 门控。投影 `spawn_on` 已有 `mode` 参数（`src/node_link/projection.rs:539`），只是 webui 侧 `spawn_remote` 硬编码 `None`（`src/core_channel/server.rs:1075-1083`）。
- **claude driver**：`cc-agent-sdk` 有 `ClaudeAgentOptions`（未用）与运行时 `set_permission_mode(PermissionMode)`（`sebas-acp/src/claude/driver.rs:327-336` 用作存活探针）。argv 从不带 `--permission-mode`。

缺陷：存活探针每秒发 `set_permission_mode(Default)`，一旦会话配置了启动 mode 会被探针覆盖——必须随本变更修掉。

测试桩现状：fake-claude（`tests/bin/fake-claude.rs`）已解析并忽略 `--permission-mode`（VALUE_FLAGS），journal 记录完整 argv；`perm` 场景走 PreToolUse hook_callback 等 webui 审批。e2e/验收已 100% 零真 token。

## Goals / Non-Goals

**Goals**
- mode 作为会话创建参数 + 会话中可切的控制面期望值，双放置路径（本机/远端）贯通。
- 全链路用现有词汇，不发明新协议语义；未知值拒绝而非降级。
- 测试全零 token，fake-claude 产生可断言的差异化行为。

**Non-Goals**
- 不暴露 plan 模式；不给通用 ACP driver / native 内核实现 mode 生效；不改节点 `record_gate` 语义；不做节点配置的 `enforces_mode` 声明面扩展。

## Decisions

### D1: wire 词汇用节点 `SessionMode`，本机映射为 CLI permission mode（用户已确认）

控制面统一说 `ask/edit/allow/auto`（含 UI）。到执行体时分叉：

| 控制面 mode | 本机 claude（argv / 运行时） | 远端节点 |
|---|---|---|
| 缺省/None | 不传 `--permission-mode` | `mode: None`（节点缺省 ask） |
| `ask` | 不传（CLI 默认逐次询问） | `mode: "ask"` |
| `edit` | `--permission-mode acceptEdits` | `mode: "edit"` |
| `allow` / `auto` | `--permission-mode bypassPermissions` | `mode: "allow"` / `"auto"` |

理由：词汇统一在控制面（UI、快照、投影 desired_mode 全是同一套）；执行体差异收敛到一个纯函数 `map_mode_to_claude(mode) -> Option<&str>`（放 `sebas-acp` claude 模块或 `src/dispatch.rs`，单一出处供 spawn 与运行时切换共用）。备选（暴露 CLI 词汇到 UI）被否——控制面出现两套 mode 语义，且节点路径对齐成本更高。

### D2: mode 走既有 model 通道同构贯通（不改协议形状）

mode 与 model 的生命周期完全同构（创建时可选、首 prompt 前应用、中途可切、失败非致命），故沿 model 已有路径逐站加字段，最小且模式已被验证：

```
CreateSessionRequest.mode
  → SessionBackend::spawn_with(prompt, dir, agent, model, mode, node)   [trait 加参]
  → CoreChannelRequest::Spawn { …, mode }                               [protocol.rs 加字段]
  → Out::WebSpawn { …, mode } / web_spawn(...)                          [sebas-dispatch]
  → handle_web_spawn: kind=claude 时 map_mode_to_claude(mode)
      → spawn_overrides 追加 --permission-mode <值>
  → 远端: spawn_remote → projection.spawn_on(..., mode, ...)            [替换硬编码 None]
```

占位会话（0-turn）：mode 与 agent/model 一起记在 mapping，首条消息触发 spawn 时随 `Out::WebSpawn` 带出（同 D2 既有占位机制，`mod.rs:1316` 的第二处 WebSpawn 同步加字段）。切换端点 `POST /api/sessions/{key}/mode` 仿 `set_session_model`：本机走 `AcpCommand::SetMode`（新增枚举变体，driver 调 `client.set_permission_mode(...)`），远端走投影 `set_mode`（`SessionOp::SetMode` 已有节点侧处理器，补控制面调用入口即可）。

备选（新开独立 mode 通道/协议帧）被否——同构生命周期没必要双轨。

### D3: 事件反馈复用"非致命错误 + 新增 ModeChanged"模式

仿 `ModelChanged`：driver 接受切换后发布 `AcpEvent::ModeChanged { mode }`；拒绝/失败发布既有非致命 Error 事件。webui 把 ModeChanged 写入快照 mode 字段（desired 随请求立即更新，effective 随事件落定——与节点链路 desired/effective 词汇对齐）。映射到 SDK：`edit→AcceptEdits`、`allow/auto→BypassPermissions`、`ask→Default`（运行时需要显式发 Default 而不是不发，以覆盖此前的放宽）。

### D4: 探针修复——发会话当前 mode 而非硬编码 Default

`driver.rs:327-336` 的 liveness probe 改为发 `self.current_permission_mode`（spawn 时初始化为映射结果，运行时切换成功后更新）。no-op 判活语义不变（CLI 对相同 mode 的 set 仍即时应答）。

### D5: fake-claude 行为差异化（测试可断言）

- `--permission-mode bypassPermissions` 时：`perm` 场景跳过 hook_callback，直接产出 tool_result（"perm done"）完成回合 → 行为级断言"免审批"。
- 其余 mode 保持既有 hook 交互（ask 对照面）。
- journal 新增运行时切换记录（收到 control_request `set_permission_model`/等价帧时写 `{"type":"mode_change","mode":...}`），argv 记录已有。
- 中途切换接受路径：fake-claude 对运行时 set_permission_mode 回 success（今日已 ack-and-ignore，升级为记录 + 影响后续 perm 场景行为）。

### D6: 前端最小面

- `client.ts`：`createSession` 加 `mode`；新增 `setSessionMode`。
- `workbench-composer.ts`：创建表单加 `wa-select` mode 下拉（默认项"agent 默认"= 不发送），紧跟 model 下拉的既有结构。
- 会话头部（dashboard）：展示 mode 标签（远端会话已有 `mode-tag`/`data-testid=session-mode` 基础，扩展为本机/远端通用）＋ 切换入口（`wa-select` 或菜单项），失败以既有非致命错误横幅呈现。

## Risks / Trade-offs

- [探针修复遗漏路径：resume/回放恢复的会话不知道自己的 mode] → spawn/resume 初始化时从 mapping 读回 desired mode 设置 `current_permission_mode`；mapping 无记录时落 Default（行为同今日）。
- [本机 bypassPermissions 下 PreToolUse hook 仍会被真实 CLI 触发] → 不依赖 hook 消失做断言；行为断言只对 fake-claude 成立，真 CLI 语义以 CLI 文档为准（acceptEdits/bypassPermissions 语义由 CLI 保证）。
- [`allow` 与 `auto` 在本机映射相同（bypassPermissions）] → 接受：本机 claude 执行体区分不了二者；节点路径二者语义不同（auto 完全不门控）。快照如实显示控制面词汇，effective 由执行体回报。
- [WebSpawn 加字段波及既有测试断言（解构 `..` 通配的用例不受影响，精确匹配的需同步）] → tasks 中列出受影响测试（`spawn_race_test.rs`、`session_endpoints_test.rs`、`core_channel/tests.rs`）。
- [中途切换时 agent 正在流式输出] → 命令经既有 AcpCommand 队列串行送达，与 SetModel 同一顺序保证；无新增并发面。

## Migration Plan

纯加法演进，无 breaking：不发送 mode 的调用方行为与今日逐字一致。部署顺序无关（webui 与 core 可分别升级——旧 core 收到带 mode 的 Spawn 帧会因 `deny_unknown_fields` 与否产生差异，需在实现时确认 protocol.rs 的 serde 策略：若 `deny_unknown_fields` 则 mode 字段为 Option 且旧帧兼容，新帧旧 core 会忽略未知字段——按既有协议演进惯例处理，实施时核对）。回滚 = 回退二进制，wire 上多余字段按各端 serde 宽容度被忽略。

## Open Questions

（无——词汇、映射、切换落地、测试程度、UI 范围均已在 grilling 中确认。）
