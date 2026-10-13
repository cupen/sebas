## ADDED Requirements

### Requirement: In detached topology, the webui's agent catalog reflects the core's execution-body report

In the watchdog-managed (detached) topology, where `sebas run` spawns `sebas core` and `sebas webui` as separate processes, the standalone webui's `GET /api/agents` response SHALL report the built-in `native` row's `reachable` and `cause` by resolving the core session channel's execution-bodies payload — not by returning the trait default (`None`, which would make every execution body appear unreachable). The webui's `CoreChannelBackend` SHALL carry the bodies snapshot from the core channel (via `StateSnapshot { domain: "agents" }` carrying an `execution_bodies` field) and SHALL override `execution_bodies()` to return that snapshot; the resulting `reachable` and `cause` for the `native` row in the webui's `GET /api/agents` SHALL match the core's current report for the `native` execution body.

#### Scenario: native reachability matches the core in detached topology

- **WHEN** the watchdog-managed topology runs with the standalone webui process, and the core's native execution body is reachable (pi binary present and provider configured)
- **THEN** the standalone webui's `GET /api/agents` reports the native row as `reachable: true` (not `false` by trait default), and the cause is the core's reported advisory (e.g. auth-ready advisory or null)

#### Scenario: native unavailability matches the core in detached topology

- **WHEN** the watchdog-managed topology runs and the core's native execution body is unavailable (binary missing or provider not configured)
- **THEN** the standalone webui's `GET /api/agents` reports the native row as `reachable: false` with the `cause` string the core reported (e.g. "未配 provider，请到 Settings → Models 配置" or "pi 二进制不可达 — 到 Settings → Agent 一键安装 pi"), not a hardcoded fixed literal that contradicts the core's report
