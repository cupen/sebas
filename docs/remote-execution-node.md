# 远程执行节点（`sebas-node`）

> **定位**：把 agent 会话放到主控以外的机器上执行。节点是**独立二进制**
> `sebas-node`，**出站** websocket 连回主控；节点机上不需要、也不包含主控角色
> （core / webui / router / im）。设计意图见
> `openspec/changes/add-remote-execution-node/`（D0/D13 等）。
>
> **维护方式**：本文以代码为准（快照 **2026-09-11**）。命令与配置键核对于
> `sebas-node/src/cli.rs`、`sebas-node/src/config.rs`、`src/node_link_cmd.rs`、
> `src/cli.rs`、`src/config.rs`。行为变化时同步更新本文。
>
> **状态**：特性默认关闭；节点的真实执行体接入与工作台呈现仍在
> `add-remote-execution-node` 的实现中（6.3 / 7.1 / 7.2 / group 8）。本文只描述
> 当前代码已经具备的操作面。

---

## 0. 前提：项目树必须先在节点机上存在

- 项目在本 change 后是 **`(节点, 节点本地路径)`** 的具名引用。路径的可用性判定
  发生在**真正 spawn 的那一侧**：节点在创建会话前检查该路径是否为本机目录
  （`sebas-node/src/session.rs`），不是目录就返回类型化拒绝
  `unusable_project_dir`，成因里**指名路径**。
- 主控**不会**创建、clone 或同步节点上的目录：远端项目的注册只登记
  `(节点, 路径)`，在主控本机**不做任何文件系统操作**
  （`sebas-webui/src/projects.rs::add_on`）。同一路径注册在两台节点上是两个互不
  等价的项目条目。
- 因此：**在节点机上先把项目 checkout / 准备好，再去主控注册**。不实现文件同步、
  不实现自动 clone。

---

## 1. 主控侧：打开节点链路并取配对 token

### 1.1 配置 `[node_link]`

节点入站端点由 **core** 托管。默认**关**、默认只监听回环：

```toml
[node_link]
enabled = true            # 默认 false：这是一个新的网络入站面，必须显式打开
listen = "127.0.0.1:9878" # 默认值；须为 IP:PORT（不接受域名）
# registry_file = "/etc/sebas/nodes.json"   # 缺省：config 文件同目录下的 nodes.json
# bootstrap_token_ttl_secs = 900            # 首个 bootstrap token 的有效期（秒）
```

环境变量 `SEBAS_NODE_LINK_LISTEN` 可覆盖 `listen`。`enabled = true` 时 `listen`
在**解析期**校验：不是合法 `IP:PORT` 就启动失败，不会拖到 ready 之后。

改完配置**重启 core**（或重启 watchdog 托管的服务）。

### 1.2 取 token（两种方式，二选一）

**A. 首启 bootstrap token（自动）**：当注册表里**既没有节点、也没有待用 token**
时，core 在启动时签发一个 bootstrap token 并打进日志（`warn` 级），例如：

```
节点链路已开放（127.0.0.1:9878）。bootstrap 配对 token（只显示这一次，900 秒内有效）：<token>
```

**只显示这一次**；之后重启不再重复吐凭据。用 `bootstrap_token_ttl_secs` 控制有效期。

**B. 显式签发（推荐日常使用）**：core 在跑、`[node_link] enabled = true` 时：

```bash
# 注意：-c/--config 是 sebas node-link 自己的选项，必须写在子命令之前
sebas node-link -c /etc/sebas/config.toml token
# 指定有效期（秒）：
sebas node-link -c /etc/sebas/config.toml token --ttl 600
```

token 打到 **stdout**（便于 `TOKEN=$(sebas node-link … token)` 取用），指引打到
stderr。token 是**一次性、带过期**的：被消费后再次使用会被永久拒绝。

```bash
# 查看已注册节点与在线态
sebas node-link -c /etc/sebas/config.toml list
```

若 core 未启用节点链路，命令以非零退出并明确提示「节点链路未启用」；core 会话通道
不可用时同样非零退出，不会假装成功。

---

## 2. 节点侧：安装、配置与配对

节点只有一个可执行文件 `sebas-node`，配置只有 `[node]` 一个段（**不复用主控的
配置 schema**）。取值优先级：**命令行 > 环境变量 > 配置文件 > 默认值**。

### 2.1 一次性配对

```bash
sebas-node \
  --node-id dev-box \
  --control-plane ws://<主控地址>:9878 \
  --join-token <上一步拿到的 token> \
  --state-dir /var/lib/sebas-node
```

- `--control-plane` 指向 **core 的 `[node_link] listen` 端点**（默认端口 9878）。
- `--state-dir` 也可用环境变量 `SEBAS_NODE_DIR`，或用配置文件里的 `[node] state_dir`；
  都缺省时落在 `<系统 data_dir>/sebas-node`。
