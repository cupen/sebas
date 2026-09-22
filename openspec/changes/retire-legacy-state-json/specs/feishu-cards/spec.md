## MODIFIED Requirements

### Requirement: Card theme configuration

The card theme color (`card.theme_color`, default `blue`) SHALL flow into the card header template. Card settings SHALL be parsed with strict (deny-unknown-fields) semantics — an unknown key in `[card]` is a configuration error rather than a silent ignore — and SHALL be persisted as a full config snapshot in the core state store, which SHALL be the only authority for card settings. Card settings SHALL NOT be persisted to a JSON file of their own. The state store file that carries the snapshot SHALL be readable and writable only by its owner, and the snapshot SHALL be replaced atomically as a whole so a reader never observes a partially written configuration.

#### Scenario: default theme

- **WHEN** no `[card]` section is configured
- **THEN** card headers render with the blue template

#### Scenario: unknown card key rejected

- **WHEN** the config file contains `[card]` with key `theme_colr`
- **THEN** configuration parsing fails with an unknown-field error

#### Scenario: card settings survive restart without a settings file

- **WHEN** an operator changes the card theme and the process restarts
- **THEN** the changed theme still applies
- **AND** no card-settings JSON file exists on disk
