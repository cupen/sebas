# ACP 驱动 opencode 验收记录（2026-09-04）

> 验收人：sebas-agent 会话；沙箱 `/tmp/sebas-acp-accept`，webui 端口 9879。
> 背景：自 `main` 上已合并 ACP 三 change（add-opencode-acp / add-acp-session-id-mapping /
> add-acp-model-selection）之后的**独立复验**，确认代码正确性。

## 环境

- opencode 1.18.25（`/usr/sbin/opencode`），登录态：仅 **OpenCode Go api**（无 OpenCode Zen）。
- sebas：`target/debug/sebas`（本会话重新 `cargo build`）。
- 沙箱配置：`[acp.agents.opencode] driver="acp" command=["opencode","acp"]`；
  `[log] level="debug"`（**关键**：`sebas run` 用 `cfg.log.level`，不是 `RUST_LOG` 环境变量——
  `RUST_LOG=sebas_acp=debug` 无效，必须在 config 加 `[log]` 段）。

## 验证结果（全部通过）

### 1. 可达性
- `GET /api/agent-kinds` → `opencode true 1.18.25`。

### 2. spawn 全链路（fresh 会话）
日志（`agent_client_protocol` DEBUG 级）证实：

1. `initialize` → `agentCapabilities.loadSession:true`、`sessionCapabilities:{close,fork,list,resume}`。
2. `session/new` → `cwd:"/tmp/sebas-acp-accept/tiny"`（project_dir 透传成功）+ **34 个模型**
   configOptions，`currentValue:"opencode/big-pickle"`（默认）。
3. `session/prompt` → 用 **ACP 真实 id**（`ses_...`，非 routing id）发送用户文本。

### 3. 模型选择（add-acp-model-selection）
创建时带 `"model":"opencode-go/deepseek-v4-flash"`：

1. `session/new` 后发 `session/set_config_option {configId:"model", value:"opencode-go/deepseek-v4-flash"}`。
2. 响应 `currentValue` 变为 `opencode-go/deepseek-v4-flash` → **切换成功**；
   summary 的 `current_model` 同步更新。
3. 之后 `session/prompt` 用切换后的模型生成，`usage_update`/`stopReason:end_turn` 正常。

### 4. 完整一轮对话（决定性证据）
免费模型 `opencode-go/deepseek-v4-flash`（因默认 big-pickle 在 opencode 侧**无登录态**会挂起——见下）：

- 消息 `"Reply with exactly the word PONG-FREE-ACP..."` → 流式返回
  `agent_message_chunk`：`PONG-FREE-` + `ACP`（拼接为 `PONG-FREE-ACP`）✓
- `usage_update`：`11342 tokens, cost $0.0025` ✓
- `stopReason: end_turn` ✓
- opencode 日志：`stream modelID=deepseek-v4-flash` → `loop step=1` → `exiting loop` ✓

### 5. 工具调用 + 权限评估（长任务）
`"List all prime numbers up to 100000..."`：

- opencode 发起 `bash` 工具调用，`evaluated permission=bash pattern="python3 -c ..."`（默认策略放行）。
- 素数结果经 `agent_message_chunk` 流式回传（`33409 33413 ...` 等）。✓

### 6. resume（add-acp-session-id-mapping）
1. 优雅 SIGTERM core → `sessions.json` 落盘：
   `{"web-...":{"session_id":"db53d32d-...","acp_session_id":"ses_f93934b27ffen6uTVdCj81JVaP",
   "current_model":"opencode-go/deepseek-v4-flash"}}` ✓（映射 + 模型都持久化）
2. 重启 core → 会话 `dormant`（从磁盘恢复）。
3. 对原 key 发消息触发 resume → 日志：
   `ACP session/load target resolved kind=opencode routing_id=db53d32d-... load_target=ses_f93934b27ffen6uTVdCj81JVaP`
   → 发出的 `session/load` 携带 **`sessionId: ses_f93934b27ffen6uTVdCj81JVaP`（真实 ACP id）** ✓
4. load 成功返回 configOptions（`currentValue` 保持切换后的免费模型）→ `session/prompt`
   → `stopReason:end_turn`，一轮回复完成。✓

## 发现的问题 / 观察项

### P1: opencode 默认模型 `opencode/big-pickle`（OpenCode Zen）在本机无登录态 → 挂起
- 现象：`session/prompt`（big-pickle）后 opencode 停在 `stream ... model=big-pickle`，
  无任何回复（60s+）；sebas 侧会话一直 active。
- 判定：**非 sebas 缺陷**——普通 opencode 会话也存在；用免费模型 `opencode-go/deepseek-v4-flash`
  秒回（CLI 与 sebas ACP 桥均如此）。
- 影响：`[acp.agents.opencode]` 无 `default` 时，config 迁移逻辑会把唯一 agent 设为默认
  kind，而 opencode 默认 model 即 big-pickle。**建议**：首次使用的用户会被默认模型挂起，
  需在文档/change 里显式提示配置 `model` 或在 opencode 里 `auth login` OpenCode Zen。

### P2: 0-turn 会话（无 prompt 创建）会把空串当 prompt 发给 opencode → 挂起
- 现象：`POST /api/sessions` 不带 `prompt` 时，日志出现
  `session/prompt { text: "" }`（空 prompt），opencode 挂起不回复。