- `--node-id` 缺省时：复用状态目录里已存的 id；从未存过则生成一个并落盘。重装后
  沿用同一个 id 可让既有的项目/会话继续指得中。

等价的配置文件（`-c node.toml`）写法：

```toml
[node]
id = "dev-box"
control_plane = "ws://<主控地址>:9878"
state_dir = "/var/lib/sebas-node"
max_sessions = 8          # 缺省 8；并发会话上限
log_retention_days = 30   # 缺省 30；节点本地日志保留期
# upstream = "local"      # 缺省 local（节点持自己凭据）；可选 control-plane-router
# default_work_dir = "/srv/work"   # 须为绝对路径；**无项目**的会话落在这里。
                                   # 没配就拒绝建立无项目会话（不替操作者挑一个目录）

# 本节点配置的 agent 执行体（用于握手能力清单；command 用于可达性探测）
[node.agents.claude]
command = "claude"
enforces_mode = false     # 该执行体能否强制 mode；缺省 false（不假定能做到）

# 本节点持有的 provider 名字（只有名字；凭据不进清单、不过河）
# providers = ["anthropic"]
```

`[node]` 段未知键会**直接报错**（打错字不会被静默忽略）。

### 2.2 节点配对后存了什么

状态目录（`[node] state_dir` / `--state-dir` / `SEBAS_NODE_DIR`）下的内容：

| 路径 | 内容 | 权限 |
|---|---|---|
| `node-id` | 稳定节点标识 | `0600` |
| `credential` | join token 换来的**长期凭据** | `0600` |
| `sessions/` | 节点本地会话 turn 日志（append-only + seq/epoch） | — |
| `materials/` | 操作者级材料按版本目录隔离的落点 | — |

- 长期凭据在配对成功、**落盘之后**才声称成功（落盘失败即配对失败）。
- 状态目录不存在时由节点创建为 `0700`；**已存在的目录不动它的权限**——若你预建了
  该目录，请自行 `chmod 0700`。
- 之后再次启动**不需要** `--join-token`：节点直接复用已存凭据。

### 2.3 部署自检

```bash
sebas-node --check \
  --control-plane ws://<主控地址>:9878 \
  --state-dir /var/lib/sebas-node
```

`--check` 只解析配置与身份、打印生效值，不建立链路、不要求已配对：

```
node id:        dev-box
control plane:  ws://<主控地址>:9878
state dir:      /var/lib/sebas-node
upstream:       local
max sessions:   8
log retention:  30 天
default work:   (未设置：无项目会话将无法落脚)
```

启动失败（未配对、配置非法、凭据被吊销、协议版本不兼容等）时，节点以退出码 **75**
退出，stderr 末行形如 `startup-failure: <原因>`；链路抖动则是**内部退避重试**
（1s 起、翻倍、封顶 60s），不退出。

---

## 3. 网络 / TLS 要求

- **反向连接**：节点**出站**拨号主控，因此**节点不需要任何入站端口**；主控只需
  暴露一个端点（`[node_link] listen`）。NAT / 内网 / 别人的机器都可作为节点。
- **`wss://` 在节点侧尚未实现**：节点对 `wss://` **如实拒绝**并以启动失败退出
  （报错写明「节点侧 TLS 尚未实现」），不会假装支持。TLS 由**部署方**终止：
  用反向代理（nginx 等）或 VPN 在节点机本地终结 TLS，节点连**本地代理**的
  `ws://`：

  ```
  sebas-node ── ws://127.0.0.1:<本地反代端口> ──▶ [反代/VPN: TLS 终结] ──▶ 主控 [node_link] listen
  ```

  即：节点进程本身不建 TLS 栈，sebas 也不承诺自带 TLS。**不要明文跨公网**——
  把 `ws://` 暴露到公网等于把配对 token 与长期凭据明文送出去。
- **默认仅回环**：`[node_link] listen` 缺省 `127.0.0.1:9878`，只有本机能连。
  要让远端节点接入，必须同时（a）把 `listen` 调到可被反代/VPN 到达的地址，
  或让反代连本机回环；(b) 由部署方提供加密与访问控制。
- **默认关闭**：`[node_link] enabled = false`。不开就没有任何节点入站面。

---

## 4. 吊销节点

```bash
sebas node-link -c /etc/sebas/config.toml revoke dev-box
```

- 立即吊销该节点的长期凭据：节点下次连接被拒，成因**指名吊销**；节点会以启动失败
  （75）退出而不是静默空转。
- 被吊销的 id **不能靠重新配对绕过**：同一 id 再次配对同样被拒（报
  `node_id_conflict`，提示先在主控侧删除该条目或换一个 id）。重装要沿用原 id，须先
  清除注册表里那条已吊销记录。
