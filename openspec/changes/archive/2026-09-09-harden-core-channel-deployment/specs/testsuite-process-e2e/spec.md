# Delta — testsuite-process-e2e

## ADDED Requirements

### Requirement: 无密钥装配旅程（事故回归）

套件 SHALL 验证不注入 `SEBAS_CORE_SECRET` 的 detached 双进程装配：core 与独立 webui 均无 env secret，core 靠自动武装、webui 靠 secret 文件发现完成连接。此为真机"socket absent"事故的回归用例。

#### Scenario: 双进程均无 secret 时启动即可达

- **WHEN** core 与独立 webui 按沙箱配置启动且两者环境均无 `SEBAS_CORE_SECRET`
- **THEN** webui `/api/summary` 的 `reachability.ok` 变为 true，secret 文件存在于解析路径
- **AND** 经 webui HTTP 面完成一次会话往返（创建 → Done）

#### Scenario: 既有 env 注入路径不回归

- **WHEN** 既有带 `SEBAS_CORE_SECRET` 的沙箱用例照常运行
- **THEN** 全部保持绿（env 优先语义未破坏既有装配）

### Requirement: 密钥轮换自愈旅程

套件 SHALL 验证 core 重启换钥后，不重启的 webui 自动恢复：杀掉 core → 重启 core（新随机 secret 覆写 secret 文件）→ webui 在重连退避内恢复可达。

#### Scenario: 重启 core 后 webui 不重启自愈

- **WHEN** 双进程装配达 reachable 后 core 被终止并以同 config 重启
- **THEN** webui 进程不重启，期间 reachability cause 如实呈现，随后 `reachability.ok` 恢复 true

### Requirement: 监督重启恢复旅程

套件 SHALL 以 watchdog 监督形态验证崩溃自愈（收窄验收账本缺口 #3）：watchdog 拉起 core + webui，杀掉 core，supervisor 按重启策略自动拉起，webui 随之恢复。

#### Scenario: watchdog 自动重启被杀的 core

- **WHEN** watchdog 监督下的 core 进程被杀死
- **THEN** supervisor 在重启延迟内重新拉起 core，通道 socket 重新出现，webui `reachability.ok` 恢复 true
