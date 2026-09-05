## Why

sebas 的正式部署形态（systemd + watchdog）目前只有 README 里两条手工命令，新机器从零到可用要人肉走完：建用户、放二进制、写 config.toml、`service --install`、验证。步骤没有可执行的载体，漂移全靠读文档。一套 ansible playbook 把部署流程固化成可执行、可重复的脚本；inventory 默认指向本地（`ansible_connection=local`），同一份脚本既是参考部署样板，也是本地测试环境的一键拉起工具。

## What Changes

- 新增 `ansible/` 目录：playbook + role，覆盖 Linux 主机上的完整部署流程——部署用户与目录布局、二进制供给、config.toml 渲染、`sebas service --install`（systemd unit + watchdog）、部署后健康检查（systemd active + WebUI `/health` 200）
- 默认 inventory 指向 `localhost`（connection=local），开箱即用于本机部署与测试；目标机为 Linux（systemd 形态）
- 二进制来源参数化（`sebas_artifact_source`）：`release`（GitHub release tar.gz，默认）/ `file`（指定本地已构建二进制，供开发联调）/ `preinstalled`（目标机已有二进制，只做配置与服务）
- playbook 幂等：重复执行收敛到同一状态，不重复生成密钥、不重复装服务
- README 部署章节增加 ansible 入口，与现有 systemd/Docker 两节并列

## Capabilities

### New Capabilities

- `ansible-deploy`：ansible 部署面的行为要求——默认本地 inventory、二进制来源三种模式、config 渲染与密钥边界（`SEBAS_CORE_SECRET` 由 watchdog 自管，脚本不碰）、幂等收敛、健康检查判定

### Modified Capabilities

（无）

## Impact

- 新增 `ansible/` 目录（playbook、roles、inventory、README），纯新增，不改动任何 Rust 代码与现有部署路径
- `README.md` 部署章节追加 ansible 小节
- `tasks.py` 可选增加 `invoke deploy` 便捷入口（指向本地 inventory）

## Non-goals

- 不改 Docker 部署路径与镜像构建
- 不做多机编排（core / gateway / webui 分机拆分部署）——单机 watchdog 形态
- 不做 molecule 或 CI 中的 playbook 自动化测试——本地手工验证
- 不支持 Windows 目标机
- 不变更 `sebas update` 升级机制与 watchdog 密钥注入机制
