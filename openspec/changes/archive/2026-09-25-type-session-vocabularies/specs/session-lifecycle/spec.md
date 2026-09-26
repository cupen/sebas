## ADDED Requirements

### Requirement: Session state vocabulary is shared and closed

The session state vocabulary — the session phase reported by the control plane and by an execution node, and the session mode (`desired` / `effective`) — SHALL be carried by a single shared definition rather than by ad-hoc strings redeclared per layer. The set of accepted values SHALL be closed and their spellings SHALL be fixed: the control plane and the execution node SHALL agree on one value set even though each emits only the subset valid for its own role. An unrecognized value received from a peer SHALL NOT fail deserialization, drop the message, or close the connection; it SHALL be carried as an explicitly-marked unknown value. The user-facing status shown by each channel surface SHALL be derived from the phase by a type-checked mapping, never by matching on string literals.

#### Scenario: Both sides share one value set with fixed spellings

- **WHEN** the control plane reports a session phase and an execution node reports the same session's phase
- **THEN** both values come from the same shared definition
- **AND** every value retains the exact spelling it had before this vocabulary was typed (including hyphenated forms such as the spawn-failure phase)

#### Scenario: An unknown phase from a newer peer is tolerated

- **WHEN** a peer sends a phase value this build does not know
- **THEN** the message is still accepted and the session remains observable
- **AND** the value is surfaced as an unknown phase rather than causing a deserialization failure or a dropped frame

#### Scenario: Adding a phase value cannot silently miss a handler

- **WHEN** a new phase value is added to the shared definition
- **THEN** every consumer that branches on the phase fails to compile until it handles the new value
- **AND** no consumer relies on a string comparison that would silently take a default branch instead

#### Scenario: Display status is derived, not string-matched

- **WHEN** a channel surface renders a session's status for the user
- **THEN** the status is produced by mapping the shared phase value to the presentation status
- **AND** a phase whose presentation mapping is missing is a compile-time omission, not a silently wrong label

#### Scenario: Session mode crosses every layer as one type

- **WHEN** a session's desired or effective mode is read, set, or forwarded through the channel, the node link, or the web UI request surface
- **THEN** it is carried as the shared mode type rather than as a free-form string
- **AND** an unrecognized mode from a peer is tolerated as an unknown mode instead of being rejected
