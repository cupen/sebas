## Context

正式部署形态已有两个既有事实：`sebas service --install`（cli-service 能力）负责 systemd unit 生成、非 root 降权、二进制 seed 到 `<data_dir>/bin/sebas`，要求 EUID 0 与绝对路径 config；`SEBAS_CORE_SECRET` 由 watchdog 启动时自行生成注入（`src/watchdog.rs:91`），部署侧不出现。release 产物为 `sebas-<tag>-x86_64-unknown-linux-gnu.tar.gz`（`.github/workflows/release.yml`，asset 名可推导）。仓库现有部署文档只有 README 的手工命令与 Docker 两节，无可执行部署载体。

## Goals / Non-Goals

**Goals:**

- `ansible/` 一套 playbook：单机 Linux 部署收敛为一条命令，且重复执行幂等
- 默认 inventory（localhost, connection=local）开箱即用，控制机与目标机同机
- 三种二进制来源（release / file / preinstalled）覆盖参考部署与本地开发联调
- 部署后自动健康判定（systemd active + `/health`）

**Non-Goals:**

- 多机编排、容器路径、molecule/CI 自动化测试、Windows 目标机（见 proposal Non-goals）
- 自动安装 ansible 本身或目标机系统依赖的 bootstrap（文档注明前提即可）

## Decisions

**D1 — 单 role + site.yml 的扁平布局**
`ansible/{site.yml, ansible.cfg, inventory/hosts.ini, roles/sebas/{tasks,defaults,templates,handlers}}`。`ansible.cfg` 把 `defaults.inventory` 固定指向仓库内 `inventory/hosts.ini`，实现"不带参数即本地"。一个能力一个 role，不做 collection、不做多 role 拆分——拆分没有第二个消费者。
*备选*：collection 形式发布到 galaxy —— 没有外部用户，过度设计。

**D2 — release 模式先解析 latest tag，再拼 asset 名下载**
`sebas_version: latest`（默认）时先调 GitHub API `releases/latest` 取 tag，再拼 `sebas-<tag>-x86_64-unknown-linux-gnu.tar.gz` 用 `get_url` 下载解包；显式版本号则跳过 API 调用。安装到 managed 路径 `/usr/local/bin/sebas`，交给 `service --install` 自行 seed 到 data_dir（不重复造 seed 逻辑）。
*备选*：固定默认版本号 —— 会漂移且误导；latest 重定向 URL —— asset 名含 tag，`latest/download` 固定名不成立。

**D3 — config 渲染用最小模板 + 变量覆盖，handler 触发单次 restart**
`sebas-config.toml.j2` 只渲染必要段（watchdog storage data_dir、webui host/port、acp.claude path、feishu disabled），所有段都有变量默认值；用户变量合并覆盖。config 任务 `notify` restart handler，保证"变更才重启、一次变更一次重启"。注意 AGENTS.md 沙箱规则揭示的坑：若干路径默认落 `~/.sebas`（state_file、download_dir、sessions_dir、channel_path）——正式部署以部署用户 home 为根是合法默认，模板不额外铺开这些路径，仅 data_dir 与 webui 端口进默认渲染面。
*备选*：整份 config.toml.example 参数化 —— 变量爆炸，维护两份样例。

**D4 — become 策略：play 级 become，属主用 owner= 显式落**
实现修正（替代原"只圈 install 一步"方案）：systemd 正型部署必然要建用户、写 `/etc/systemd`、写部署用户属主的文件——控制连接账户（root 或免密 sudo 者）不等于部署用户，散点 become 反而绕。故 play 级 `become: true`（前提写进 README：免密 sudo 或直接 root），所有落在部署用户名下的文件/目录用 `owner=/group=` 显式归属。
*备选*：只 install 一步 become —— 连接账户写不了部署用户的 home，属主来回 chown，否决。

