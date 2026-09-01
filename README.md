# sebas

**Agent 桥接到飞书，把 AI 助手装进口袋。**

sebas 是一个 Rust 守护进程，将 Claude Code（及其他兼容 agent）接入飞书，让你在飞书客户端即可驱动 AI 助手执行开发任务——无需死守终端，随时查看进度、审批权限、切换会话。

```
你在飞书发消息 ──► sebas 收到 ──► 拉起 Claude Code 执行 ──► 流式回写到飞书卡片
```

## 状态
开发中……

---

## 愿景

### 短期：远程驱动开发任务

日常开发场景——重构、批量编辑、跑测试、查日志——往往需要长时间占用终端。sebas 把这些任务搬到飞书，让你：

- 在工位上用飞书 Bot 发指令，后台执行，**边干活边等结果**
- 离开座位时用手机查看进度、审批权限申请
- 多个会话并行，互不干扰

### 长期：跨机器多 Agent 并行协作

sebas 的架构设计不止于单机单 agent。长期探索的方向是：

- **多 agent 并行**：将一个需求拆解为多个子任务，分配给不同 agent 实例同时推进，最后汇总结果
- **跨机器协作**：不同机器上的 sebas 实例协同工作，各自承担不同角色（如开发机跑 Claude Code、专用 GPU 服务器跑代码审查 agent）
- **统一调度入口**：飞书作为唯一交互界面，背后是分布式的 agent 集群

---

## 架构

进程拓扑、IPC 语义与 crate 职责的完整描述见 [docs/architecture.md](docs/architecture.md)。
```
┌─────────────────────────────────────────────────────────────────┐
│                        飞书客户端                                │
│             私聊 / 群聊 / 话题 ── 交互卡片 + 按钮               │
└──────────────────────────┬──────────────────────────────────────┘
                           │ 飞书 WebSocket 长连接
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│  sebas daemon (Rust)                                            │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ ws_loop    ──►  RouterEventHandler  ──►  RouterHandle   │    │
│  │   (长连接)        (事件解析)            (路由分发)       │    │
│  └───────────────┬─────────────────────────────────────────┘    │
│                  │ Out 指令 (发卡片 / 更新 / 反应)               │
│                  ▼                                              │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ dispatch_out    ──►  飞书 API (send_card / update_card)  │    │
│  └─────────────────────────────────────────────────────────┘    │
│                  │                                              │
│                  │ AcpEvent (流式事件)                           │
│                  ▼                                              │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ sebas-acp::claude (SessionManager + CcDriver)                   │    │
│  │   每个 session = 一个 Claude Code 子进程                  │    │
│  └─────────────────────────────────────────────────────────┘    │
│                  │                                              │
│                  ▼                                              │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ sebas-gateway (LLM 双协议网关)                                  │    │
│  │   Anthropic / OpenAI 协议  ──► 多 provider 路由          │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

### 核心设计

| 层次 | 组件 | 职责 |
|------|------|------|
| **接入层** | `ws_loop` + `sebas-feishu` crate | 飞书 WebSocket 长连接（断线重连）、事件解析、API 调用 |
| **路由层** | `sebas-router` crate | 会话映射、slash 命令解析、权限处理、状态机管理 |
| **Agent 层** | `sebas-acp` crate | Claude Code 子进程生命周期、流式事件泵送、中断恢复 |
| **网关层** | `sebas-gateway` crate | Anthropic/OpenAI 双协议透传、多 provider 路由、用量记录 |

---

## 功能特性

- **流式卡片更新**——Claude Code 的思考、工具调用、文本输出实时写入飞书卡片，无需等待最终结果
- **表情反应状态机**——卡片 Emoji 反应实时反映处理阶段：🖊 输入中 → ⚙ 执行中 → ✅ 完成 / ❌ 失败
- **权限交互卡片**——工具权限请求（Bash/Read/Write）以交互按钮呈现：允许一次 / 允许本次会话 / 拒绝
- **多会话管理**——`/new` 开新会话、`/sessions` 查看列表、`/switch` 切换、`/resume` 恢复
- **会话持久化**——优雅关闭时保存状态，重启后懒恢复，支持 Claude 原生 `resume`
- **媒体转发**——图片、文件、音频自动下载并传入 agent 上下文
- **LLM Provider 网关**——内置双协议网关，把请求按模型名路由到任意上游 provider
- **Provider CRUD**——`/provider` 命令在飞书直接管理 LLM provider，无需编辑配置文件
- **运行时配置**——`/settings` 调整卡片主题色、思考内容展示、文本截断、工具输出折叠等

---

## 快速开始

### 前置条件

1. 在 [飞书开放平台](https://open.feishu.cn/) 创建应用，开启权限：
   - `im:message`（接收和发送消息）
   - `im:message.group_at_msg`（群聊 @ 机器人）
   - `im:message.p2p_msg`（私聊消息）
   - 在事件订阅中启用 **长连接（WebSocket）** 模式（非 webhook）
2. 安装 Rust 工具链（1.90+）和 `claude` CLI

### 本地运行

```bash
# 1. 克隆并配置
git clone git@github.com:cupen/sebas.git
cd sebas
cp config/config.toml.example config.toml

