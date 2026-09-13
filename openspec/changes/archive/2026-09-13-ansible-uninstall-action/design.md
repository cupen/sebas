# 设计：ansible uninstall action

## Context

role 现状是单路径安装：`tasks/main.yml` 230 行从变量校验到 `sebas service --install` 一条龙（用户/目录/下载解压/二进制/config 渲染/服务安装）。没有卸载路径，下线一台机器只能手动 stop + 删文件，而目标机上的 `config.toml`（provider key、`sebas_config_extra` 凭据）、`core.secret`、sessions DB 都是泄露面。见 proposal.md — Why。

## Goals / Non-Goals

**Goals:** 变量切换动作、uninstall 幂等收敛、复用二进制自带的 `sebas service --uninstall` 作权威拆除、数据与密钥面清净、README 用法。
**Non-Goals:** 不做备份/导出（卸载即放弃数据）；不做多机编排策略变化；不改 install 行为一丝一毫。

## Decisions

### D1. `tasks/main.yml` 变分发器，安装流程原样迁入 `install.yml`

首个任务 = `sebas_action` 白名单断言（`install`/`uninstall`，非法值 `fail:` 快速失败，此时还没碰任何目标机状态）。install 分支 `include_tasks: install.yml`（内容 = 现 main.yml 全文，零语义改动），uninstall 分支 `include_tasks: uninstall.yml`。分发器保证默认值 `install` 在 `defaults/main.yml` 声明——不传变量永远走安装。

### D2. uninstall 步序（`tasks/uninstall.yml`）

1. **权威拆除**：`sebas_bin_path` 存在 → 执行 `sebas service --uninstall`（它知道 unit 名、数据目录语义，stop + disable + 删 unit + daemon-reload 一次做全）。二进制缺失 → 兜底：`systemctl stop/disable`（`failed_when: false`）+ 删 unit 文件（`<data_dir>` 内或 systemd 路径的 sebas unit，`find` 探测）+ `daemon-reload`。
2. **删二进制**：`sebas_bin_path` 与 seeded 副本 `<data_dir>/bin/sebas`（service --install 的稳定副本），各自 `state: absent` + 移除空父目录 `<data_dir>/bin`。
3. **清数据与密钥面**：`sebas_data_dir` 整目录（sebas.db / core.secret / downloads / router-usage / claude-sessions 等全部）、`sebas_config_path` 的父目录中 sebas 专属部分（config 在 `~/.sebas/` 内时随 data_dir 一并没了；配置外置时单独删文件）、`~/.config/sebas`。幂等由 `state: absent` 天然保证。
4. **删部署用户**：`userdel -r`（连 home，覆盖 2/3 的默认归属路径）。用户不存在不报错。
5. **控制机清理**：`delegate_to: localhost` 删 `/tmp` tarball 与解压目录，`failed_when: false` + `ignore_errors`，best-effort。
6. **debug 汇报**：`debug:` 输出卸载结果摘要（删了什么、什么本就不在）。

容错统一手法：目标态 = 不存在，`state: absent` / `failed_when: false` 全覆盖；每步都假设"上一步可能没执行过"。

### D3. 安全开关语义

uninstall 是破坏性动作，触发面收窄到显式传参：只有 `-e sebas_action=uninstall`（或 host_vars）会进卸载分支，默认 install 保证误跑 playbook 无害（spec 已定）。role 不做二次确认（Ansible 语义里 extra-vars 就是显式意图；README 写明破坏性）。

## Risks / Trade-offs

- [`sebas service --uninstall` 自身失败（如 unit 已半残）] → 该步 `failed_when: false`，后续兜底步骤仍收敛到"不存在"；卸载的验收标准是终态不是过程。
- [`userdel -r` 在 home 下有 role 之外文件时报错] → `failed_when: false` 后补 `file: state=absent` 兜底删 home。
- [config 外置路径（`sebas_config_path` 不在 data_dir 内）漏删] → 步骤 3 单独显式删该文件，不依赖目录递归。

## Migration Plan

纯 role 内改动，一次落地；对既有 install 用户零影响（main.yml 迁入 install.yml 后逐字不变）。

## Open Questions

（无。）
