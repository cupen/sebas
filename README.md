# sebas

**忠诚的 AI 同事永不下班。**

sebas 是一个 agent 工作台，通过项目管理和 agent 编排，以最舒服的姿势驱动 Claude Code / Codex 等 agent 持续为你工作—— PC 主控，IM 遥控，多项目多会话并行。  
工位上，和 AI 同事一起工作；沙发上/候机厅，用飞书 / IM 语音指挥协同工作。

> 状态：开发中...

---

## 长期愿景

**从一个 agent 工作台，长成一个 agent 协作网络。**

- **交互自由**——任意 Web / IM / 客户端作为适配器即插即用，不锁渠道。
- **多 agent 并行协作**——一个需求拆给多个 agent 实例同时推进。
- **跨机器协同**——开发机跑 Claude Code，专用服务器跑审查 agent。

---

## 功能特性

- **项目工作台**——PC 主控，按项目目录组织会话，多会话并行互不干扰
- **权限审批**——工具权限请求以按钮抛出：允许一次 / 允许本会话 / 拒绝
- **开放通道**——web / 飞书 / 任意 IM 客户端都是可插拔适配器，不锁死在一家渠道
- **模型路由**——Anthropic / OpenAI 双协议透传，Claude Code 等客户端不必改代码即可接入 DeepSeek、Kimi、GLM 等任意上游

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
./target/release/sebas core --config ./config.toml --webui
```

浏览器打开 <http://127.0.0.1:9797>, 新建项目、开会话、发指令，agent 的输出实时出现在时间线里。

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

### Ansible（参考部署 + 本地测试环境）

[`.ansible/`](.ansible/) 是上述 systemd 形态的可执行样例：单机 Linux 一条命令从零到健康检查通过；默认 inventory 指向本机（不走 SSH），同一份脚本既是部署参考，也是本地测试环境的一键拉起工具。

前提：控制机装有 ansible ≥ 2.x（Windows 控制机走 WSL）；目标机为 systemd Linux，控制账户对目标机有免密 sudo（或直接以 root 执行）。

```bash
cd .ansible
# 本地部署（默认 inventory = localhost）。release 模式从 GitHub release 下载二进制
ansible-playbook site.yml

# 本地测试环境：手动 cargo build 后打成 release 同款 tarball（layout: <basename>/sebas），
# playbook 只负责搬过去、不再 cargo build
cargo build --release
mkdir -p sebas-dev-x86_64-unknown-linux-gnu && cp ../target/release/sebas sebas-dev-x86_64-unknown-linux-gnu/
tar -czf sebas-dev-x86_64-unknown-linux-gnu.tar.gz sebas-dev-x86_64-unknown-linux-gnu
ansible-playbook site.yml -e sebas_artifact_source=file -e sebas_artifact_file=$PWD/sebas-dev-x86_64-unknown-linux-gnu.tar.gz

# 部署到远端主机：提供自己的 inventory，playbook 零改动
ansible-playbook site.yml -i inventory/prod.ini

# 只铺前置（用户/目录/二进制），不渲染配置、不动服务
ansible-playbook site.yml --skip-tags sebas_install
```

常用变量（全部有缺省，见 [`.ansible/roles/sebas/defaults/main.yml`](.ansible/roles/sebas/defaults/main.yml)）：`sebas_artifact_source`（release/file/preinstalled）、`sebas_version`（latest/显式 tag）、`sebas_deploy_user`、`sebas_webui_listen`（形如 `host:port`；模板里 split 取 host/port）、`sebas_config_extra`（追加 TOML 片段）。`file` 模式要求 `sebas_artifact_file` 指向一份 release 同 layout 的 tarball（`<basename>/sebas`）——本地手动 `cargo build --release` 后打包即可。升级 = 更新二进制后重跑 playbook（重跑会经 `service --install --force` 重新 seed 并重启）；已部署机器也可继续用 `sebas update`，两者都以 `<data_dir>/bin/sebas` 为准。仓库私有或无外网时用 file/preinstalled 模式。

**反向代理 / nginx**：sebas 默认监听 loopback，仅本机可访问。公开部署需要 nginx 反代，模板见 [`.ansible/examples/nginx-sebas-vhost.conf`](.ansible/examples/nginx-sebas-vhost.conf)（占位符 `__SEBAS_DOMAIN__`/`__UPSTREAM_HOST__`/`__UPSTREAM_PORT__` 自行替换后 `nginx -t` 校验 + reload）。sebas 本身不绑证书、不管 TLS，由宿主 nginx 终结。

### Docker

镜像启动时即校验 agent 二进制（缺 `claude` 会以明确报错退出）——原生安装的直接把宿主机二进制挂进去即可：

```bash
docker run -d --name sebas \
  --restart unless-stopped \
  -p 9797:9797 \
  -v "$(readlink -f "$(which claude)")":/usr/local/bin/claude:ro \
  ghcr.io/cupen/sebas:latest \
  core --webui --webui-host 0.0.0.0