# 2. 编辑 config.toml，填入飞书应用凭证
#    - feishu.app_id
#    - feishu.app_secret
#    - feishu.owner_id（你的 open_id，ou_xxx 格式）

# 3. 构建
cargo build --release

# 4. 启动
./target/release/sebas run --config ./config.toml
```

### Docker 运行

#### 使用环境变量启动（推荐）

镜像默认会查找 `/app/config.toml`，找不到时回退到环境变量：

```bash
docker run -d --name sebas \
  --restart unless-stopped \
  -e SEBAS_FEISHU_APP_ID=cli_xxxxxxxxxxxx \
  -e SEBAS_FEISHU_APP_SECRET=xxxxxxxxxxxxxxxxxxxxxxxx \
  -e SEBAS_LOG_LEVEL=info \
  ghcr.io/cupen/sebas:latest
```

#### 使用配置文件启动

```bash
docker run -d --name sebas \
  --restart unless-stopped \
  -v /path/to/your/config.toml:/app/config.toml:ro \
  ghcr.io/cupen/sebas:latest
```

#### 完整示例：带网关的 Docker 部署

```bash
# 拉取最新镜像
docker pull ghcr.io/cupen/sebas:latest

# 启动（同时启用内置 LLM 网关）
docker run -d --name sebas \
  --restart unless-stopped \
  -e SEBAS_FEISHU_APP_ID=cli_xxxxxxxxxxxx \
  -e SEBAS_FEISHU_APP_SECRET=xxxxxxxxxxxxxxxxxxxxxxxx \
  -e SEBAS_GATEWAY_LISTEN=0.0.0.0:8787 \
  -e SEBAS_LOG_LEVEL=info \
  -p 8787:8787 \
  ghcr.io/cupen/sebas:latest \
  sebas run --config /app/config.toml --gateway
