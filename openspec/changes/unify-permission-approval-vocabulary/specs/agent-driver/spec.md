## MODIFIED Requirements

### Requirement: Cross-driver permission routing through the webui review card

The system SHALL route permission requests from every driver through the same downstream channel, so a permission request raised by either the Claude driver or the ACP driver SHALL be addressable through the webui review card. The full decision vocabulary is `allow_once` / `allow_session` / `deny` / `escalate`. The `escalate` decision (a one-shot allow carrying the operator's reason) is meaningful only for the native kernel; when the owning execution body is an ACP driver, an `escalate` decision SHALL be delivered as `allow_once` and the downgrade SHALL be logged. The system SHALL name, in the `PermissionRequest` the driver emits, the `request_id` as `<kind-slug>:<raw-id>` so ids from different drivers cannot collide, and SHALL decode it back to the raw id when delivering the answer to the owning driver.

#### Scenario: Permission round-trip works for an ACP agent

- **WHEN** a native-ACP agent raises a permission request for a tool the policy gates
- **THEN** the webui shows the review card
- **AND** the chosen decision is delivered to the ACP driver, which answers the ACP permission with the mapped `PermissionOption.kind`
- **AND** the request id carries the kind slug so it is unambiguous across sessions

#### Scenario: escalate falls back to allow-once for an ACP agent

- **WHEN** the operator answers `escalate` on a permission request whose owning execution body is an ACP driver
- **THEN** the decision delivered to that ACP driver is `allow_once` (ACP has no escalate equivalent)
- **AND** the downgrade is logged

#### Scenario: Claude permission reaches the webui (gap fix)

- **WHEN** a Claude Code session raises a `PermissionRequest`
- **THEN** the `InProcessBackend` (the ACP-path session backend) forwards it to the webui as a `PermissionNotice`
- **AND** `answer_permission` on that backend delivers the decision back to the session, instead of returning the trait default `false`
