# gateway — Provider Router（LLM 网关）设计文档

> 日期：2026-08-06
> 状态：已评审（计划经用户确认）
> 作者：Claude（与 cupen 协作）

## 1. 背景与目标

开发一个 LLM provider router：对外同时暴露 **Anthropic** 与 **OpenAI** 两套 API 协议面，按**模型名**把请求路由到对应上游 provider。作为 sebas workspace 的新 crate `gateway` 落地（`router` 名字已被聊天消息路由器占用），同时为 sebas 自身（Claude Code 流量经 `ANTHROPIC_BASE_URL` 接入）和任意外部客户端服务。

设计目标：

1. **双协议面 100% 覆盖** —— Anthropic 与 OpenAI 官方 API 的全部端点可路由、字节无损，分期交付，覆盖率由 spec-diff CI 机械保证。
2. **纯透传** —— 同协议转发，不改写协议体；Anthropic 格式请求只去 Anthropic 协议的 provider，OpenAI 同理。
3. **按模型名路由** —— 路由表 + `provider/model` 命名空间 + 默认回退；model rename 属于路由层职责。
4. **生产可用的最小闭环** —— P0 即含鉴权、限流/配额、用量统计。

非目标：

- Anthropic↔OpenAI 协议转换（用户已明确排除，仅记录为未来方向）
- 多租户计费系统、Web 管理后台
- 语义缓存（P2 候选）

**已锁定的关键决策（与用户确认）：**

| 决策点 | 结论 |
|---|---|
| 定位 | sebas 同仓新 crate（Rust + axum），`sebas gateway` 子命令运行 |
| 转发模式 | 纯透传（同协议转发） |
| 覆盖范围 | 全量端点，分期交付（P0 推理核心 → P1 全量 + 管理面 → P2 难传输层） |
| P0 附加能力 | 鉴权+key 管理、用量统计+可观测性、限流/配额（故障转移放 P1） |
| 架构 | 通用透传引擎 + 特例处理（覆盖率结构性达成，而非逐端点手写） |

## 2. 端点面调研（权威来源）

- **OpenAI**：官方 OpenAPI spec（github.com/openai/openai-openapi）共 **182 个路径**——inference / files / uploads / batches / fine-tuning / assistants v2 / vector_stores / realtime / images / audio / videos / evals / containers / conversations / chatkit / skills / organization admin（约 60 个）。完整清单见附录 B。
- **Anthropic**：无公开 OpenAPI spec，以官方 SDK 资源清单（anthropic-sdk-typescript `api.md`）+ platform.claude.com API reference 为准，约 **120 个端点**——messages / count_tokens / batches / models / files / skills / managed agents beta 全家桶（agents, sessions, events, threads, environments, deployments, vaults, memory_stores, user_profiles, tunnels, dreams）。完整清单见附录 A。

## 3. 总体架构

```
客户端（Claude Code / anthropic SDK / openai SDK / curl）
   │  ANTHROPIC_BASE_URL=http://host:8787   或   OPENAI_BASE_URL=http://host:8787
   ▼
┌─ gateway crate ────────────────────────────────────────────────┐
│ 1. 协议识别 proto      anthropic-version header + 路径表嗅探      │
│ 2. 鉴权 auth           下游 key 校验（Bearer / x-api-key）        │
│ 3. 限流 quota          令牌桶 RPM + token 配额记账                │
│ 4. 路由 routing        提取 model → 解析 provider → model rename │
│ 5. 透传引擎 proxy      重写 auth header → 流式转发（SSE 不缓冲）   │
│ 6. usage tee           SSE/JSON 中增量解析 usage → jsonl 记录     │
└────────────────────────────────────────────────────────────────┘
   │ 同协议转发（Anthropic 协议面 / OpenAI 协议面）
   ▼
上游 provider（anthropic / openai / deepseek / kimi / glm / ...）
```

### Crate 结构（新增 workspace member `gateway/`）

