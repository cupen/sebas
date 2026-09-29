## MODIFIED Requirements

### Requirement: Control RPC transport and authentication

The control plane SHALL speak JSON-Lines over a Unix domain socket at
`<SEBAS_HOME>/run/control.sock` (the sebas home's run directory), mode
0600, one task per accepted stream. Every envelope SHALL carry `version`
(must be 1), a `secret` matching the watchdog's per-instance startup
secret, and an `actor` of either `Cli { uid }` or `Feishu { open_id,
chat_id }` — a `System` actor cannot be forged on the wire. Wrong or missing
secret → `unauthorized`; wrong version → `unsupported_version`. The secret
is generated at startup (`{pid}-{timestamp}`) and never persisted, so
restarting the watchdog invalidates outstanding clients.

#### Scenario: wrong secret rejected

- **WHEN** a request envelope carries a secret that does not match the
  watchdog's instance secret
- **THEN** the response is `Rejected { code: "unauthorized" }`

#### Scenario: system actor unforgeable

- **WHEN** a client sends an envelope whose actor field claims `system`
- **THEN** deserialization rejects it before any handler runs

#### Scenario: endpoint follows the sebas home

- **WHEN** `SEBAS_HOME` is pinned and the watchdog starts with no explicit
  socket override
- **THEN** the control socket is created at `<SEBAS_HOME>/run/control.sock`
- **AND** no path under `XDG_RUNTIME_DIR` or a per-uid temporary directory
  is consulted
