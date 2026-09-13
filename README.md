# sebas

**忠诚的 AI 同事，永不下班。**

sebas 是一个 Agent 工作台：通过项目管理与 Agent 编排，驱动 Claude Code / Codex 等 Agent 持续为你工作——PC 主控、IM 遥控，多项目、多会话并行。

工位上，与 AI 同事并肩协作；沙发上、候机厅里，用飞书 / IM 语音指挥。

> **项目状态**：开发中，功能与配置可能随版本演进调整。

---

## 长期愿景

**自由、开放的 Agent 协作平台。**

- **自由交互**：任意 Web / IM / 客户端均可作为适配器即插即用，不绑定渠道。
- **多 Agent 并行协作**：一个需求可拆分给多个 Agent 实例同时推进。
- **跨机器协同**：开发机运行 Claude Code，专用服务器运行审查 Agent。

---

## 功能特性

- **Agent 工作台**：PC 主控，按项目组织会话，多会话并行、互不干扰。
- **权限审批**：工具权限请求以按钮形式呈现，支持允许一次 / 允许本会话 / 拒绝。
- **开放通道**：Web / 飞书 / 任意 IM 客户端都是可插拔适配器，不锁定单一渠道。
- **模型路由**：Anthropic / OpenAI 双协议透传，Claude Code 等客户端无需改代码，即可接入 DeepSeek、Kimi、GLM 等任意上游。

---

## 快速开始

前置条件：Rust 工具链（1.90+）。若要真正把任务派给 Agent，还需安装 `claude` CLI。

```bash
# 1. 克隆并准备配置（无必填项；不填飞书凭证即为纯 Web 形态）
git clone git@github.com:cupen/sebas.git
cd sebas
cp config/config.toml.example config.toml

# 2. 构建并启动（WebUI 默认监听 127.0.0.1:9797）
cargo build --release
./target/release/sebas core --config ./config.toml --webui
```

浏览器打开 <http://127.0.0.1:9797>，新建项目、创建会话、发送指令，Agent 输出会实时出现在时间线上。

---

## 部署

### systemd 服务（watchdog）

`sebas service --install` 会写入 systemd 系统服务，并由 **watchdog** 监督：默认仅启动 WebUI；`core`（Agent 执行）与飞书可按需在 WebUI 服务页启用。升级使用 `sebas update`。

```bash
# 需要一个已存在的非 root 账户，以及绝对路径的 config.toml
sudo sebas service --install --user sebas --config /etc/sebas/config.toml --auto-start

systemctl status sebas        # 服务状态
sebas ctl status              # 控制面快照
```

### Ansible（参考部署 + 本地测试环境）

[`.ansible/`](.ansible/) 是上述 systemd 部署形态的可执行样例：在单机 Linux 上，一条命令即可从零部署到健康检查通过。默认 inventory 指向本机（不走 SSH），因此同一套脚本既是部署参考，也是本地测试环境的一键拉起工具。

前提：控制机已安装 Ansible ≥ 2.x（Windows 控制机可通过 WSL）；目标机为 systemd Linux，控制账户需具备免密 sudo（或直接以 root 执行）。

```bash
cd .ansible
# 本地部署（默认 inventory = localhost）。release 模式从 GitHub Release 下载二进制
ansible-playbook site.yml

# 本地测试环境：手动 cargo build 后，打成与 release 相同 layout 的 tarball
# layout：<basename>/sebas；playbook 只负责搬运，不再执行 cargo build
cargo build --release
mkdir -p sebas-dev-x86_64-unknown-linux-gnu && cp ../target/release/sebas sebas-dev-x86_64-unknown-linux-gnu/
tar -czf sebas-dev-x86_64-unknown-linux-gnu.tar.gz sebas-dev-x86_64-unknown-linux-gnu
ansible-playbook site.yml -e sebas_artifact_source=file -e sebas_artifact_file=$PWD/sebas-dev-x86_64-unknown-linux-gnu.tar.gz

# 部署到远端主机：提供自己的 inventory，playbook 零改动
ansible-playbook site.yml -i inventory/prod.ini

# 跳过 systemd 服务安装；前置准备、配置渲染与健康检查仍会执行
ansible-playbook site.yml --skip-tags sebas_install
```

