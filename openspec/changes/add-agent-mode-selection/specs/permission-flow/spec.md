## MODIFIED Requirements

### Requirement: Session mode gates whether a decision is requested

Each session SHALL carry a mode (`ask`, `edit`, `allow`, `auto`, and any further values the control plane defines) that decides whether a tool action needs a decision at all. Under `auto` the session SHALL run without producing permission requests. The mode SHALL be a desired value held by the control plane, and the execution side SHALL report the mode it actually enforces. An execution body that cannot enforce a mode SHALL report that fact rather than appearing to enforce it. `auto` SHALL NOT be the default mode and SHALL leave an audit trail when selected.

The mode SHALL be selectable at session creation time from the control-plane surfaces (web session create and mid-session mode switch), not only assigned by node-side defaults. On the local (in-process) claude execution path, the control-plane mode SHALL map onto the claude CLI's permission-mode vocabulary by convention — `ask` → CLI default (no flag), `edit` → acceptEdits, `allow`/`auto` → bypassPermissions — applied as a spawn-time flag and switchable at runtime; an execution body that receives a mode it cannot apply SHALL NOT fail the session (non-fatal, same posture as model selection). The node-link path SHALL carry the control-plane mode verbatim as its existing gate vocabulary.

#### Scenario: auto runs without prompting

- **WHEN** a session's mode is `auto` and its agent invokes a tool that would otherwise be gated
- **THEN** no permission request is produced and the tool proceeds

#### Scenario: desired and effective mode can differ

- **WHEN** the control plane sets a mode an execution body cannot enforce
- **THEN** the reported effective mode states what is actually enforced, and the difference is visible to the operator

#### Scenario: auto is an explicit choice

- **WHEN** a session is created without an explicit mode
- **THEN** it does not default to `auto`

#### Scenario: local claude session honors allow at creation

- **WHEN** a local claude session is created with mode `allow` and its agent invokes a gated tool
- **THEN** the tool proceeds without a permission request (bypassPermissions applied at spawn)

#### Scenario: local claude mid-session switch to edit relaxes edit gating

- **WHEN** a running local claude session is switched from `ask` to `edit` and then invokes a file-edit tool
- **THEN** the edit proceeds without a permission request while other gated categories still ask

#### Scenario: unknown mode is rejected, not degraded

- **WHEN** a create or switch request carries a mode outside the vocabulary
- **THEN** the request is rejected with an explicit error and the session's mode is unchanged