- 根因：`api.rs` 注释说「无 prompt = 0-turn 占位，不 spawn」，但实现 `let prompt =
  req.prompt.unwrap_or_default();` 直接把空串送入 spawn 流程，未走占位分支。
- 影响：webui「新建会话」如果允许不带 prompt 创建，会话创建即卡住。
- 建议：空 prompt 时应创建 0-turn 占位（不 spawn 子进程），首条消息到达后再 spawn。

### P3: `close` 会话后 opencode 子进程残留（低优先级）
- 现象：`POST /api/sessions/{key}/close` 返回 `closed`，但 opencode 子进程仍在。
- 根因：`web_close_session` 调了 `mgr.kill`（发 cancel 信号），但 ACP 驱动在
  `cancel_rx → break` 后**依赖 `AcpAgent` drop 关闭 stdin**；opencode 对 stdin 关闭
  不敏感（可能在等网络），子进程悬挂。
- 影响：沙箱里多次 close 会累积僵尸 opencode 进程。
- 建议：kill 时显式关闭 stdin 或对 opencode 用 `kill`/进程组终止（低优先级）。

### 观察: `backend` 参数解析
- `spawn_with` 接收 `backend` hint，`parse_acp_kind` 只认 `acp:<slug>` 前缀；
  `"opencode"`（无前缀）→ `None` → 走默认 kind。实测由于 config 里 opencode 是唯一
  agent（自动成为默认），`backend="opencode"` 与 `backend="acp:opencode"` 效果相同。
  但若配置了多个 agent，`"opencode"` 会落到默认 kind 而非 opencode——**前端应传
  `acp:opencode`**。非缺陷，记录备查。

## 如何复现（给后续会话/operator）

```bash
mkdir -p /tmp/sebas-acp-accept/tiny && cd /tmp/sebas-acp-accept/tiny
cat > /tmp/sebas-acp-accept/config.toml <<'EOF'
[router]
state_file = "/tmp/sebas-acp-accept/sessions.json"
[media]
download_dir = "/tmp/sebas-acp-accept/downloads"
[watchdog.core]
channel_path = "/tmp/sebas-acp-accept/core.sock"
[watchdog.webui]
host = "127.0.0.1"
port = 9879
[feishu]
enabled = false
[acp.agents.opencode]
driver = "acp"
command = ["opencode", "acp"]
[log]
level = "debug"
EOF
printf '{"providers":{}}' > /tmp/sebas-acp-accept/providers.json
printf '{}' > /tmp/sebas-acp-accept/state.json
SEBAS_CORE_SECRET=fake SEBAS_STATE_FILE=/tmp/sebas-acp-accept/state.json \
  SEBAS_GATEWAY_PROVIDER_OVERLAY=/tmp/sebas-acp-accept/providers.json \
  SEBAS_AGENT_PROVIDER_API_KEY=fake-key \
  target/debug/sebas run -c /tmp/sebas-acp-accept/config.toml --webui --webui-port 9879
# 建会话（带免费模型以避免 big-pickle 挂起）：
curl -X POST localhost:9879/api/sessions -H 'Content-Type: application/json' \
  -d '{"prompt":"Reply with exactly: PONG","project_dir":"/tmp/sebas-acp-accept/tiny",
       "backend":"acp:opencode","model":"opencode-go/deepseek-v4-flash"}'
```

## 修复验证（同日第二轮，双进程形态：`sebas run` + `sebas webui`）

P2/P3 按上述「建议」修复后复验。注意 P2 修复比首轮建议多覆盖了一层：
**wire 路径**（`CoreChannelBackend`）原本没有占位语义，trait 默认实现会把空串
当 prompt 走 `CoreChannelRequest::Spawn` 发给 core——正是双进程部署（watchdog
形态）下必踩的路径。故新增 `CoreChannelRequest::CreatePlaceholder{project_dir,
model}`（client 覆写 + server 处理器，project_dir 校验与 Spawn 同款；kind 与
Spawn 一致钉 core 默认 agent，model 透传）。

沙箱验证（opencode 为唯一 agent 即默认 kind，免费模型）：

1. **P2（0-turn 占位）**：`POST /api/sessions` 不带 prompt → 立即返回 key，
   `/api/sessions` 出现 `spawning` 行（带 project_dir），**无 opencode 子进程**；
   首条消息 → 子进程才 spawn，`current_model` = 创建时请求的
   `opencode-go/deepseek-v4-flash`（model 经 wire 一路带到 spawn），真实回复
   `PONG-WIRE-P2` 流式回传。wire 单测
   `create_placeholder_wires_a_zero_turn_session` 断言占位不发 `Out::WebSpawn`、
   首条消息触发 SpawnNew 且携带 model。
2. **P3（close 收割）**：spawn 后记录子进程 pid，`POST …/close` → pid 消失
   （进程组收割）。单测 `kill_reaps_child_process` 以 /proc 扫描断言 mock 子
   进程在 kill 后不复存在。
3. **优雅退出**：SIGTERM core → `core.sock` 移除、`sessions.json` 落盘（close
   后为 `{}`：closed 映射已移除，Spawning 占位从不落盘），端口释放。

观察更正：首轮记录的「0-turn 占位列表不可见」实为未修复时的表现（空 prompt
直发 spawn、native claude 无凭据即死、映射被清理）；修复后占位以 `spawning`
状态正常出现在列表中。