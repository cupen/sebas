## Why

role 目前只有安装路径：重复跑是重装/升级，想下线一台机器只能手动 stop + 删文件。而留在目标机上的 `config.toml`（provider API key、`sebas_config_extra` 里的凭据）、`core.secret`（密钥材料）、sessions DB 都是泄露面——卸载必须连数据一起清。复用既有 role，用一个变量切换动作即可，不需要新 role。

## What Changes

- role 新增变量 **`sebas_action: install | uninstall`**（默认 `install`；非法值 fail fast）。`tasks/main.yml` 变为分发器：现有安装流程原样迁入 `tasks/install.yml`，新增 `tasks/uninstall.yml`。
- **uninstall 流程**（复用二进制自带的 `sebas service --uninstall` 作权威第一步）：
  1. 二进制在位 → `sebas service --uninstall`（stop + disable + 删 unit + daemon-reload）；不在位 → 裸 `systemctl` + 删 unit 文件兜底；步骤全部容错（目标态 = 不存在，幂等）。
  2. 删两份二进制：`sebas_bin_path`（/usr/local/bin/sebas）与 seeded 副本 `<data_dir>/bin/sebas`。
  3. 清数据与密钥面：`sebas_data_dir`（sebas.db、core.secret、downloads、router-usage 等全部）、`sebas_config_path`、`~/.config/sebas`。
  4. 删部署用户（`userdel -r` 连 home 一起，覆盖上述路径的默认归属）。
  5. 控制机侧 `/tmp` 的 tarball 与解压目录清理（delegate_to localhost，best-effort）。
  6. 汇报卸载结果。
- README 补 uninstall 用法（`-e sebas_action=uninstall`）。

## Capabilities

### New Capabilities

- `deployment`：sebas 单机 Ansible 部署 role 的行为规约——install（固化现状行为）与 uninstall（含数据清除、幂等收敛）两个 action 的要求。

### Modified Capabilities

（无）

## Impact

- 代码：`.ansible/roles/sebas/tasks/main.yml` 拆分为分发器 + `install.yml`/`uninstall.yml`，`defaults/main.yml` 增 `sebas_action`；README。
- 安全性：uninstall 是破坏性动作，仅由显式传参触发；默认 `install` 保证误跑 playbook 无害。

## Non-goals

- 不做"删程序留数据"的中间档——卸载即清库，避免泄露是本意。
- 不做多机编排的差异化处理（沿用现有 playbook / host_vars 机制）。
- 不改 `sebas service --uninstall` 的二进制侧行为。
