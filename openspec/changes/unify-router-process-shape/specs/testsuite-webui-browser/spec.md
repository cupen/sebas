## MODIFIED Requirements

### Requirement: 沙箱装配与清理边界

套件 SHALL 以一次性沙箱目录装配被测后端：`sebas core --config <path>
--webui` 与 `sebas router --config <path> --debug` **两进程形态**（router
不再内嵌于 core；core 旗标不含 `--router`），agent 为仓库自带 fake-claude
桩；MUST 覆盖全部默认路径与凭据 env（state DB、state file、provider
overlay、core secret、auth file），MUST NOT 绑定 9797 或读写真实
`~/.sebas`。套件结束（无论成败）SHALL 结束后端进程（含 router 子进程）
并删除沙箱目录——POSIX 上后端 SHALL 优雅退出（SIGTERM），Windows 上允许
硬终止、目录删除尽力而为；失败时 SHALL 保留现场目录并在输出中给出路径，
供排障复用。

#### Scenario: 启动即隔离

- **WHEN** 套件拉起后端
- **THEN** 全部状态（config、DB、media、ACP 会话目录、auth 文件）位于一次性沙箱目录，webui 端口 ≠ 9797，进程 env 不含指向真实 `~/.sebas` 的路径；core 与 router 为两个独立进程且 router 带 debug test provider

#### Scenario: 成功退出清理

- **WHEN** 全部用例通过、套件退出
- **THEN** 后端进程（core 与 router）结束、沙箱目录被删除、端口恢复可用；POSIX 上后端为 SIGTERM 优雅退出，Windows 上允许硬终止（清理尽力而为）

#### Scenario: 失败保留现场

- **WHEN** 任一用例失败
- **THEN** 套件报告失败并在输出中保留沙箱目录路径，目录内含后端日志可供复现