常用变量（均有默认值，见 [`.ansible/roles/sebas/defaults/main.yml`](.ansible/roles/sebas/defaults/main.yml)）：`sebas_artifact_source`（`release` / `file` / `preinstalled`）、`sebas_version`（`latest` / 显式 tag）、`sebas_deploy_user`、`sebas_webui_listen`（形如 `host:port`，模板中拆分 host / port）、`sebas_config_extra`（追加的 TOML 片段）。`file` 模式要求 `sebas_artifact_file` 指向一份与 release 相同 layout 的 tarball（`<basename>/sebas`）——本地手动 `cargo build --release` 后打包即可。升级 = 更新二进制后重跑 playbook（重跑会经 `service --install --force` 重新 seed 并重启）；已部署机器也可继续使用 `sebas update`。两者均以 `<data_dir>/bin/sebas` 为准。仓库私有或目标机无外网时，使用 `file` / `preinstalled` 模式。

**反向代理 / Nginx**：sebas 默认监听 loopback，仅本机可访问。公开部署需要使用 Nginx 反向代理，模板见 [`.ansible/examples/nginx-sebas-vhost.conf`](.ansible/examples/nginx-sebas-vhost.conf)。将占位符 `__SEBAS_DOMAIN__` / `__UPSTREAM_HOST__` / `__UPSTREAM_PORT__` 替换后，执行 `nginx -t` 校验并 reload。sebas 本身不处理证书与 TLS，由宿主机 Nginx 终结。

**卸载（破坏性）**：role 通过 `sebas_action` 切换动作，默认 `install`——不传该变量跑 playbook 永远执行安装/升级，误跑无害。要下线一台机器，显式传入 `uninstall`：

```bash
cd .ansible
ansible-playbook site.yml -e sebas_action=uninstall
```

⚠️ **这是破坏性动作且不做二次确认**：会停止并删除 systemd 服务、删除两份二进制（`/usr/local/bin/sebas` 与 `<data_dir>/bin/sebas`）、删除整个数据目录（含 sessions DB、`core.secret` 密钥材料、downloads、用量日志）、删除渲染出的 config.toml（provider API key、`sebas_config_extra` 里的凭据随之清除）、删除 `~/.config/sebas`，并用 `userdel -r` 连 home 一起删除部署用户。**卸载即清库，不备份不导出**——如需保留数据请先手动迁移。卸载流程幂等容错：对部分拆除（服务已停、二进制已缺）或已清空的机器重复执行同样收敛成功。远端主机换 `-i` 指定自己的 inventory 即可。

### Docker

镜像启动时会校验 Agent 二进制；缺少 `claude` 会以明确错误退出。原生安装的场景可直接挂载宿主机二进制：

```bash
docker run -d --name sebas \
  --restart unless-stopped \
  -p 9797:9797 \
  -v "$(readlink -f "$(which claude)")":/usr/local/bin/claude:ro \
  ghcr.io/cupen/sebas:latest \
  core --webui --webui-host 0.0.0.0
```

或挂载配置文件（`claude` 挂载按需保留）：

```bash
docker run -d --name sebas \
  --restart unless-stopped \
  -p 127.0.0.1:9797:9797 \
  -v /path/to/config.toml:/app/config.toml:ro \
  ghcr.io/cupen/sebas:latest \
  core --webui --webui-host 0.0.0.0
```

> 通过 npm 等非自包含方式安装的 `claude` 无法直接挂载，建议在自定义镜像中预装（`FROM ghcr.io/cupen/sebas` 后 COPY）。`--webui-host 0.0.0.0` 仅在容器内需要：WebUI 默认只绑定 loopback，不传该参数时映射出的端口不可达。查看日志：`docker logs -f sebas`；本地构建镜像：`invoke build-image`。

---

## 远程执行节点（`sebas-node`，实验性）

本功能可将 Agent 会话放到主控以外的机器上执行。节点是**独立二进制**（第二个可分发产物，不包含 `core` / `webui` / `router` / `im` 等主控角色），以**出站 WebSocket** 连接主控——节点无需开放任何入站端口。该特性默认关闭（`[node_link] enabled = false`，`listen` 默认仅回环 `127.0.0.1:9878`）。

