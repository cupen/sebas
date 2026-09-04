# opencode ACP 接入——B 档冒烟清单（人工/半自动验收）

> 来源：`openspec/changes/add-opencode-acp`（design D4 B 档 + tasks 3.x）。
> A 档（自动化回归）已由 `cargo test -p sebas-acp`（含 `tests/acp_resume_test.rs`）
> 与 `sebas-webui::agent_kinds` 单测覆盖；本文档是**真实 opencode** 的环境验收，
> 需要：opencode 二进制可达、opencode 已登录 provider（`~/.local/share/opencode/auth.json`）、
> operator 的 webui（`sebas-webui/frontend` dev server，127.0.0.1:5273）可用。

## 前置

```bash
# 1. 配置（追加到 sebas 配置文件的 [acp] 段）
[acp.agents.opencode]
driver = "acp"
command = ["opencode", "acp"]

# 2. 可达性（纯探测，无副作用）
sebas agent-kinds list -c <config>
# 期望：opencode true <裸版本号>
```

> 已在本机验证（2026-09-04）：`opencode true 1.18.25`。

## 步骤与预期

| # | 操作 | 预期 |
|---|------|------|
| 1 | 配置 opencode 条目 + `agent-kinds list` | `opencode true <version>`；无二进制时 `false command not found` |
| 2 | webui 创建会话下拉选 opencode → 发提示 | 流式 text 增量（及 tool start/end 当有工具调用）；事件与 claude 会话同表面 |
| 3 | 触发工具权限（如让 agent 执行 shell） | 审查卡出现，`allow_once`/`allow_session`/`deny` 均可用；答复回传 opencode |
| 4 | `/cancel` 中断一个进行中的回合 | 会话可继续（opencode 支持 cancel 通知）。<br>✅ **已修复**（add-opencode-acp 2026-09-04）：webui 直达路径补上了命令解析，`/cancel` 现在发 ACP `session/cancel` 通知；修复后 cancel 同一会话继续对话正常（`AFTER_CANCEL`）。feishu 路径本就正常。 |
| 5 | 结束/重启后 resume 该会话（webui 或命令） | `resumed=true`、同一会话继续；**记录 resume 后是否重放历史消息**（R1） |

## 已知限制与观察项

- **R1 历史重放**：opencode `session/load` 会把历史消息作为 `agent_message_chunk` 重放
  给客户端。若 webui 出现"resume 后旧消息重复渲染"，记录现象——本期不预实现抑制，
  留后续 change（design R1）。
- **`/cancel`（webui 直达路径缺口，已修复）**：曾被当普通 prompt 经 `session/prompt` 发给
  opencode 文本处理（不转 ACP cancel 通知）——webui 消息路由（`web_send_message`）缺
  feishu 路径的命令解析。**已在 add-opencode-acp 内修复**：`web_send_message` 补命令解析，
  `/cancel` → `session/cancel` 通知；`/status` `/cost` `/compact` 同接线；无活跃会话明确回复。
- **凭据**：opencode 用其自身登录态（`auth.json`），不经 sebas 的 provider 注入。
  sebas 的 `extra_env`（Direct/Gateway 模式）对 opencode 不注入 `OPENAI_BASE_URL` 等。
- **进程生命周期**：opencode acp 在 stdin 关闭时退出；sebas kill 会话即关闭 stdin。
- **首 token 延迟**：opencode 以 `cwd` 建上下文索引（`opencode.db` 可能上百 MB）；
  大型 cwd 下首 token 明显变慢，非挂死（沙箱实测：全仓库 cwd 数十秒，`/tmp` 秒回）。

## 结果记录

冒烟完成后，把下列证据附到 change/issue 备注：
- `agent-kinds list` 输出
- 会话截图或事件日志摘要（text/tool/权限卡各一）
- `/cancel` 与 resume 的观察（尤其 R1 历史重放结论）

### resume 已真正打通（add-acp-session-id-mapping，2026-09-04）

真实验收证明「路由 id ↔ 真实 ACP session id」映射已打通 resume：