```
gateway/
  Cargo.toml
  src/
    lib.rs
    config.rs    # [gateway] 配置模型、env 覆盖、校验（house style 对齐 src/config.rs）
    server.rs    # axum 启动、挂载、中间件栈、优雅退出
    proto.rs     # 协议识别：header 嗅探 + 路径表 + 显式前缀挂载
    routing.rs   # model 提取、路由表（精确/glob/命名空间/默认）、model rename
    proxy.rs     # 透传引擎：header 改写、流式转发、SSE tee、超时/取消
    sse.rs       # 两协议 SSE 增量解析器（仅提取 usage，容忍未知事件）
    auth.rs      # 下游 key 鉴权中间件
    quota.rs     # 内存令牌桶 + 配额记账
    usage.rs     # usage record → jsonl sink（P1: metrics、成本估算）
    error.rs     # 网关自身错误 → 按协议面输出对应错误格式
  tests/
    contract/    # 表格驱动 contract tests（每端点：请求原样到上游、响应原样返回）
    support/     # mock upstream（axum）、fake keys、SSE fixtures
```

运行方式：root bin 增加子命令 `sebas gateway`（对齐 `src/cli.rs` 现有 run/replay/record 范式），配置走同一 `config.toml` 的 `[gateway]` 段。

## 4. 核心设计

### 4.1 协议识别（同端口双协议面）

两套协议的路径有**碰撞**（`/v1/models`、`/v1/files`、`/v1/skills` 两边都有），解决：

- **主路径：bare `/v1/*` 挂载 + 嗅探**。Anthropic 客户端（SDK / Claude Code）必带 `anthropic-version` header → 判为 Anthropic 协议；否则按 OpenAI 路径表判定。碰撞路径由 header 仲裁。
  - 必须支持 bare `/v1/*`：`ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` 都是直接指根路径，这是 sebas 自身（Claude Code 流量）接入的方式。
- **辅助路径：显式前缀挂载** `/anthropic/v1/*` 与 `/openai/v1/*`，无歧义，供显式配置使用。

### 4.2 路由（model 名 → provider）

解析优先级：`provider/model` 命名空间 > 精确匹配 > glob 前缀 > key 级默认 > 全局默认。

- **model 提取**：POST JSON body 的 `model` 字段（body 缓冲重放——LLM 请求体 MB 级，可接受）；无 body 端点（GET/DELETE）从路径参数（如 `/v1/models/{model}`）或回退默认 provider；multipart 端点跳过提取走默认/路径规则。
- **协议约束**：解析到的 provider 协议面必须与请求协议一致，否则返回明确错误（纯透传原则，不做协议转换）。
- **model rename**（路由层职责，不算协议转换）：如 Bedrock 要 `anthropic.claude-*` 前缀、Azure 把 model 放进 URL path。provider 级 `model_map` 配置。

配置示例：

```toml
[gateway]
listen = "127.0.0.1:8787"

[[gateway.keys]]                    # 下游客户端 key
key = "sk-gw-local-dev"
name = "claude-code"
rpm = 600                           # 限流：每分钟请求数
daily_token_quota = 50_000_000      # 每日 token 配额
allow_models = ["claude-*", "deepseek-*"]

[gateway.providers.anthropic]       # 上游 provider
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"   # 密钥从 env 读，不落盘、不落日志

[gateway.providers.deepseek]
protocol = "anthropic"              # DeepSeek 的 Anthropic 兼容端点
base_url = "https://api.deepseek.com/anthropic"
api_key_env = "DEEPSEEK_API_KEY"

[gateway.providers.deepseek-oai]
protocol = "openai"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"

[[gateway.routes]]                  # 路由表（按序匹配）
model = "claude-*"
provider = "anthropic"

[[gateway.routes]]
model = "deepseek-*"
provider = "deepseek"

[[gateway.routes]]
model = "gpt-*"
provider = "openai"
```

### 4.3 透传引擎（axum + reqwest）

