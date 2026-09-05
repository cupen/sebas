# sebas

**自托管 agent 工作台：工位上并肩，手机上遥控。**

工位上，在网页工作台里和 agent 一起干活；离开工位，用飞书 / IM 指挥它继续干活。sebas 是一个自托管的 agent 工作台，驱动 Claude Code 等兼容 agent——网页主控，IM 遥控，多会话并行。

> 状态：开发中，核心链路已贯通（网页 / 飞书双通道 → 多会话执行 → 流式回写）。

```
离开工位？手机上发条消息 ──► sebas 派活给 agent ──► 进度流式回到你的网页/IM
```

## 功能特性

**工作台（webui 主控）**

- **项目导向的会话区**——按项目目录分组管理 agent 会话：项目列表、会话侧栏、时间线与输入区
- **inbox**——你离开期间到达的 turn 流，回来一次看清
- **权限审批**——工具权限请求（Bash/Read/Write）以按钮呈现：允许一次 / 允许本会话 / 拒绝
- **诚实降级**——core 断连时界面明确告知原因，不装死

**会话执行**

- **ACP 桥**——驱动 Claude Code 等兼容 agent，每个会话一个子进程
- **流式卡片更新**——思考、工具调用、文本输出实时上屏，无需等待最终结果
- **会话持久化**——优雅关闭保存状态，重启后懒恢复
- **媒体转发**——图片、文件、音频自动下载并进入 agent 上下文

**通道**

- **适配器可插拔**——核心只依赖中立通道抽象（`sebas-channels`），通道按配置注册
- **双通道对等**——`web` 与 `feishu` 是同一个注册表里的两个适配器，随时只开其一

**LLM Provider 网关**

- **双协议透传**——Anthropic / OpenAI 协议按模型名路由到 DeepSeek、Kimi、GLM、MiniMax 等上游

---

## 快速开始

前置：Rust 工具链（1.90+）。要真正派活给 agent，还需安装 `claude` CLI。

```bash
# 1. 克隆并准备配置（无必填项——不填飞书凭证即为纯网页形态）
git clone git@github.com:cupen/sebas.git
cd sebas
cp config/config.toml.example config.toml

# 2. 构建并启动（webui 默认监听 127.0.0.1:9797）
cargo build --release
./target/release/sebas run --config ./config.toml --webui
```

浏览器打开 <http://127.0.0.1:9797>（工作台在 `/agent` 页）：新建项目、开会话、发指令，agent 的输出实时出现在时间线里。

---

## 部署

### systemd 服务（watchdog）

`service --install` 会写入一个 systemd 系统 unit，由 **watchdog** 监督：默认只启动 WebUI，core（agent 执行）与飞书按需在 WebUI 服务页启用。升级用 `sebas update`。

```bash
# 需要一个已存在的非 root 账户与绝对路径的 config.toml
sudo sebas service --install --user sebas --config /etc/sebas/config.toml --auto-start

systemctl status sebas        # 状态
sebas ctl status              # 控制面快照
```

### Docker

```bash
docker run -d --name sebas \
  --restart unless-stopped \
  -p 9797:9797 \
  ghcr.io/cupen/sebas:latest \
  sebas run --webui
```

或挂载配置文件：

```bash
docker run -d --name sebas \
  --restart unless-stopped \
  -p 9797:9797 \
  -v /path/to/config.toml:/app/config.toml:ro \
  ghcr.io/cupen/sebas:latest \
  sebas run --webui
```

> 注意：容器内需有 `claude` CLI 才能驱动 agent——建议将宿主机二进制挂载进容器或在自定义镜像中预装。日志：`docker logs -f sebas`；本地构建镜像：`invoke build-image`。

---

## 接入飞书（可选）

飞书只是 sebas 的一个可选通道，用于离开工位后远程遥控——不接入不影响网页工作台的任何功能。

