## MODIFIED Requirements

### Requirement: 有界时间与平台门控

每个用例 MUST 以显式超时为界(禁止无界等待),套件总时长 SHOULD 控制在数分钟内。套件 MUST 在 Linux 与 Windows 双平台通过编译并可运行——不得存在未门控的平台性编译期依赖;平台相关用例(如 SIGTERM)MUST 按平台条件编译门控,在不支持的平台上跳过且不判失败。套件用例 MUST 以 `#[ignore]` 标注,不进入默认 `cargo test` 路径。

#### Scenario: 平台不支持时跳过而非失败

- **WHEN** 套件在无 SIGTERM 语义的平台(如 Windows)上运行
- **THEN** 平台门控用例被跳过,其余用例正常运行,套件整体不因此失败

#### Scenario: 挂起子进程不拖垮套件

- **WHEN** 任一外部依赖(核心/webui/ACP 子进程)未在用例超时内给出预期响应
- **THEN** 该用例在超时处失败并留下可诊断的日志/残留路径,套件其余用例继续执行

#### Scenario: 双平台编译通过

- **WHEN** 在 Linux 与 Windows 上分别执行 `cargo test --test testsuite_e2e_test --no-run`
- **THEN** 两平台均编译成功,不存在因平台差异引发的编译错误

## ADDED Requirements

### Requirement: Windows 进程树收割

沙箱拆卸在 Windows 上 MUST 收割被记录子进程的整棵进程树(含核心派生的孙进程,如 ACP 子进程与被监督重生的核心),不留孤儿进程占用沙箱端口或锁住沙箱目录;对已退出目标的收割尝试 SHALL 被视为成功,不阻塞沙箱目录的清理。

#### Scenario: Windows 拆卸不留孤儿

- **WHEN** 任一用例在 Windows 上结束(无论通过或失败),且核心在用例期间派生过子进程
- **THEN** 沙箱拆卸后不存在仍占用沙箱端口或持有沙箱目录句柄的孤儿进程,沙箱目录可被删除

### Requirement: harness 平台安全

一键入口与沙箱清理流程 MUST 在 Windows 与 Linux 上均安全执行:进程存活探测 SHALL 在任何平台上都不得终止被探测进程;清理流程 SHALL 仅作用于确认为陈旧沙箱的目标;不得因引用目标平台不存在的信号而中断入口流程。

#### Scenario: Windows 存活探测不误杀

- **WHEN** 在 Windows 上执行 `invoke testsuite-e2e` 触发陈旧沙箱清理,探测到存活进程的 pid
- **THEN** 探测动作不终止该进程,仅陈旧沙箱记录的进程被清理

#### Scenario: 入口不因平台缺失的信号中断

- **WHEN** 在 Windows 上执行 testsuite 入口及其拆卸路径
- **THEN** 全程不因引用平台缺失的信号(如 SIGKILL)抛异常而中断
