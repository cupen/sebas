## ADDED Requirements

### Requirement: Decision vocabulary has a single shared definition

The cross-driver permission decision set (`allow_once` / `allow_session` / `deny` / `escalate`) SHALL be carried by exactly one shared definition. Each layer that previously declared its own decision type — the ACP driver, the web UI session backend, the native bridge, the agent policy layer, and the node link — SHALL use that single definition instead of a parallel declaration, converting only where a boundary's accepted subset genuinely differs. Every boundary SHALL preserve the exact serialized spelling it used before, and SHALL preserve the subset of values it actually sends today; unifying the type SHALL NOT cause any boundary to start sending a value it did not send before. An unrecognized decision value received from a peer SHALL be tolerated as an unknown decision rather than failing the message. The `escalate` downgrade for execution bodies without an escalate equivalent (delivered as `allow_once`, with the downgrade logged) SHALL remain the only semantic adaptation between layers.

#### Scenario: One decision type, not five

- **WHEN** the workspace is searched for types representing a permission decision
- **THEN** exactly one shared definition exists
- **AND** the ACP driver, web UI session backend, native bridge, agent policy layer, and node link all use it
- **AND** no hand-written conversion between parallel decision enums remains

#### Scenario: An existing boundary sends the same values as before

- **WHEN** the control plane answers a permission request owned by an execution body that accepts only a subset of the decision set
- **THEN** the values sent on that boundary are exactly the set sent before the vocabulary was unified
- **AND** an `escalate` decision is delivered as `allow_once` with the downgrade logged

#### Scenario: Serialized spellings are unchanged

- **WHEN** any decision is serialized on the core channel, the node link, or the web UI surface
- **THEN** its serialized spelling is byte-identical to the pre-unification spelling

#### Scenario: An unknown decision is tolerated

- **WHEN** a peer sends a decision value this build does not know
- **THEN** the message is accepted and the unknown decision is surfaced
- **AND** no parked approval is silently resolved by a decision that could not be interpreted
