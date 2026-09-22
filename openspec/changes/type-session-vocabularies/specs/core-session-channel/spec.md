## ADDED Requirements

### Requirement: Turn content vocabulary is closed and forward-tolerant

The turn-content vocabulary carried over the session channel — the turn entry kind and its element type — SHALL be carried by a shared closed set of values with fixed spellings rather than by free-form strings matched independently by each consumer. A consumer that renders turn content SHALL match on the shared type, so that adding a value is a compile-time change rather than a silent fallthrough. A turn entry carrying an element type this build does not know SHALL still be delivered and rendered as a generic block; it SHALL NOT be dropped, truncated, or turned into an error.

#### Scenario: Rendered content keeps its exact spelling

- **WHEN** turn entries already present in a session's transcript are re-read after this vocabulary is typed
- **THEN** every entry's element type and kind deserialize to the same value as before
- **AND** the rendered output for each entry is unchanged

#### Scenario: An unknown element type renders as a generic block

- **WHEN** a turn entry arrives whose element type this build does not know
- **THEN** the entry is still delivered to the channel surface
- **AND** it is rendered as a generic content block rather than being dropped or shown as an error

#### Scenario: Adding an element type cannot silently miss a renderer

- **WHEN** a new element type is added to the shared set
- **THEN** each consumer that branches on element type fails to compile until it handles the new value
- **AND** no consumer relies on a string comparison that would silently render the default block