- **请求**：剔除 hop-by-hop headers 与下游 key，注入上游 `x-api-key`/`Authorization`；body 流式转发。
- **响应**：status/headers/body 流式回传，**SSE 逐 chunk flush 不缓冲**；记录 TTFT。
- **超时**：connect 10s；读/总超时放宽到分钟级（LLM 长请求）；客户端断开联动 cancel 上游。
- **错误**：上游错误的 status+body **原样透传**；网关自身错误（401 鉴权 / 429 限流 / 502 无路由）按协议面输出对应格式：
  - Anthropic: `{"type":"error","error":{"type":"authentication_error","message":"..."}}`
  - OpenAI: `{"error":{"message":"...","type":"invalid_request_error","code":...}}`

### 4.4 特例处理（通用引擎上的 SPI 点）

| 特例 | 方案 | 期 |
|---|---|---|
| body 提取 model | 缓冲重放（上限保护） | P0 |
| **usage 统计** | SSE tee：字节透传的同时增量解析事件提取 usage（Anthropic: `message_start`/`message_delta`；OpenAI: 末尾 chunk `usage`，非流式解析响应 JSON）。**解析失败不影响透传** | P0 |
| multipart 大文件（files/uploads/audio） | body 流式不缓冲；路由走默认 provider/路径规则 | P1 |
| WebSocket（OpenAI Realtime） | axum WS upgrade + tokio-tungstenite 双向帧转发 | P2 |
| Admin API（`/v1/organization/*` 等） | 鉴权模型不同（admin key），独立 provider 配置项 | P2 |

### 4.5 鉴权 / 限流 / 用量（P0 三大附加能力）

- **鉴权**：静态 key 表（P0，TOML 配置）→ 管理 API 签发/吊销 + sqlite 持久化（P1）。中间件提取 `Authorization: Bearer` 或 `x-api-key`。
- **限流**：单进程内存令牌桶：per-key RPM + 每日 token 配额（usage 事后记账）。超限按协议格式返回 429 + `retry-after`。
- **用量**：每请求一条 record：`{ts, key, protocol, model, provider, upstream_model, status, latency_ms, ttft_ms, input_tokens, output_tokens, cache_read/creation_tokens, error}` → jsonl 落盘（`~/.local/state/sebas/gateway-usage.jsonl`）+ tracing。Prometheus metrics 与成本估算（provider 价格表）放 P1。

### 4.6 "100% 覆盖"的验证策略

纯透传下覆盖率是**结构性达成**的（任意路径都能转发），需要验证的是"每个官方端点路由正确、字节无损"：

1. **contract test**：mock upstream 记录入站请求，对每个端点断言 ① 方法/路径/headers/body 原样到达 ② 上游响应（含 SSE）原样返回 ③ 路由解析正确。表格驱动参数化，一个 case 一行。
2. **spec-diff CI**：CI 拉取官方 spec（OpenAI openapi.yaml；Anthropic SDK api.md）生成端点清单，与测试覆盖矩阵 diff——未覆盖端点必须显式标注 `pending`（含原因），否则 CI 失败。防止"自以为覆盖了"以及上游新增端点掉队。
3. 覆盖率指标纳入 `scripts/check_coverage.sh` ratchet。

## 5. 分期计划

### P0 — 骨架 + 推理核心 + 三大能力

1. `gateway` crate 骨架 + `sebas gateway` 子命令 + `[gateway]` 配置加载（含 env 覆盖、校验）
2. 协议识别（嗅探 + 显式前缀挂载）
3. 路由表（glob、命名空间、默认回退、model rename）
4. 通用透传引擎：非流式 + SSE 字节透传、超时/取消、错误格式
5. model 提取（body 缓冲重放）
6. 鉴权中间件（静态 key）
7. 限流（RPM + 日 token 配额）
8. usage tee + jsonl 记录
9. contract test 框架 + mock upstream；P0 端点清单全绿：
   - Anthropic：`POST /v1/messages`（流式/非流式）、`POST /v1/messages/count_tokens`、`GET /v1/models`、`GET /v1/models/{id}`
   - OpenAI：`POST /v1/chat/completions`（流式/非流式）、`POST /v1/responses`、`POST /v1/embeddings`、`GET /v1/models`、`GET /v1/models/{id}`