```toml
[node_link]
enabled = true
listen = "127.0.0.1:9878"
```

```bash
sebas node-link -c ./config.toml token            # 签发一次性配对 token（输出到 stdout）
sebas-node --node-id dev-box \
  --control-plane ws://<主控地址>:9878 \
  --join-token <token> --state-dir /var/lib/sebas-node
sebas node-link -c ./config.toml list             # 查看节点与在线状态
sebas node-link -c ./config.toml revoke dev-box   # 吊销；无法通过同 ID 重新配对绕过
```

使用前必须了解以下三点：

- **节点侧尚未实现 `wss://`**：TLS 由部署方的反向代理 / VPN 终结，节点连接本地代理的 `ws://`。
- **项目目录必须已存在于节点机**：主控不会创建或克隆目录。
- **主控缺席时，权限请求会无限期挂起**：`ask` 模式会话无法推进；节点既无本地放行能力，也没有超时机制。

完整说明见 [docs/remote-execution-node.md](docs/remote-execution-node.md)。

---

## 接入飞书（可选）

飞书是 sebas 的可选通道，用于离开工位后远程遥控；不接入不影响 Web 工作台的任何功能。

1. 在[飞书开放平台](https://open.feishu.cn/)创建应用，开启权限 `im:message`、`im:message.group_at_msg`、`im:message.p2p_msg`，事件订阅选择**长连接（WebSocket）**模式（而非 Webhook）。

2. 配置凭证（也可使用环境变量 `SEBAS_FEISHU_APP_ID` / `SEBAS_FEISHU_APP_SECRET`）：

   ```toml
   [feishu]
   app_id = "cli_xxx"
   app_secret = "xxx"
   owner_id = "ou_xxx"   # 你的 open_id
   ```

   显式开关：`enabled = true` 强制接入（凭据不全时拒绝启动），`enabled = false` 强制停用；未设置时根据凭据是否完整自动判定。

   > **架构（extract-im-service）**：飞书接入采用 `core` + `im` 双服务形态。`core` 是纯会话核心，不建立任何 IM 连接；watchdog 会在飞书启用时自动拉起独立的 `sebas im` 进程（也可手动运行 `sebas im -c config.toml`）。该进程承载飞书 WebSocket、卡片渲染、命令与表单，并通过核心会话通道驱动会话。

3. 重启后，私聊机器人发送 `hello`，应能看到流式卡片与 Emoji 状态变化（🖊 输入中 → ⚙ 执行中 → ✅ 完成）。**BREAKING（extract-im-service）**：`core` 进程不再直接承载飞书连接；升级后必须使用 `im` 服务形态（watchdog 默认自动拉起）。

### 飞书内 slash 命令

| 命令 | 说明 |
|------|------|
| `/new` | 开启新会话 |
| `/sessions` | 列出当前会话 |
| `/switch <n>` | 切换到第 n 个会话 |
| `/resume` | 恢复上一个会话 |
| `/cancel` | 中断当前处理 |
| `/btw` | 优先排队（当前任务完成后优先处理） |
| `/settings` | 调整卡片主题、截断、折叠等 |
| `/provider` | 管理 LLM Provider（交互表单） |
| `/help` | 查看帮助信息 |

---

## 模型路由

sebas 内置一个双协议（Anthropic / OpenAI）纯透传模型路由。将 Claude Code 或任意兼容 SDK 的 `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` 指向本路由，即可把流量分发到 DeepSeek、Kimi、GLM、MiniMax、Ark、DashScope、Gemini 等上游 Provider。

```toml
[router]
listen = "127.0.0.1:8787"
auth_token = "sk-gw-local-dev"

[provider.anthropic]
api_key_env = "ANTHROPIC_API_KEY"

[router.routes]
"claude-*" = ["anthropic"]
```

```bash
# router 以独立进程运行（或交由 watchdog 托管：config [watchdog.router] enabled = true）
sebas router --config ./config.toml

# 客户端接入
ANTHROPIC_BASE_URL=http://127.0.0.1:8787 ANTHROPIC_API_KEY=sk-gw-local-dev claude
```

配置优先级：CLI 参数 > 环境变量 > TOML 文件 > 默认值。完整配置说明见 [`config/config.toml.example`](config/config.toml.example)（逐项注释）与 `openspec/specs/`。

---

## 架构

进程拓扑、三套 IPC 通道与子命令入口的完整说明见 [docs/architecture/process-ipc-subcommands.md](docs/architecture/process-ipc-subcommands.md)；术语以 [openspec/glossary.md](openspec/glossary.md) 为准。

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
│   dispatch：会话映射 · slash 命令 · 权限状态机 · 编排 │
│   执行体：ACP 桥（Claude Code 子进程）               │
│           │ 原生内核 sebas-agent（开发中）           │
└──────────────────────┬──────────────────────────────┘
                       ▼
        sebas-router（Anthropic / OpenAI → 多 Provider）
```

核心不特判任何通道：适配器把渠道入站事件翻译为中立 `ChannelEvent`，再把中立 `ChannelCard` 渲染为渠道形态。

> 上图为逻辑视图；自 extract-im-service 起，飞书适配器运行在独立 `im` 进程中，通过核心会话通道与 `core` 通信。完整进程拓扑见上方架构文档。

---

## 项目结构

```
sebas/
├── src/                  # 主二进制：CLI、编排、事件循环、watchdog、core session channel
├── sebas-channels/       # 通道中立抽象：ChannelKey / ChannelEvent / ChannelCard / AdapterRegistry
├── sebas-feishu/         # 飞书适配器与 API 客户端（消息、卡片、媒体、表单）
├── sebas-im/             # IM 服务层：适配器宿主与交互状态机（卡片、审批、命令、表单、reactions、媒体）
├── sebas-webui/          # WebUI dashboard 与 Web 通道适配器
├── sebas-dispatch/       # 会话分发领域层：会话映射、命令、权限状态机（原 sebas-router）
├── sebas-acp/            # ACP 桥：驱动 Claude Code 等外部 Agent
├── sebas-agent/          # 原生 Agent 内核（开发中）
├── sebas-router/         # 模型路由：LLM Provider 透传代理（Anthropic / OpenAI 双协议；原 sebas-gateway）
├── sebas-node/           # 执行节点：独立二进制 sebas-node（远程执行，不含主控角色）
├── sebas-node-link/      # 主控 ↔ 节点共用的链路协议契约类型（两侧都可依赖的 crate）
├── sebas-ipc/            # 跨平台本地 IPC 传输层（core session channel / control RPC / router 状态订阅共用）
├── sebas-startup/        # sebas / sebas-node 共用的启动失败退出路径
├── config/               # 配置文件示例
├── tests/                # 集成测试（含 fake-claude 测试桩）
│   └── testsuite-webui/  # 浏览器级 UI 旅程（Playwright + Chromium，见下）
└── docs/                 # 设计文档（架构、前端联调等）
```

## 测试

- **进程级 e2e**：`invoke testsuite-e2e`（`tests/testsuite_e2e_test.rs`）；单用例 `invoke testsuite-e2e --case <name>`。
- **旅程级验收**：`invoke testsuite-acceptance`（`tests/testsuite_acceptance_test.rs`）；单旅程 `invoke testsuite-acceptance --case <name>`。
- **浏览器级 UI 旅程**：`invoke testsuite-webui`。一键构建（dist 自动重建）→ 装配一次性沙箱（`core --webui` + 独立 `sebas router --debug` 两进程 + fake-claude 桩，不触碰真实 `~/.sebas` 与 9797）→ 运行全部旅程（主 config + auth config）→ 按运行结果清理。
  - 单旅程：`invoke testsuite-webui --case <spec 文件名>`（如 `--case first-paint`；`--case auth` 跑鉴权形态，端口 9898）。
  - 任一用例失败会保留沙箱现场，并在输出中打印路径（含后端日志 `core.log`），便于复现：`TESTSUITE_REUSE=1 TESTSUITE_KEEP=1 invoke testsuite-webui --case <name>`。
