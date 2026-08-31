## MODIFIED Requirements

### Requirement: Service unit generation

`sebas service --install` SHALL write a systemd **system** unit to
`/etc/systemd/system/sebas.service` with: `After=`/`Wants=`
`network-online.target`, `Type=simple`, `User=`/`Group=` set to the
`--user` value (the service never runs as root). The unit's `ExecStart`
SHALL run the **watchdog** entrypoint (`<fixed-binary> watchdog --config
<absolute config>`), not the bare core. The fixed binary path SHALL be
`<data_dir>/bin/sebas`, where `data_dir` is resolved from the config's
`[watchdog.storage].data_dir` and falls back to the `--user` home-derived
data dir when unset. At install time the current binary SHALL be seeded to
`<data_dir>/bin/sebas`; update replaces that file in place so a machine
reboot runs the latest version. Installation requires EUID 0. The unit
SHALL include hardening directives `NoNewPrivileges`, `ProtectSystem=full`,
`ProtectHome=read-only`, and `PrivateTmp`; it SHALL NOT set
`ProtectSystem=strict` (self-upgrade must be able to replace the binary).
The rendered `ExecStart` paths SHALL be systemd-escaped so paths containing
whitespace or special characters remain valid. `RUST_LOG` SHALL be taken
from `--log-level` when given, otherwise inherited from the installing
environment (falling back to `info` when unset/empty). A best-effort
symlink at `/usr/local/bin/sebas` SHALL point to the fixed binary path
(failure to create it is not an error). `Restart=on-failure`,
`RestartSec=5`, `WantedBy=multi-user.target` are unchanged.

#### Scenario: unit content

- **WHEN** installing as root with `--user sebas`
- **THEN** the unit runs `<data_dir>/bin/sebas watchdog --config <config>`
  under the `sebas` user, restarts on failure after 5 s, and boots at
  multi-user target

#### Scenario: privilege required

- **WHEN** `sebas service --install` runs as a non-root user
- **THEN** the command exits with code 4 without writing anything

#### Scenario: fixed binary seeded at install

- **WHEN** installing when `<data_dir>/bin/sebas` does not yet exist
- **THEN** the current binary is copied there and the unit's `ExecStart`
  references it

#### Scenario: paths with spaces escaped

- **WHEN** the config path contains a space
- **THEN** the rendered `ExecStart` is quoted/escaped and systemd parses it
  as a single argument

### Requirement: Service install validation and exit codes

The service installer SHALL validate and fail with distinct exit codes: 2
for argument conflicts (missing action, both install and uninstall); 3 when
the unit exists (without `--force`) or is absent on uninstall; 4 for
non-root, an empty or root `--user`, a **nonexistent `--user` account**, or
a non-absolute binary path; 5 for a missing or non-absolute `--config`; 6 on
unsupported platforms (macOS/Windows). `--force` overwrites an existing
unit. A new `--log-level` flag SHALL be accepted (install only, ignored by
uninstall) and validated to be a non-empty string.

#### Scenario: user must not be root

- **WHEN** installing with `--user root`
- **THEN** the command exits 4

#### Scenario: existing unit

- **WHEN** the unit file exists and `--force` is not passed
- **THEN** the command exits 3 leaving the existing unit untouched

#### Scenario: nonexistent user

- **WHEN** installing with `--user nosuchuser`
- **THEN** the command exits 4 before writing anything

### Requirement: Service start and uninstall

With `--auto-start`, install SHALL run `systemctl daemon-reload` and
`systemctl enable --now sebas.service`; without it, install only writes the
unit and daemon-reloads, printing manual enable instructions. In BOTH cases,
after writing the unit and daemon-reload, if the unit is currently active
(started), install SHALL explicitly `systemctl restart sebas.service` so a
repeated `install` is a deterministic "reload config and take effect"
operation. Uninstall requires
the unit to exist, then best-effort stops and disables it, removes the unit
file, and daemon-reloads.

#### Scenario: manual start path

- **WHEN** installing without `--auto-start`
- **THEN** the service is not enabled or started and the output tells the
  user how to do so

#### Scenario: uninstall absent unit

- **WHEN** uninstalling when no unit file exists
- **THEN** the command exits 3

#### Scenario: reinstall restarts a running service

- **WHEN** installing over an existing, currently-active unit
- **THEN** the unit is rewritten, daemon-reloaded, and restarted so the new
  config takes effect