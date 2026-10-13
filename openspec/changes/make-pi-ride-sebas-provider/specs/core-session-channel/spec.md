## ADDED Requirements

### Requirement: StateSnapshot exposes execution_bodies for the agents domain

The core session channel's `StateSnapshot { domain: "agents" }` response payload SHALL include an `execution_bodies` field — a map from execution-body name (`"native"`, future `"acp"`) to a status object `{ ok: bool, cause: Option<String> }`. The server SHALL populate this field on every `StateSnapshot` response for the `agents` domain by reading the current in-process `DualSessionBackend::execution_bodies()` report; the standalone webui (detached topology) SHALL consume this field to render accurate `reachable` and `cause` for the built-in `native` row in `GET /api/agents` rather than falling back to the trait default (`None`, which would make every execution body appear unreachable). The field SHALL also be present in the `Changed { scope: "agents" }` notification payload-equivalent for clients that re-fetch on the subscription stream, so a reachability change in the core is reflected on the next `StateSnapshot` call.

#### Scenario: StateSnapshot.agents includes execution_bodies

- **WHEN** any client (in-process webui or standalone webui over the channel) sends `StateSnapshot { domain: "agents" }` to the core
- **THEN** the response payload includes `execution_bodies.native = { ok: <bool>, cause: <Option<String>> }` reflecting the core's current native execution-body status, and an entry for any other registered execution body

#### Scenario: Reachability change is observable on next snapshot

- **WHEN** the core's native execution-body status changes (e.g. pi binary appears or the operator configures a provider in Settings → Models) and a client subsequently sends `StateSnapshot { domain: "agents" }`
- **THEN** the response payload's `execution_bodies.native` reflects the new status (ok and cause); clients SHALL re-fetch this snapshot on a `Changed { scope: "agents" }` subscription notification
