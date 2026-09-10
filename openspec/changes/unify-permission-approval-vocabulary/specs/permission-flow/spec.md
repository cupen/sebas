## MODIFIED Requirements

### Requirement: Three decision outcomes

The system SHALL support three user decisions on a Feishu permission card: `Allow once`, `Allow session`, and `Deny`. This capability owns the Feishu-side rendering and the per-chat allowlist for the hook-driven path; the cross-driver decision vocabulary (including `escalate`) and `request_id` namespacing are governed by `agent-driver`. Each decision maps to a distinct hook output and a distinct post-click card state.

#### Scenario: Allow once approves this call only

- **WHEN** the user clicks `Allow once`
- **THEN** the hook callback returns `permissionDecision: allow` for this `request_id`
- **AND** the `(tool, args)` signature is NOT added to the allowlist
- **AND** the card flips in place to a resolved "已允许（仅本次）" state

#### Scenario: Allow session approves and remembers

- **WHEN** the user clicks `Allow session`
- **THEN** the hook callback returns `permissionDecision: allow`
- **AND** the exact `(tool, args)` signature is added to the per-chat allowlist
- **AND** the card flips in place to a resolved "已允许（本会话）" state

#### Scenario: Deny rejects the call

- **WHEN** the user clicks `Deny`
- **THEN** the hook callback returns `permissionDecision: deny`
- **AND** the allowlist is not modified
- **AND** the card flips in place to a resolved "已拒绝" state
