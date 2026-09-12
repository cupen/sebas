# deployment Specification

## Purpose
sebas 单机 Ansible 部署 role 的行为规约：一个 role 通过 `sebas_action` 变量在 install 与 uninstall 两个动作间切换。install 固化既有部署行为；uninstall 以幂等、容错的方式把服务、二进制、数据与凭据面从目标机撤干净——下线即彻底下线，不留凭据残留。

## Requirements

### Requirement: Action selection via sebas_action

The role SHALL select its behavior from the `sebas_action` variable with exactly two legal values: `install`（默认）and `uninstall`. Any other value SHALL fail the play immediately with a diagnostic naming the variable and the legal values. The default MUST be harmless: running the playbook without the variable SHALL perform the existing install behavior, never a destructive action.

#### Scenario: Default action installs

- **WHEN** the playbook runs without `sebas_action`
- **THEN** the role performs the install flow exactly as before this change

#### Scenario: Illegal action fails fast

- **WHEN** `sebas_action` is set to a value other than `install` or `uninstall`
- **THEN** the play fails immediately with an error naming `sebas_action` and the two legal values, and no task touches the target host

### Requirement: Uninstall removes service, binaries, data, and user

The uninstall flow SHALL converge the target host to "sebas 从未存在"：stop and remove the systemd service, remove both managed binaries, delete the data and secret surface, and remove the deploy user with its home. Every step SHALL be idempotent — re-running uninstall on an already-clean host SHALL succeed without errors — and tolerate partial states (service already gone, binary already missing). The flow SHALL use `sebas service --uninstall` as the authoritative teardown step when the binary is present; when it is absent, the role SHALL fall back to direct systemctl stop/disable plus unit-file removal. The data cleanup SHALL include the data directory (sessions DB, `core.secret`, downloads, usage logs), the rendered config file, and `~/.config/sebas`. The deploy user SHALL be removed together with its home directory. Cleanup of controller-side temporary artifacts (downloaded tarball and extraction directory under `/tmp`) SHALL run on the control node and remain best-effort — its failure SHALL NOT fail the play.

#### Scenario: Full uninstall on a live install

- **WHEN** uninstall runs against a host with a running sebas service
- **THEN** the service is stopped and its unit removed, both binaries are gone, the data directory and config (including `core.secret`) are deleted, the deploy user and home are removed, and the play reports success

#### Scenario: Uninstall is idempotent

- **WHEN** uninstall runs a second time on the same host
- **THEN** every step observes an already-absent target, no task fails, and the play reports success

#### Scenario: Missing binary falls back to bare teardown

- **WHEN** the managed binary is absent but a systemd unit or running process remains
- **THEN** the role stops/disables via systemctl directly and removes the unit file, then continues with data and user cleanup

#### Scenario: Controller-side cleanup is best-effort

- **WHEN** the temporary tarball or extraction directory on the control node cannot be removed
- **THEN** the play continues and reports success; the residue is left for the operator

### Requirement: README documents the uninstall entry

The role README SHALL document the uninstall action with its invocation (`-e sebas_action=uninstall`), state that it is destructive (service, binaries, data, secrets, and the deploy user are removed), and note that omitting the variable keeps the default install behavior.

#### Scenario: Operator can find the uninstall usage

- **WHEN** the role README is consulted
- **THEN** it shows the uninstall invocation, the destructiveness warning, and the install-by-default guarantee