10. spec-diff CI 骨架（P0 清单 + 其余标注 pending）
11. e2e 验证：`ANTHROPIC_BASE_URL` 指向网关跑通 Claude Code；openai SDK 同理

### P1 — 全量端点 + 管理面 + 故障转移

1. 端点全覆盖 contract tests（表格驱动）：Anthropic 全量 + OpenAI 全量（182 paths；realtime WS 与 organization admin 标注 P2）
2. multipart 大文件流式透传（files/uploads/audio transcriptions）
3. 故障转移 + 多 key 负载均衡（同模型多 provider/key，429/5xx/超时切换，透传上游 retry-after）
4. 管理 API（key 签发/吊销/用量查询）+ sqlite 持久化
5. Prometheus `/metrics` + 成本估算（可配置价格表）
6. Bedrock/Vertex/Azure 形态适配（model rename、URL path 注入、SigV4 如需）

### P2 — 难传输层 + 高级能力

1. WebSocket 透传（OpenAI Realtime；预留 Anthropic WS）
2. Admin API 面（OpenAI `/v1/organization/*` 约 60 端点；Anthropic admin）
3. 虚拟模型/别名路由（如 `auto`、`cheap` → 策略链）
4. 语义缓存、请求改写钩子
5. （仅记录方向，用户已明确不做）Anthropic↔OpenAI 协议转换

## 6. Provider 格局调研

### 海外知名 provider

| 类别 | 厂商 |
|---|---|
| 第一方前沿 | **OpenAI**、**Anthropic**、**Google**（Gemini / Vertex AI）、**xAI**（Grok）、**Mistral**、Cohere、Meta（Llama，经托管商） |
| 云托管平台 | Azure OpenAI / Microsoft Foundry、AWS Bedrock、Google Vertex AI |
| 推理云 | Groq、Together AI、Fireworks、Cerebras、SambaNova、DeepInfra、Morph |
| 聚合网关 | OpenRouter、Hugging Face Inference、Portkey、LiteLLM |
| 其他 | Perplexity、Cloudflare Workers AI、Replicate、CoreWeave |

### 国内知名 provider

| 类别 | 厂商 |
|---|---|
| 第一方 | **DeepSeek**、**阿里通义**（百炼 DashScope）、**字节豆包**（火山方舟）、**智谱 GLM**（BigModel）、**Moonshot Kimi**、**MiniMax**、阶跃星辰、百度文心（千帆）、腾讯混元、讯飞星火、商汤日日新、百川、零一万物、华为盘古 |
| 聚合 | 硅基流动 SiliconFlow、OpenRouter |

### 协议面支持矩阵（纯透传模式下决定能接谁）

| Provider | Anthropic 协议端点 | OpenAI 兼容端点 |
|---|---|---|
| Anthropic（第一方 / Bedrock Mantle / Vertex / Foundry） | ✅ | — |
| OpenAI（第一方 / Azure） | — | ✅ |
| DeepSeek | ✅ `api.deepseek.com/anthropic` | ✅ |
| Moonshot Kimi | ✅ `api.moonshot.cn/anthropic` | ✅ |
| 智谱 GLM | ✅ `open.bigmodel.cn/api/anthropic` | ✅ |
| MiniMax | ✅ `api.minimaxi.com/anthropic` | ✅ |
| 通义 DashScope | — | ✅ compatible-mode |
| 豆包（火山方舟） | ✅ `ark.cn-beijing.volces.com/api/coding` | ✅ `ark.cn-beijing.volces.com/api/coding/v3` |
| Google Gemini | — | ✅ openai-compat |
| xAI / Mistral / Groq / Together / Fireworks / OpenRouter / SiliconFlow | — | ✅ |