**D7 — `service --install` 幂等门控（spike 已定案：代码阅读 + WSL 实测）**
`service --install` 本身**非幂等**：unit 已存在且无 `--force` → 退出码 3（代码 `src/service.rs:364`；WSL 实测复现："unit already exists … use --force"，连跑两次均 exit 3）。seed 副本是**拷贝**而非软链（`seed_stable_binary`，`src/upgrade.rs:576`；实测确认 seed 落 `<data_dir>/bin/sebas`、root 属主 0755），升级二进制后必须重跑 install 才重新 seed。实测还确认：install 依次做 config 校验 → seed → `/usr/local/bin/sebas` 软链（best-effort）→ 写 unit（安装时 ExecStart 为 `<seed> watchdog --config <config>`，与当时的 cli-service spec 一致；rename-cli-surface 后新装 unit 烘 `run --config`，旧 unit 靠 `watchdog` 隐藏别名继续启动）→ `daemon-reload`；非 systemd 主机（WSL）在 daemon-reload 失败 exit 1，**前置行为已发生**。因此 playbook 门控执行：unit 不存在（首装）或 `<data_dir>/bin/sebas` 与供给二进制 sha256 不一致（升级）才跑 `--force`，其余跳过；config 变更走 restart handler（首次部署 install 已用新配置拉起，handler 以 `when: sebas_install is not defined or not sebas_install.changed` 抑制，保证"一次变更一次重启"）。
*仍未实测*：`enable --now`、restart-if-active、`systemctl is-active` 等依赖 systemd 的路径——本机无 systemd 目标机（见 D8），待真机复验。

**D5 — 健康检查双断言**
`systemctl is-active sebas` + `uri` GET `http://127.0.0.1:{{ sebas_webui_port }}/health`（默认 9797），`block/rescue` 里用 assert 报告具体哪条断言、观测值是什么。超时给重试窗（服务启动 grace）。

**D6 — 本地测试环境的验证方式**
控制节点不支持 Windows——本地测试路径写明 WSL/Linux 下 `ansible-playbook site.yml`；验证 = 真跑一遍 + 立即重跑一遍断言幂等（第二遍 changed=0、无 restart）。不做 molecule。
*备选*：molecule + Docker 容器当目标机 —— systemd-in-docker 的额外复杂度不值。

## Risks / Trade-offs

- [`service --install` 重复执行非幂等（unit 已存在退出码 3）] → 已定案，见 D7：stat + checksum 门控 + `--force`。
- [GitHub API rate limit / 目标机无外网] → `file` 模式兜底（离线部署合法路径）；API 仅 release+latest 时调用。
- [部署用户无免密 sudo] → install 步失败即停，报错信息指向前提；不静默降级。
- [二进制手工升级与 `sebas update` 自升级并存] → 文档声明：用 playbook 部署的机器，升级 = 重跑 playbook（或继续用 `sebas update`，二者都以 `<data_dir>/bin/sebas` 为准，不冲突但 playbook 不感知 update 后的版本漂移，健康检查不校验版本）。

## Migration Plan

纯新增目录，不影响既有部署路径。回滚 = 删除 `ansible/` 目录与 README 小节。已部署的机器卸载走既有 `sebas service --uninstall` + 删用户/目录，playbook 不提供 destroy（非目标，避免误删数据）。

**D8 — 验证环境与验证边界（远程 systemd 目标机复验后更新）**
控制机：WSL2 kali（ansible 2.21）。目标机双形态均验证：
- **本地**（默认 inventory，localhost/connection=local）：用户/目录/属主、file/preinstalled 供给、幂等重跑 changed=0、config 渲染 + 真实 Linux 二进制开机冒烟（/health ok、SIGTERM 优雅退出）。
- **远程 systemd 目标机**（`-i` 指向仓库外 inventory，仅写 ssh 别名，凭据与地址零入库）：全量首跑绿（install changed → active → /health ok，handler 抑制）；无变更重跑 changed=0 且 `ActiveEnterTimestamp` 不动；config 变更恰一次重启后新端口健康绿；占位端口负面用例 playbook 红且 rescue 报出双观测值（`service_state=active` + `http_status=404`——进程存活断言会撒谎，HTTP 断言兜住）。
- 实现期发现并修复：① handler 在 play 末尾才 flush，健康检查先于重启跑——`flush_handlers` 前置到 install 之前；② "systemd active 但 webui 子进程僵死"的假健康不自愈——健康检查加"HTTP 重试耗尽 → 重启一次 → 复验"内层兜底（healthy 运行不触发重启，不违幂等要求）。
- 运行时观察（非本变更范围）：watchdog 对僵死的 webui 子进程不自动 respawn，需单元重启恢复——另行立项跟踪。
- release 模式：仓库公开但尚无 release（`/releases/latest` 404），asset 名按 release.yml 模板（`sebas-<tag>-<target>.tar.gz`）实现，首发 release 后核对 tag 命名即可。
- ansible-lint 控制机未安装（任务标注"若可用"），`--syntax-check` 全过。

## Open Questions

- （已关闭，见 D7）`service --install` 幂等行为 → 非 `--force` 退出码 3；playbook 用 stat+checksum 门控。运行时复验待真机 systemd 目标机，见 D8 验证边界。