1. 在[飞书开放平台](https://open.feishu.cn/)创建应用，开启权限 `im:message`、`im:message.group_at_msg`、`im:message.p2p_msg`，事件订阅选择**长连接（WebSocket）**模式（非 webhook）。
2. 配置凭证（或用环境变量 `SEBAS_FEISHU_APP_ID` / `SEBAS_FEISHU_APP_SECRET`）：

   ```toml
   [feishu]
   app_id = "cli_xxx"
   app_secret = "xxx"
   owner_id = "ou_xxx"   # 你的 open_id
   ```

   显式开关：`enabled = true` 强制接入（凭据不全拒绝启动）、`enabled = false` 强制停用；缺省按凭据是否齐全隐式判定。
3. 重启后私聊机器人发 `hello`，应看到流式卡片与 Emoji 反应（🖊 输入中 → ⚙ 执行中 → ✅ 完成）。

### 飞书内 slash 命令

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

```bash
# 随主服务启动（或独立进程：sebas gateway --config …）
sebas run --config ./config.toml --gateway

# 客户端接入
ANTHROPIC_BASE_URL=http://127.0.0.1:8787 ANTHROPIC_API_KEY=sk-gw-local-dev claude
```

配置优先级：CLI 参数 > 环境变量 > TOML 文件 > 默认值。完整配置说明见 `config/config.toml.example`（逐项注释）与 `openspec/specs/`。

---

## 愿景

- **短期**：网页工作台是主控台，IM 是遥控器——工位内外无缝切换，多会话并行互不干扰。
- **长期**：通道中立，任意 IM / 客户端作为适配器即插即用；多 agent 并行协作（一个需求拆解给多个 agent 实例同时推进）、跨机器协同（开发机跑 Claude Code，专用服务器跑审查 agent）。

---

## 架构

进程拓扑、IPC 语义与 crate 职责的完整描述见 [docs/architecture.md](docs/architecture.md)；术语以 [openspec/glossary.md](openspec/glossary.md) 为准。

```
   网页工作台          飞书客户端         未来 IM / 客户端
       │                  │                  │
       ▼                  ▼                  ▼
┌─────────────────────────────────────────────────────┐
│ 通道适配器（AdapterRegistry，按配置注册）             │
│   WebAdapter  │  FeishuAdapter  │  …                │
└──────────────────────┬──────────────────────────────┘
     中立事件 ChannelEvent  ⇅  中立卡片 ChannelCard
                       ▼
┌─────────────────────────────────────────────────────┐
│ core（sebas run，长驻进程，会话状态的单一权威）       │
│   router：会话映射 · slash 命令 · 权限状态机 · 编排   │
│   执行体：ACP 桥（Claude Code 子进程）               │
│           │ 原生内核 sebas-agent（开发中）           │
└──────────────────────┬──────────────────────────────┘
                       ▼
        sebas-gateway（Anthropic/OpenAI → 多 provider）
```

核心不特判任何通道：适配器把渠道入站事件翻译为中立 `ChannelEvent`，把中立 `ChannelCard` 渲染为渠道形态。

---

## 项目结构

```
sebas/
├── src/                  # 主二进制：CLI、编排、事件循环、watchdog、core session channel
├── sebas-channels/       # 通道中立抽象：ChannelKey / ChannelEvent / ChannelCard / AdapterRegistry
├── sebas-feishu/         # 飞书适配器与 API 客户端（消息、卡片、媒体、表单）
├── sebas-webui/          # WebUI dashboard 与 web 通道适配器
├── sebas-router/         # 路由引擎（会话映射、命令解析、权限状态机）
├── sebas-acp/            # ACP 桥：驱动 Claude Code 等外部 agent
├── sebas-agent/          # 原生 agent 内核（开发中）
├── sebas-gateway/        # LLM Provider 网关（Anthropic/OpenAI 双协议）
├── config/               # 配置文件示例
├── tests/                # 集成测试（含 fake-claude 测试桩）
└── docs/                 # 设计文档（架构、前端联调等）
```
