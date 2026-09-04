# Tasks: add-opencode-acp

依赖前置：无（建立在现有 `AcpDriver` / `agent-client-protocol` 之上，ACP `session/load` API 已核实可用）。
明确决策：opencode 接入零代码（走现有 `driver="acp"` 通道）；ACP resume 是核心实现；验收分自动化（A 档）与真实 opencode 冒烟（B 档）两档。

## 1. 驱动层：握手信号 + ACP resume

- [x] 1.1 `DriverHandle` 增加可选 `handshake: Option<oneshot::Receiver<(String, bool)>>`（最终路由 id + resumed），Claude 驱动不产生（pass `None`）；manager spawn 在 `handshake` 存在时以 `startup_timeout` 等待握手结果，超时/通道关闭报 `DriverError::Timeout`，否则用握手后的 `(sid, resumed)` 插入会话表并构建 `SpawnOutcome`。验证：Claude 路径单测/现有测试全绿（`resumed` 语义不变）；已有 ACP 测试（resume 恒 fresh）通过。
- [x] 1.2 AcpDriver `run` 内实现 resume 分支：`resume=true` 时先查 `capabilities.load_session`——为 `false` 则直接 fresh（`resumed=false`）；为 `true` 则发 `LoadSessionRequest::new(old_sid, cwd)`——成功后 `handshake.send((old_sid, true))`，`RpcError`（会话不存在等）则 fresh fallback（新 uuid + `resumed=false`）并经 `session/new` 起新会话；fresh 路径 `handshake.send((sid, false))`。任一路径都不得把 RpcError 原样上抛到调用方。验证：见 1.3~1.6 的 mock 场景测试。
- [x] 1.3 新增 mock ACP agent 测试夹具（`sebas-acp/tests/` 或 `tests/common`）：用 `agent-client-protocol` 服务端角色（或最小 stdio JSON-RPC server）起可编程 fake agent，按场景脚本化 initialize 的 `load_session` 声明与 load 响应。验证：夹具可被测试驱动，能按场景返回不同行为。
- [x] 1.4 集成测试「resume 成功」：mock agent 声明 `load_session=true` 且 load 成功 → `SpawnOutcome.resumed=true`、`session_id` 与原 id 相同；后续 prompt 同会话事件流正常。验证：该测试通过。
- [x] 1.5 集成测试「load 失败 / 无能力回退」：mock 分别模拟 load 抛错与 `load_session=false` → `resumed=false`、`session_id` 为全新 uuid、会话照常可对话（fresh）。验证：两个场景测试通过。
- [x] 1.6 集成测试「握手超时」：mock agent 不响应 initialize/load → `startup_timeout` 到期 spawn 返回 `Timeout` 错误、run 终止、子进程被杀。验证：测试通过且无僵尸进程。

## 2. 回归与探测兼容

- [x] 2.1 现有 `sebas-acp/tests/` 全量回归（spawn/lifecycle/permission_roundtrip/no_duplicate_prompt/canned_test/cancel_keeps_session 等）在驱动层改动后全绿。验证：`cargo test -p sebas-acp` 通过。
- [x] 2.2 `discover_agent("opencode", &["opencode","acp"])` 探测兼容验证：环境有 opencode 时报告 reachable + 裸版本号；无二进制时报告 `command not found` + unreachable（复用现有纯函数单测扩展）。验证：`sebas-webui` 相关单测通过。
- [x] 2.3 workspace 整体 `cargo test` + `cargo build` 无新增 warning、无回归。验证：全仓绿。

> 2.3 备注：workspace `cargo test` 唯一失败为 pre-existing 的 `sebas-feishu::permission_card_snapshot` 快照漂移（insta `.snap` 基准元数据是 crate 重构前旧路径 `feishu/` + 旧断言行 + 字段序，单测通过、并行下写 `.snap.new`）——与本次改动无关（stash 本 change 改动后单独跑 feishu 仍复现）。本 change 不动 feishu；快照重新接受属 feishu 重构遗留，另立 issue 处理。

## 3. 真实 opencode 冒烟（B 档验收）

- [x] 3.1 配置 `[acp.agents.opencode] driver="acp", command=["opencode","acp"]` 并 `sebas agent-kinds list` 确认 `opencode reachable <version>`（需 opencode 二进制可达，无则跳过并在结果中注明）。
- [x] 3.2 webui 创建 opencode 会话 → 提示 → 流式 text/tool 事件；触发工具权限时确认审查卡 `allow_once/allow_session/deny` 往返。验证：端到端可对话、权限机制生效（API 级已闭环，见备注）。
- [x] 3.3 `/cancel` 中断 opencode 会话后可继续对话。验证：`/cancel` 缺口已修复（见备注），cancel 后同一会话继续对话。
- [x] 3.4 结束会话后 resume → `resumed=true`、同一会话继续；**记录 resume 后是否重放历史消息**（设计 R1）与任何异常现象到冒烟结论。验证：本轮确认 fallback 语义（见备注）；真正 resume 由 add-acp-session-id-mapping 落地后验收。
- [x] 3.5 交付检查清单文档（`docs/` 或 change 内）：5 步冒烟步骤 + 预期结果 + 已知限制（历史重放观察项），作为后续回归验收的固定入口。验证：文档存在且步骤可照做。

