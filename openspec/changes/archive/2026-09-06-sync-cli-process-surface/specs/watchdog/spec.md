## MODIFIED Requirements

### Requirement: Control request surface

The RPC SHALL serve: `Status`, `EventsSince`, `Update`, `Rollback`,
`RestartCore`, `ServiceStatus`, `ServiceStatusFor`, `ServiceSet`,
`ServiceRestart`, `Confirm`, and `Cancel`. `ServiceSet` and
`ServiceRestart` SHALL act on the auxiliary managed services (webui,
router, im) as specified in the Service lifecycle requirement; requests
naming the core service SHALL be rejected with an actionable error.
`Confirm` and `Cancel` SHALL be accepted only from a Feishu actor with a
`chat_id`; any other actor gets `unauthorized`.

#### Scenario: service set accepted

- **WHEN** a client sends `ServiceSet { service: "webui", desired: "off" }`
- **THEN** the response is `Accepted` and the WebUI child stops

#### Scenario: service set rejected

- **WHEN** a client sends `ServiceSet { service: "core", desired: "off" }`
- **THEN** the response is `Rejected` with an actionable message pointing
  to `RestartCore` (core lifecycle is supervised, not user-toggled)

#### Scenario: cli cannot confirm

- **WHEN** a `Cli` actor sends `Confirm { token }`
- **THEN** the response is `Rejected { code: "unauthorized" }`
