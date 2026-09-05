## 1. 目录骨架与默认 inventory

- [x] 1.1 建 `ansible/` 骨架：`site.yml`（空 play 骨架）、`ansible.cfg`（`defaults.inventory = inventory/hosts.ini`）、`inventory/hosts.ini`（`localhost ansible_connection=local ansible_python_interpreter=auto_silent`）、`roles/sebas/{tasks,defaults,templates,handlers}` 目录树。验证：`ansible-playbook site.yml --list-hosts` 列出 localhost，`--list-tasks` 不报错

## 2. 部署用户、目录与二进制供给

- [x] 2.1 defaults/main.yml 全量变量（`sebas_deploy_user`、`sebas_webui_port=9797`、`sebas_artifact_source=release`、`sebas_version=latest`、`sebas_bin_path=/usr/local/bin/sebas`、`sebas_config_path` 等）+ tasks 前置：建部署用户（已存在则跳过）、`~/.sebas` 与 config 目录布局、属主归属。验证：Linux 目标机上跑 play，`id sebas` 与目录属主符合预期，重复执行不报 changed
- [ ] 2.2 二进制供给三模式：`release`（`sebas_version=latest` 先调 GitHub API 解析 tag，再 `get_url` 下载 `sebas-<tag>-x86_64-unknown-linux-gnu.tar.gz` 解包安装到 `sebas_bin_path`；显式版本跳过 API）、`file`（controller 路径 copy）、`preinstalled`（stat 校验存在，不动二进制）。验证：三种模式各跑一遍，`sebas_bin_path` 处二进制可执行（`--version` 或 `--help` 退出码 0）；`file` 模式断言无网络请求

## 3. 配置渲染与服务安装

- [x] 3.1 `templates/sebas-config.toml.j2` 最小模板（watchdog storage data_dir、webui host/port、acp.claude path、feishu disabled，全部变量化有默认）+ config 任务 notify restart handler（service 存在后生效）。验证：渲染文件含预期段落且无空必填项；模板不含任何 `SEBAS_CORE_SECRET` 相关内容（grep 断言）
- [x] 3.2 前置验证（决定 3.3 写法）：在 WSL/VM 目标机手动执行 `sebas service --install` 两次，确认重复执行行为（幂等 changed / 报错文案），把结论写进 design 的 Open Question 并定 playbook 的 `changed_when` 语义
- [ ] 3.3 服务安装任务：以部署用户 + 绝对 config 路径执行 `sebas service --install`，仅该任务 `become: true`，按 3.2 结论处理幂等；服务未装时才装、装过则收敛。验证：`systemctl is-active sebas` = active，`systemctl show -p User sebas` = 部署用户，ExecStart 为 watchdog 入口；重跑 play 不报错

## 4. 健康检查与幂等收敛

- [ ] 4.1 部署后健康检查：`systemctl is-active` + `uri` GET `http://127.0.0.1:{{ sebas_webui_port }}/health`，带启动重试窗（如 10 次 × 3s），rescue 里 assert 报出具体失败断言与观测值。验证：正常部署 playbook 绿；把 `sebas_webui_port` 改成被占用/错误端口重跑，playbook 红且错误信息指明是哪条断言、观测到什么
- [ ] 4.2 幂等收敛：输入不变重跑一遍，`changed=0` 且 service 未重启（对比 `ActiveEnterTimestamp`）；改一个 config 变量（如 webui port）重跑，config 更新且恰好一次 restart（handler 通知一次、时间戳变化一次）。验证：两段命令输出分别满足上述断言

## 5. 文档与端到端收尾

- [x] 5.1 README 部署章节加 ansible 小节：前提（控制机 ansible ≥2.x，Windows 控制机注明走 WSL；目标机 systemd + 部署用户免密 sudo）、默认本地 inventory 用法、三模式切换示例、升级 = 重跑 playbook 与 `sebas update` 的关系声明；可选 `tasks.py` 加 `deploy` 入口（标注需 Linux/WSL 控制机）。验证：README 小节命令逐条可复制执行；`invoke deploy`（若做）在 WSL 下能转发到 `ansible-playbook site.yml`
- [ ] 5.2 端到端收尾：在 WSL/Linux 目标机以默认 inventory 完整跑通 release 模式部署（systemd active + `/health` 200 + 幂等重跑），`ansible-playbook site.yml --syntax-check` 与 `ansible-lint`（若可用）通过；conventional commit。验证：完整执行记录（playbook recap 两遍 + 健康检查输出）附在提交说明或 PR 描述中
