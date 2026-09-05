## Purpose

Reproducible single-host deployment of sebas on Linux via Ansible: the playbook is the executable reference for the official systemd + watchdog form, and the bundled default inventory turns the same playbook into a one-command local test environment.

## ADDED Requirements

### Requirement: Default inventory targets the local host

The repository SHALL ship a default inventory whose only host is `localhost` with local connection semantics (no SSH). Running the playbook with no inventory override SHALL deploy against the control machine itself. Users targeting a remote host SHALL only need to supply their own inventory; no playbook content SHALL depend on which inventory is used.

#### Scenario: Out-of-the-box local run

- **WHEN** the playbook runs with the bundled default inventory and no extra arguments
- **THEN** all plays execute against the local machine without any SSH connection attempt

#### Scenario: Remote inventory swap

- **WHEN** the playbook runs with a user-provided inventory listing a remote host
- **THEN** the same deployment flow executes against that host over SSH

### Requirement: Binary provisioning modes

The playbook SHALL provision the sebas binary according to a deployment variable `sebas_artifact_source` with exactly three modes:

- `release` (default): download the sebas release archive for the target platform from the project's GitHub releases and install the contained binary
- `file`: copy a controller-local binary from a configured path to the target
- `preinstalled`: use a binary already present on the target and skip provisioning

#### Scenario: Default release download

- **WHEN** the playbook runs with defaults on a host without sebas installed
- **THEN** the release archive is downloaded and the extracted binary is installed at the managed binary path with executable permission

#### Scenario: Developer-supplied binary

- **WHEN** `sebas_artifact_source=file` and a local binary path is configured
- **THEN** that exact binary is placed at the managed binary path and no network download occurs

#### Scenario: Preinstalled binary

- **WHEN** `sebas_artifact_source=preinstalled`
- **THEN** the playbook verifies a sebas binary exists on the target and proceeds with configuration and service setup without modifying the binary

### Requirement: Configuration rendering with secret boundary

The playbook SHALL render the target's config.toml from deployment variables onto a minimal viable default (no required user input for a WebUI-only deployment). The playbook SHALL NOT create, read, rotate, or persist the core channel secret (`SEBAS_CORE_SECRET`): that secret is owned by the watchdog at runtime.

#### Scenario: Minimal WebUI-only deployment

- **WHEN** the playbook runs with no configuration variables beyond defaults
- **THEN** a valid config.toml is rendered (Feishu disabled or absent) and the deployed service starts with it

#### Scenario: Secret boundary respected

- **WHEN** the playbook runs in any mode
- **THEN** no playbook task or rendered file contains a generated or stored `SEBAS_CORE_SECRET` value

### Requirement: Service installation through the official entrypoint

The playbook SHALL install the system service by invoking `sebas service --install` (the cli-service capability) with the deployed user and absolute config path, escalating privileges only for that step. The playbook SHALL NOT hand-write or modify the systemd unit file. The resulting service SHALL run the watchdog entrypoint as a non-root user.

#### Scenario: Service installed and running

- **WHEN** the playbook completes on a clean host
- **THEN** a `sebas` systemd unit exists, `systemctl is-active sebas` reports active, and the unit's user is the configured non-root deployment user

### Requirement: Post-deploy health check

After service installation the playbook SHALL verify deployment health: the systemd service is active AND the WebUI `/health` endpoint responds successfully on the configured port. On failure the playbook SHALL stop and report which check failed with the observed evidence.

#### Scenario: Healthy deployment passes

- **WHEN** the service is active and `/health` returns success
- **THEN** the playbook finishes successfully and reports the service state and WebUI URL

#### Scenario: Broken deployment fails loudly

- **WHEN** `/health` does not respond successfully after installation
- **THEN** the playbook fails, naming the failed check (service state vs HTTP health) and the observed value

### Requirement: Idempotent convergence

Re-running the playbook with unchanged inputs SHALL converge without error and without restarting the running service. When rendered configuration inputs change, the playbook SHALL update the config and restart the service exactly once.

#### Scenario: Re-run with no changes

- **WHEN** the playbook runs a second time with identical inputs
- **THEN** it completes successfully, the service is not restarted, and the deployed files are unchanged

#### Scenario: Config change triggers single restart

- **WHEN** a configuration variable changes between runs
- **THEN** the rendered config is updated and the service restarts once during that run
