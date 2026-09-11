## MODIFIED Requirements

### Requirement: Session placement is a function of the project

A session SHALL be placed on the node named by its project reference. A session with no project SHALL be placed on the configured default execution node. When the target node is offline, the session request SHALL fail with a cause naming the node and its state; the request SHALL NOT be queued for later delivery. The placement request SHALL carry the session's creation-time desired mode, and mid-session mode changes SHALL travel to the node over the existing SetMode primitive; the node's reported effective mode remains the node's fact, not the control plane's wish.

#### Scenario: project decides the node

- **WHEN** the operator starts a session from a project registered against a node
- **THEN** the session is placed on that node

#### Scenario: project-less session uses the default node

- **WHEN** a session originates without a project directory
- **THEN** it is placed on the configured default execution node and its origin remains visible

#### Scenario: offline node fails honestly

- **WHEN** a session is requested for a project whose node is offline
- **THEN** the request fails with a cause naming the node and its state
- **AND** no placeholder session is created that implies the work has begun

#### Scenario: creation-time mode crosses the link

- **WHEN** a session is placed with a creation request carrying `mode`
- **THEN** the node spawn op carries that mode and the node applies it per its own gate semantics, reporting effective mode back

#### Scenario: mid-session mode switch crosses the link

- **WHEN** the control plane switches a placed session's mode mid-session
- **THEN** the SetMode op arrives at the node, the node updates its gate behavior and reports the outcome, and the control-plane projection reflects the reported mode