> 要点：国产主力（DeepSeek/Kimi/GLM/MiniMax/豆包）都提供 **Anthropic 兼容端点**，纯透传即可让 Claude Code 流量直达国产模型，无需协议转换。
>
> P0 实施期核实结论：
> - **豆包（火山方舟）**：官方文档《接入 AI 工具 / Claude Code》（docs.volcengine.com/docs/82379/1928262，2026-08-04 更新）明确给出 Anthropic 协议 base `https://ark.cn-beijing.volces.com/api/coding`（用作 `ANTHROPIC_BASE_URL`）与 OpenAI 协议 base `.../api/coding/v3`，并注明「请勿使用 `.../api/v3`」。结论：两端点均有。
> - **通义 DashScope**：未提供 Anthropic 协议端点。endpoint 探测（2026-08-07）：`POST /compatible-mode/v1/messages`、`/anthropic/v1/messages`、`/v1/messages` 均 404；`/compatible-mode/v1/chat/completions` 返回 OpenAI 格式 401（`invalid_api_key`）。DashScope 仅提供 OpenAI 兼容端点 `compatible-mode/v1`，Anthropic 协议面无对应路径。结论：Anthropic 端点无。

## 7. 关键风险与技术难点

1. **长请求**：LLM 请求分钟级，读超时必须放宽；客户端断开要联动取消上游（axum/reqwest drop 语义）。
2. **SSE tee 健壮性**：解析器必须容忍未知事件类型与截断帧——解析失败只丢 usage 统计，绝不影响透传字节流。
3. **multipart 与 model 提取的缓冲策略冲突**：multipart 端点跳过 body 提取。
4. **429 语义区分**：网关自身限流 429 vs 上游 429 透传（保留上游 `retry-after`）。
5. **header 改写正确性**：`host`/`content-length`/`authorization`/`x-api-key` 必须重写，`anthropic-beta`、`anthropic-version` 等业务 header 原样透传。
6. **命名隔离**：新 crate `gateway` ≠ 现有 `router` crate；配置段 `[gateway]` ≠ `[router]`。

## 8. 验证方式

- `cargo test --workspace`（含 gateway contract tests）
- spec-diff CI 全绿（P0 清单 covered，其余显式 pending）
- 手动 e2e：
  ```bash
  sebas gateway --config config/config.toml
  ANTHROPIC_BASE_URL=http://127.0.0.1:8787 claude   # 经网关到 anthropic，验证流式 + 权限卡
  OPENAI_BASE_URL=http://127.0.0.1:8787 openai api chat.completions.create -m gpt-5 ...
  curl http://127.0.0.1:8787/v1/models -H "x-api-key: sk-gw-local-dev" -H "anthropic-version: 2023-06-01"
  ```
- 用量验证：请求后检查 `gateway-usage.jsonl` 记录完整（含 token 数、TTFT）

## 附录 A：Anthropic 端点清单（约 120，来自官方 SDK api.md）

**GA 核心**：`POST /v1/messages`、`POST /v1/messages/count_tokens`、`GET /v1/models`、`GET /v1/models/{model_id}`
**Message Batches**：`POST /v1/messages/batches`、`GET /v1/messages/batches`、`GET /v1/messages/batches/{id}`、`POST .../cancel`、`GET .../results`、`DELETE .../{id}`
**Files（beta）**：`POST /v1/files`、`GET /v1/files`、`GET /v1/files/{id}`、`GET /v1/files/{id}/content`、`DELETE /v1/files/{id}`
**Skills（beta）**：`POST /v1/skills`、`GET /v1/skills`、`GET /v1/skills/{id}`、`DELETE /v1/skills/{id}` + versions 子资源 4 个（POST/GET/GET/DELETE `/v1/skills/{id}/versions[/{version}]`）+ `GET .../versions/{version}/content`
**Managed Agents（beta，全部带 `?beta=true` 变体）**：
- agents：list/create/get/update/archive + `GET /v1/agents/{id}/versions`
- sessions：list/create/get/update/delete/archive + events（list/send/stream）+ threads（list/get/archive/events/stream）+ resources（list/add/get/update/delete）
- environments：CRUD/archive + work（poll/stats/get/ack/heartbeat/stop）
- deployments：create/list/get/pause/unpause/archive/run + deployment_runs（list/get）
- vaults：CRUD/archive + credentials（CRUD/archive/mcp_oauth_validate）
- memory_stores：CRUD/archive + memories（CRUD）+ memory_versions（list/get/redact）
- user_profiles：list/create/get/update/enrollment_url
- tunnels：CRUD/archive + certificates（list/create/get/archive）+ reveal_token/rotate_token
- dreams：list/create/get/cancel/archive（实验性）
- 以上均有对应的 GA 路径与 `?beta=true` 变体（SDK 对同一端点注册 GA+beta 两个签名）