- 吊销一个不存在的节点：非零退出，并明确「未做任何改动」。
- 吊销不删除注册表条目；`list` 中该节点状态为 `revoked`。

---

## 5. 主控丢失的后果（重要）

**受影响的是「决策」，不是「在飞执行」。**

- 节点上的权限请求只能上行主控裁决，节点**没有任何本地放行入口**。主控不可达时，
  权限门控的动作**无限期 park**：**没有超时、没有本地审批、没有降级放行**——这是
  设计性质，不是缺陷。代码里也不存在任何计时器参与裁决。
- 因此，**处于 `ask` 模式的会话在主控缺席期间无法推进**（任何受门控的动作都会停在
  那里）；其它模式下被门控的动作同理。节点也不会在主控缺席时接受新工作——它没有
  本地指挥面。
- **运维能做的**：**恢复主控**。重连后节点会重报仍然悬空的请求，它们**还在**，
  可以对它们下决定。不存在第二条恢复路径。
- **如果主控永久丢失**：该节点上处于 `ask` 悬空的会话**不可恢复**（节点侧没有
  第二出口）。这是必须在部署前知道的取舍。
- 与决策无关的部分不受影响：**ws 断开或主控重启不杀远端子进程**，在飞 turn 继续，
  重连后按同一套对账重挂到同一会话；只有**节点自身重启**才会终止其会话。
- 正当出口：确需让节点在主控缺席时也能持续跑，用 `auto` 模式。但 `auto` 等于把该
  机器完全交给 agent 自主执行（**不是默认值**，且留有审计痕迹），只在明确接受该
  风险时使用；`ask` 模式下不要把「主控必须在线」当成可以绕过的约束。

---

## 6. 最小走查清单

1. 节点机上准备好项目目录（不存在的目录不会被主控创建）。
2. 主控 `[node_link] enabled = true` + 合适的 `listen`，重启 core。
3. `sebas node-link -c <config> token` 取一次性 token（或从首启日志里拿）。
4. 节点机 `sebas-node --node-id <id> --control-plane ws://<主控地址>:9878 --join-token <token> --state-dir <dir>`。
5. `sebas node-link -c <config> list` 应看到该节点；节点侧状态目录出现 `node-id` 与
   `credential`（均 `0600`）。
6. 断开 token 重启节点（不带 `--join-token`）应仍能接入。
7. 撤销：`sebas node-link -c <config> revoke <id>`，节点再连接应被拒且成因指名吊销。

## 7. 进程托管（systemd）

节点不走主控的 watchdog 服务表：它是**独立二进制**，由操作者自己的 init 托管。仓库
不假设 systemd，但给出等价物最容易照抄的形态。

**就绪信号形态：`Type=simple`。** 节点没有需要通知 init 的"就绪"握手——它起来就
拨号，拨不上按 1s→60s 退避重试；因此不需要 `Type=notify`，也不引入 `sd_notify`
依赖。**永久性失败**（凭据被吊销 / 协议版本不兼容 / `wss://` / 未配对）以 **75**
退出，且 stderr 末行是 `startup-failure: <原因>`——`systemctl status` 与
`journalctl -u sebas-node` 末尾直接能看到它。

```ini
# /etc/systemd/system/sebas-node.service
[Unit]
Description=sebas execution node
After=network-online.target
Wants=network-online.target
# 永久性故障时不要无限空转：5 分钟内连续失败 10 次就停手，让人去看 journal。
StartLimitIntervalSec=300
StartLimitBurst=10

[Service]
# 见上：无需 notify。
Type=simple
User=sebas
# 把 join token / 路径之类从环境里给（可选）。
EnvironmentFile=-/etc/sebas/node.env
ExecStart=/usr/local/bin/sebas-node --config /etc/sebas/node.toml
Restart=on-failure
RestartSec=5
# 状态目录必须是**持久的**：凭据、节点本地会话日志都在里面；用
# RuntimeDirectory 会在每次重启后把执行事实删掉。
StateDirectory=sebas-node
# 节点只做出站连接，不需要任何 inbound 端口；也不需要额外 capability。
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/srv/work

[Install]
WantedBy=multi-user.target
```

**与主控同机时的注意**：节点**不监听任何端口**（反向拨号），默认配置下主控的
`[node_link] listen` 也是回环地址，所以同机部署只需让节点拨
`ws://127.0.0.1:9878`；两边状态目录互不相干（主控在它的配置目录下，节点在
`--state-dir`）。同机双进程、两条链路互不干扰由沙箱用例实际覆盖（见交付报告）。

> 本文档的端到端走查是否在本环境实际跑通，见交付报告；未在本环境验证的步骤已在
> 报告中标注（`docs/` 内不写未验证内容）。
