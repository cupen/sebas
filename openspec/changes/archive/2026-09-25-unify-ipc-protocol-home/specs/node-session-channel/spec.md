## ADDED Requirements

### Requirement: Link contract is pinned by fixtures

The node link's existing version and capability negotiation SHALL be backed by a checked-in contract fixture set, so that the link's wire shapes are held by a mechanical gate rather than by the discipline of the crate that owns them. The fixtures SHALL pin each link message's serialized shape and its set of field names.

#### Scenario: A link message change must update the fixture

- **WHEN** a link message gains, loses, or renames a field
- **THEN** the link's contract fixture test fails until the fixture is updated deliberately
- **AND** the change record states whether the alteration is compatible or breaking

#### Scenario: Version negotiation is covered by the same gate

- **WHEN** the handshake or its version negotiation changes
- **THEN** the fixture set covering the handshake is updated in the same change
- **AND** the existing exact-version comparison and the rejection naming both versions remain in force

#### Scenario: The link keeps one shared definition

- **WHEN** the control plane and the node are built
- **THEN** both obtain the link's message types from the single shared contract crate
- **AND** neither side carries a redeclared copy