## 附录 B：OpenAI 端点清单（182 路径，来自官方 openapi.yaml）

- **推理**：`/chat/completions`（+`/{id}`、`/{id}/messages`）、`/completions`、`/responses`（+`/input_tokens`、`/compact`、`/{id}`、`/{id}/cancel`、`/{id}/input_items`，均有 `?beta=true` 变体）、`/embeddings`、`/moderations`、`/content_provenance_checks`
- **模型**：`/models`、`/models/{model}`
- **文件/上传**：`/files`（+`/{id}`、`/{id}/content`）、`/uploads`（+`/{id}/parts`、`/{id}/complete`、`/{id}/cancel`）
- **批处理**：`/batches`、`/batches/{id}`、`/batches/{id}/cancel`
- **图像/音频/视频**：`/images/generations|edits|variations`、`/audio/speech|transcriptions|translations|voices|voice_consents[/{id}]`、`/videos`（+`/characters`、`/edits`、`/extensions`、`/{id}`、`/{id}/content`、`/{id}/remix`）
- **微调**：`/fine_tuning/jobs`（+`/{id}`、`/cancel`、`/checkpoints`、`/events`、`/pause`、`/resume`）、`/fine_tuning/checkpoints/{id}/permissions[/{pid}]`、`/fine_tuning/alpha/graders/run|validate`
- **Assistants v2**：`/assistants[/{id}]`、`/threads`（+`/runs`、`/{tid}`、`/{tid}/messages[/{mid}]`、`/{tid}/runs` 含 `/cancel`、`/steps[/{sid}]`、`/submit_tool_outputs`）
- **向量库**：`/vector_stores`（+`/{id}`、`/{id}/files[/{fid}]`、`/{fid}/content`、`/{id}/file_batches` 含 `/cancel`、`/files`、`/{id}/search`）
- **Evals**：`/evals`（+`/{id}`、`/{id}/runs`、`/{rid}`、`/{rid}/output_items[/{oid}]`）
- **容器/会话/ChatKit**：`/containers`（+files 子资源 4 个）、`/conversations`（+items 子资源 3 个）、`/chatkit/sessions`（+`/cancel`）、`/chatkit/threads`（+`/{id}`、`/{id}/items`）
- **Realtime**：`/realtime/sessions`、`/realtime/client_secrets`、`/realtime/transcription_sessions`、`/realtime/translations/client_secrets`、`/realtime/calls`（+`/{id}/accept|hangup|refer|reject`）——另有 WS 数据通道（不在 OpenAPI 路径内，P2）
- **Skills**：`/skills`（+`/{id}`、`/{id}/content`、`/{id}/versions[/{ver}]`、`/versions/{ver}/content`）
- **Organization Admin（约 60）**：`admin_api_keys`、`audit_logs`、`certificates`、`costs`、`data_retention`、`groups`、`invites`、`projects`（含 api_keys / certificates / groups / model_permissions / rate_limits / service_accounts / spend_alerts / spend_limit / users 等子资源）、`roles`、`spend_alerts`、`spend_limit`、`usage/*`（completions / embeddings / images / audio_* / moderations / vector_stores / web_search_calls / code_interpreter_sessions / file_search_calls）、`users`（含 roles）
