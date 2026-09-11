## ADDED Requirements

### Requirement: mode 透传与中途切换用例

进程级 e2e 套件 SHALL 覆盖 agent mode 链路，全部用例走既有桩（fake-claude 等），零真模型调用：

- **argv 透传用例**：以带/不带 `mode` 各创建一个本机 claude 会话，经 fake-claude journal 断言子进程 argv 是否含映射后的 `--permission-mode` 值（ask/缺省 → 无该参数，edit → acceptEdits，allow/auto → bypassPermissions）。
- **行为差异化用例**：以 mode=allow 创建会话并发送 `perm` 场景消息，断言门控放行、回合完成且未产生审批卡片（与 ask 模式下同场景产生审批形成对照）。
- **中途切换用例**：对运行中的 fake-claude 会话 POST mode 切换，断言 journal 记录运行时模式切换、快照 mode 更新（接受路径），以及失败路径产生非致命错误且会话存活。
- **拒绝用例**：未知 mode 的创建/切换请求返回 400。

#### Scenario: argv 透传断言

- **WHEN** 以 mode=allow 创建会话并完成一个回合
- **THEN** journal 中该子进程 argv 含 `--permission-mode bypassPermissions`；不带 mode 的对照会话 argv 不含该参数

#### Scenario: allow 模式下 perm 场景免审批

- **WHEN** 以 mode=allow 创建的会话收到 `perm` 场景消息
- **THEN** fake-claude 不产生 PreToolUse 审批交互，工具直接执行并完成回合

#### Scenario: 中途切换记录于 journal

- **WHEN** 对运行中的 fake-claude 会话切换 mode
- **THEN** journal 记录该运行时权限模式切换，会话快照的 mode 更新为新值
