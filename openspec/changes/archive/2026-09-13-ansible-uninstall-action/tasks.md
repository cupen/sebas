# Tasks: ansible-uninstall-action

## 1. 分发器与 install 迁移

- [x] 1.1 `defaults/main.yml` 增 `sebas_action: install`；`tasks/main.yml` 改为白名单断言 + 分发器，现安装流程逐字迁入 `tasks/install.yml`；非法值 fail fast 的断言任务落地

## 2. uninstall 流程

- [x] 2.1 `tasks/uninstall.yml` 按 design D2 步序实现：权威 `sebas service --uninstall`（二进制在位）/ 裸 systemctl + unit 文件兜底（不在位）→ 删双二进制 → 清 `sebas_data_dir` / `sebas_config_path` / `~/.config/sebas` → `userdel -r` → 控制机 `/tmp` best-effort 清理 → debug 汇报；全步骤幂等容错

## 3. 文档与验证

- [x] 3.1 README 补 uninstall 用法（`-e sebas_action=uninstall`）与破坏性警示、默认 install 保证
- [x] 3.2 语法与静态验证：`ansible-playbook --syntax-check` 过；有 docker/VM 可用则对容器跑一轮 install→uninstall→再 uninstall（幂等）实测，没有则以 syntax-check + 显式逐任务核对汇报（如实说明验证边界）