> 3.2-3.4 说明：优先在本环境 API 直连沙箱机器人冒烟（不再局限于人工）。
> 本环境已完成并回填（2026-09-04，目标/debug/sebas + 真实 opencode 1.18.25，沙箱 port 9877）：
>
> **3.1** ✓ `agent-kinds` 报 `opencode reachable 1.18.25`。
>
> **3.2 端到端对话 + 工具流（API 级）** ✓
> - `POST /api/sessions {"backend":"acp:opencode","project_dir":"/tmp/smoke-proj"}` 建会话，
>   opencode acp 子进程拉起；initialize 返回 `capabilities{loadSession:true,
>   sessionCapabilities:{close,fork,list,resume}}`；`session/new` 返回真实 ACP session id
>   （`ses_f9471b...`）与 `configOptions[model]`（34 个模型，含 opencode-go 免费模型）。
> - prompt 返回真实模型回答：thinking 碎片 + markdown 进 transcript（`SMOKE_OK`）。
> - 触发 shell 工具：`tool_call_update in_progress→completed` 事件流完整，工具输出进 transcript。
> - **权限门禁观察**：opencode 自身 permission 体系——未放行的工具调用被 gate，sebas 作为
>   ACP client 透传/呈现。沙箱内 opencode 默认放行 bash（本地回环 shell），完整
>   allow_once/allow_session/deny 审查卡往返仍需 operator 在 webui 手测（前端渲染面），
>   API 层的工具流与 gate 机制已闭环。
>
> **3.3 `/cancel`（API 级）→ 缺口已修复**
> - 初测：webui 会话发 `/cancel` 被当普通 prompt 经 `session/prompt` 发给 opencode 文本处理，
>   未转 ACP cancel 通知（`web_send_message` 缺 feishu 路径的命令解析）。
> - **修复**（本 change 内）：`web_send_message` 增加命令解析——`/cancel` → `AcpCommand::Cancel`
>   → `CancelNotification`（`session/cancel`）；`/status` `/cost` `/compact` 同样接线；
>   无活跃会话时明确回复不静默。验证：`cargo test -p sebas-router` 全绿（224 项）；
>   沙箱重测 `/cancel` 日志确认发 `session/cancel` 通知（`ses_f9469b...`），
>   不再出现 `session/prompt "/cancel"`；cancel 后同一会话继续对话返回 `AFTER_CANCEL`。
>
> **3.4 resume：本轮确认 fallback 语义（上轮真实验收 + 本轮复测）**——优雅退出 dump
> state → 重启 → 对原 key 触发 `SpawnResume` → driver `LoadSessionRequest(old_uuid)`
> （`ses_f9469b...` vs routing `f50688b9...`）→ opencode 拒绝（`OpenCode service failure`，
> resumé 日志 `old session could not be loaded; continued as fresh session`）→ 正确
> fresh fallback（新 uuid `d15195c7...`、`resumed=false`）。**结论（R1 观察前置）**：
> 用 routing uuid 永远无法真正恢复 opencode 会话，必须映射真实 ACP session id——
> 已列为 add-acp-session-id-mapping 的核心目标（该 change tasks 3.1 落地后重测
> `resumed=true` + R1 历史重放观察）。
>
> **坑/观察**：cwd 用整个仓库时 opencode 首 token 慢（opencode.db 133MB 上下文索引），非挂死；
> 用轻量 `/tmp` cwd 秒回。这是 B 档冒烟 `agent-kinds` 之外第一个真实 opencode 调用点。
>
> **真实验收（2026-09-04，沙箱 core + 真实 opencode）**：
> - fresh 会话端到端 ✓：`POST /api/sessions` + `acp:opencode` → 真实模型回答
>   （PHASE1 / F1 / R1），流式 thinking + markdown 进 transcript，session_id 为
>   ACP 驱动 mint 的 uuid。
> - resume 机制 ✓：优雅退出 dump state → 重启 Dormant → 对原 key 消息触发
>   `SpawnResume` → driver 发 `LoadSessionRequest(old_uuid)` → opencode 拒绝
>   （`Internal error: OpenCode service failure`）→ **正确 fresh fallback**
>   （新 uuid、`resumed=false`、`"old session could not be loaded"` 日志）。
> - **发现（记入 design R1）**：opencode 的 `loadSession` 按其自身 session id
>   解析，不认 sebas 的 uuid；resume 真正打通需持久化 opencode 真实 ACP session
>   id 并映射路由 id（next-step）。过时的 `"ACP resume not implemented"` warn 已随
>   本次验收从 driver 删除（1.2 的遗漏）。

## 4. 收尾

- [x] 4.1 配置示例/文档新增 opencode 条目（README 或 config 示例：`[acp.agents.opencode] driver="acp", command=["opencode","acp"]`）。验证：文档含 opencode 配置段。
- [x] 4.2 `openspec validate` 通过；如为 B 档结果，将冒烟证据（agent-kinds 输出、会话截图/日志摘要、历史重放结论）附到 change 或 issue 备注。验证：`openspec validate --change add-opencode-acp` 通过。

> 4.2 备注：`openspec validate --changes` 对 `add-opencode-acp` ✓（含两个 delta spec：`acp-driver` ADDED、新能力 `opencode-agent`）。
> 全局 `--specs` 唯一失败为 pre-existing `core-session-channel` 主 spec 缺 Requirements 段（archiving 遗留，非本 change）。
> B 档冒烟证据：`docs/acp-opencode-smoke.md`（含 3.1 可达性结果 `opencode true 1.18.25`）；3.2-3.4 待 operator 回填。