```

或挂载配置文件（`claude` 挂载同上，按需加上）：

```bash
docker run -d --name sebas \
  --restart unless-stopped \
  -p 9797:9797 \
  -v /path/to/config.toml:/app/config.toml:ro \
  ghcr.io/cupen/sebas:latest \
  core --webui --webui-host 0.0.0.0
```

> `claude` 为 npm 等非自包含方式安装时无法直接挂载，建议在自定义镜像中预装（`FROM ghcr.io/cupen/sebas` 后 COPY）。`--webui-host 0.0.0.0` 仅容器内需要：webui 默认只绑 loopback，不传它发布的端口不可达。日志：`docker logs -f sebas`；本地构建镜像：`invoke build-image`。

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

   > **架构（extract-im-service）**：飞书接入 = core + im 双服务形态。core 是纯会话核心（不建立任何 IM 连接）；watchdog 在飞书启用时自动拉起独立 `sebas im` 进程（也可手动运行 `sebas im -c config.toml`），由它承载飞书 WebSocket、卡片渲染、命令与表单，经核心会话通道驱动会话。
3. 重启后私聊机器人发 `hello`，应看到流式卡片与 Emoji 反应（🖊 输入中 → ⚙ 执行中 → ✅ 完成）。**BREAKING（extract-im-service）**：core 进程不再直挂飞书——升级后飞书必须走 im 服务形态（watchdog 默认自动拉起）。

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

## 模型路由

sebas 内置一个双协议（Anthropic/OpenAI）纯透传的模型路由。让 Claude Code 或任意兼容 SDK 经 `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` 指向本路由，即可将流量路由到 DeepSeek、Kimi、GLM、MiniMax、Ark、DashScope、Gemini 等上游 provider。

```toml
[router]
listen = "127.0.0.1:8787"
auth_token = "sk-gw-local-dev"

[provider.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"

[router.routes]
"claude-*" = ["anthropic"]
```

```bash
# 随主服务启动（或独立进程：sebas router --config …）
sebas core --config ./config.toml --router

# 客户端接入
ANTHROPIC_BASE_URL=http://127.0.0.1:8787 ANTHROPIC_API_KEY=sk-gw-local-dev claude
```

配置优先级：CLI 参数 > 环境变量 > TOML 文件 > 默认值。完整配置说明见 `config/config.toml.example`（逐项注释）与 `openspec/specs/`。

---

## 架构

进程拓扑、三套 IPC 通道与子命令入口的完整描述见 [docs/architecture/process-ipc-subcommands.md](docs/architecture/process-ipc-subcommands.md)；术语以 [openspec/glossary.md](openspec/glossary.md) 为准。

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
│ core（sebas core，长驻进程，会话状态的单一权威）      │
│   router：会话映射 · slash 命令 · 权限状态机 · 编排   │
│   执行体：ACP 桥（Claude Code 子进程）               │
│           │ 原生内核 sebas-agent（开发中）           │
└──────────────────────┬──────────────────────────────┘
                       ▼
        sebas-router（Anthropic/OpenAI → 多 provider）
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
├── sebas-router/         # 模型路由：LLM provider 透传代理（Anthropic/OpenAI 双协议；原 sebas-gateway）
├── sebas-dispatch/       # 会话分发领域层（会话映射/命令/权限；原 sebas-router）
├── config/               # 配置文件示例
├── tests/                # 集成测试（含 fake-claude 测试桩）
│   └── testsuite-webui/        # 浏览器级 UI 旅程（Playwright + chromium，见下）
└── docs/                 # 设计文档（架构、前端联调等）
```

## 测试

- 进程级 e2e：`invoke testsuite-e2e`（`tests/testsuite_e2e_test.rs`）；单用例 `invoke testsuite-e2e --case <name>`。
- 旅程级验收：`invoke testsuite-acceptance`（`tests/testsuite_acceptance_test.rs`）；单旅程 `invoke testsuite-acceptance --case <name>`。
- **浏览器级 UI 旅程**（本仓库新增）：`invoke testsuite-webui`。一键构建（dist 自动重建）→
  装配一次性沙箱（`sebas core --router --debug --webui` + fake-claude 桩，绝不触碰真实
  `~/.sebas` 与 9797）→ 运行全部旅程（主 config + auth config）→ 按运行结果清理。
  单旅程：`invoke testsuite-webui --case <spec 文件名>`（如 `--case first-paint`；`--case auth`
  跑鉴权形态，端口 9898）。任一用例失败会保留沙箱现场并在输出里打印路径（含后端日志
  `core.log`），供复现：`TESTSUITE_REUSE=1 TESTSUITE_KEEP=1 invoke testsuite-webui --case <name>`。
