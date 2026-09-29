## MODIFIED Requirements

### Requirement: Uninstall removes service, binaries, data, and user

The uninstall flow SHALL converge the target host to "sebas 从未存在"：stop
and remove the systemd service, remove both managed binaries, delete the
data and secret surface, and remove the deploy user with its home. Every
step SHALL be idempotent — re-running uninstall on an already-clean host
SHALL succeed without errors — and tolerate partial states (service already
gone, binary already missing). The flow SHALL use `sebas service
--uninstall` as the authoritative teardown step when the binary is present;
when it is absent, the role SHALL fall back to direct systemctl stop/disable
plus unit-file removal. The data cleanup SHALL include the sebas home
directory (the state databases, `core.secret`, sockets and run state, media
downloads, usage logs, and upgrade artifacts), the rendered config file
wherever it was rendered, and `~/.config/sebas` from legacy installs. The
deploy user SHALL be removed together with its home directory. Cleanup of
controller-side temporary artifacts (downloaded tarball and extraction
directory under `/tmp`) SHALL run on the control node and remain best-effort
— its failure SHALL NOT fail the play.

#### Scenario: Full uninstall on a live install

- **WHEN** uninstall runs against a host with a running sebas service
- **THEN** the service is stopped and its unit removed, both binaries are
  gone, the sebas home and rendered config (including `core.secret`) are
  deleted, the deploy user and home are removed, and the play reports
  success

#### Scenario: Uninstall is idempotent

- **WHEN** uninstall runs a second time on the same host
- **THEN** every step observes an already-absent target, no task fails, and
  the play reports success

#### Scenario: Missing binary falls back to bare teardown

- **WHEN** the managed binary is absent but a systemd unit or running
  process remains
- **THEN** the role stops/disables via systemctl directly and removes the
  unit file, then continues with data and user cleanup

#### Scenario: Controller-side cleanup is best-effort

- **WHEN** the temporary tarball or extraction directory on the control node
  cannot be removed
- **THEN** the play continues and reports success; the residue is left for
  the operator