```

> 注意：`claude` CLI 需要在容器内可用。生产部署建议将宿主机的 `claude` 二进制挂载到容器内，或在自定义镜像中预装。

#### 查看日志

```bash
docker logs -f sebas
```

#### 本地构建（可选）

如需自行构建镜像：

```bash
invoke build-image
```

---

## 配置

### 最小配置

```toml
[feishu]
app_id = "cli_xxx"
app_secret = "xxx"
owner_id = "ou_xxx"
```

### 配置优先级

CLI 参数 > 环境变量 > TOML 文件 > 默认值

### 关键环境变量

| 变量 | 说明 |
|------|------|
| `SEBAS_FEISHU_APP_ID` | 飞书应用 App ID |
| `SEBAS_FEISHU_APP_SECRET` | 飞书应用 App Secret |
| `SEBAS_LOG_LEVEL` | 日志级别（默认 `info`） |
| `SEBAS_GATEWAY_LISTEN` | 网关监听地址（如 `127.0.0.1:8787`） |
| `SEBAS_GATEWAY_PROVIDER_OVERLAY` | Provider 覆盖文件路径 |

### 完整配置说明

详见 `config/config.toml.example`（含逐项注释）与 `openspec/specs/`（各 capability 的行为契约，如 `cli-service`、`feishu-cards`）。

---

## 命令

| 命令 | 说明 |
|------|------|
| `/new` | 开启新会话 |
| `/sessions` | 列出当前会话 |
| `/switch <n>` | 切换到会话 n |
| `/resume` | 恢复上一个会话 |
| `/cancel` | 中断当前处理 |
| `/btw` | 优先排队（当前任务完成后优先处理） |
| `/settings` | 调整卡片主题、截断、折叠等 |
| `/provider` | 管理 LLM provider（交互表单） |
| `/help` | 帮助信息 |

---

## LLM Provider 网关

sebas 内置一个双协议（Anthropic/OpenAI）纯透传 LLM 网关。让 Claude Code 或任意兼容 SDK 经 `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` 指向本网关，即可将流量路由到 DeepSeek、Kimi、GLM、MiniMax、Ark、DashScope、Gemini 等上游 provider。

### 最小网关配置

```toml
[gateway]
listen = "127.0.0.1:8787"
auth_token = "sk-gw-local-dev"

[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"

[gateway.routes]
"claude-*" = ["anthropic"]
```

### 启动方式

```bash
# 独立进程
sebas gateway --config ./config.toml

# 随主服务启动（随机端口）
sebas run --config ./config.toml --gateway --debug
```

### 客户端接入

```bash
# Claude Code 经网关
ANTHROPIC_BASE_URL=http://127.0.0.1:8787 ANTHROPIC_API_KEY=sk-gw-local-dev claude

# OpenAI SDK 经网关
OPENAI_BASE_URL=http://127.0.0.1:8787 OPENAI_API_KEY=sk-gw-local-dev ...
```

详见 `openspec/specs/gateway-core/`（路由与协议面契约）及 `openspec/specs/gateway-auth-rate-limit/`（鉴权与限流）。

---

## 项目结构

```
sebas/
├── src/                  # 主二进制：配置、CLI、编排、事件循环
├── sebas-feishu/        # 飞书 API 客户端（消息、卡片、媒体、事件、表单）
├── sebas-router/        # 路由引擎（状态机、命令解析、会话映射、权限处理）
├── sebas-acp/           # ACP 层：claude 子模块（cc-agent-sdk）
├── sebas-gateway/       # LLM Provider 网关（Anthropic/OpenAI 双协议）
├── config/               # 配置文件示例
├── tests/                # 集成测试（含 fake-claude 测试桩）
├── scripts/              # 辅助脚本
└── docs/                 # 设计文档与实现计划
```

---

## 当前状态

MVP / 持续开发中。核心链路已贯通：飞书 WebSocket 长连接 → 事件解析 → 路由分发 → Claude Code 子进程 → 流式回写卡片。已知局限：

- 会话状态持久化到 `state_file`，重连后懒恢复；但关闭时正在处理的轮次会丢失
- `/compact`、`/cost` 作为字面提示传给 Claude；`/model`、`/cd`、`/status`、`/help` 已解析但未完整接入
- 未配置 CI 流水线
- 覆盖率目标（sebas-router ≥90%、cards ≥90%、整体 ≥80%）为设定目标，尚未验证

---

## 手动冒烟测试

1. 启动 sebas，确认 `sebas started` 日志
2. 飞书私聊发 "hello"；确认响应卡片出现 Emoji 反应序列
3. 发 "list the files here"；确认出现权限卡片——点击 Allow
4. 发 `/new`；确认新会话创建
5. 发 `/sessions`；确认两个会话可见
6. 重启 sebas，在同一会话发消息；确认会话恢复