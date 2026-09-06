# dispatch-commands Specification（delta）

## MODIFIED Requirements

### Requirement: Control commands forwarded to the watchdog

`/upgrade [dev] [--dry-run]`, `/rollback`, `/restart`, `/services`, `/system`, `/router <action>`, and `/webui status` SHALL be translated into watchdog control requests issued by the IM service itself over the control RPC socket (no longer relayed through the core), and their results returned as plain text messages — no session is required. When the control credential is not configured in the IM service, the system SHALL reply with a plain-text notice explaining the missing control plane instead of issuing the request. A watchdog communication failure or rejection SHALL surface as a plain-text failure message.

#### Scenario: /system with no session

- **WHEN** the user sends `/system` with no session mapped
- **THEN** a watchdog status request is issued by the IM service and the result is sent as plain text

#### Scenario: Missing control credential

- **WHEN** a control command is issued while the IM service holds no control secret
- **THEN** the user receives a plain-text notice that the control plane is unavailable and how to enable it

#### Scenario: Watchdog offline

- **WHEN** a control command is issued and the watchdog control RPC call fails
- **THEN** the user receives a plain-text failure message naming the command and the error