1. `agent-kinds list -c <sandbox-config>`：`opencode true 1.18.25`。
2. fresh 建会话（`POST /api/sessions`，backend `acp:opencode`，`project_dir=/tmp/sebas-proj`）：
   `session/new` 返回真实 ACP id `ses_f9442373affeUr2hQiOnmaocgc`；首 prompt 正常回复。
3. 优雅 SIGTERM → `sessions.json` 含 `"acp_session_id":"ses_f9442373affeUr2hQiOnmaocgc"`
   （映射落盘，重启后 resume 可读）。
4. 重启 core → 对原 key 发消息触发 resume → 日志：
   `ACP session/load target resolved kind=opencode routing_id=76e7beea-857d-4dab-b6db-c0e889467d2e load_target=ses_f9442373affeUr2hQiOnmaocgc`，
   且发出的 `session/load` 携带 `"sessionId": "ses_f9442373affeUr2hQiOnmaocgc"`（**真实 id，非路由 uuid**）。
5. resume 结果：prompt 正常回复；`starting a fresh session` 日志 0 次；无 `OpenCode service failure`；
   routing id 保留（`resumed=true`）→ 会话真正恢复。

**R1 历史重放观察**：本次 resume 的 prompt 为单 turn 无工具调用，opencode 在 load 后未重放
历史文本块（核心日志 0 条 `agent_message_chunk`）。未观察到重复渲染；是否在长会话/多轮
prompt 下重放留后续 change 观察（Open Question）。

**既有观察（非 add-acp-session-id-mapping 引入）**：webui resume 路径未携带 `project_dir`，
load 的 `cwd` 回落进程目录（`work_dir_for` 对 ACP agent 恒 `None`）。

### 模型选择已打通（add-acp-model-selection，2026-09-04）

真实验收（沙箱 9877，`--webui`）证明「configOptions → 模型下拉 → `session/set_config_option` → 快照 current_model 更新」链路完整：

1. **spawn outcome 携带模型列表**：`POST /api/sessions {backend:"acp:opencode", project_dir:"/tmp/sebas-proj"}` → `GET /api/sessions/<key>` 详情返回
   `current_model: "opencode/big-pickle"` + `available_models`（**34 个 opencode 模型**，含 free 套餐）——数据源是 agent 的
   `session/new` 响应的 `configOptions`，非硬编码。
2. **create-with-model**：`POST {..., "model":"opencode/nemotron-3.5-lightning-free"}` → 会话建立后首 prompt 前应用，
   快照 `current_model` = 请求的模型（有效）。
3. **中程切换成功**：`POST /api/sessions/<key>/model {"model_id":"opencode/mimo-v2.5-free"}` → driver 发出
   `session/set_config_option {configId:"model", value:"opencode/mimo-v2.5-free"}`（用**真实 ACP 会话 id**
   `ses_...` 寻址），opencode 接受 → 事件 `ModelChanged` → 快照 `current_model` 更新为 `opencode/mimo-v2.5-free`，
   transcript 留 `⚙ model → ...` 行。
4. **无效模型显式拒绝**：`{"model_id":"not-a-real-model"}` → opencode 回复 `Invalid params: model not found: ...` →
   driver 发**非 terminal** `Error`（transcript `❌ set model "..." failed: agent 拒绝设置模型（会话仍使用原模型）: ...`），
   `current_model` **不变**（`opencode/mimo-v2.5-free`）。
5. **create-with-invalid 非致命**：带 `bogus-model-xyz` 创建 → 会话仍建立、首 prompt 正常，transcript 呈现非致命错误行。
6. **set_config_option 返回值语义（Open Question 落地）**：opencode 的 `session/set_config_option` 成功响应回显
   `configOptions`（含最新 `currentValue`）——driver 正是从响应 `configOptions` 刷新本地 current model。
   **前端体验**：无模型选项的 agent（Claude 驱动 / mock 无 configOptions）→ `available_models` 为 null → 不显示模型下拉、无错误。