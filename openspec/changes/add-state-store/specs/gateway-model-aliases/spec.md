## MODIFIED Requirements

### Requirement: Alias entity and persistence

A model alias SHALL be persisted in the core state store as a row with fields: `alias` (the custom name clients use), `provider` (the bound provider name), and optional `upstream_model` (the real model name forwarded upstream). Aliases survive restarts, are read and written through the core channel state methods (in-process within core), and coexist with other stored data without affecting it.

#### Scenario: alias persists across restart

- **WHEN** an alias `my-claude` is created and the gateway process restarts
- **THEN** requests for model `my-claude` still route to the bound provider

### Requirement: Alias validation

Alias writes MUST be validated before persistence: the alias is non-empty, does not contain `/` (reserved for the namespace syntax), and references an existing provider. Invalid alias writes SHALL be rejected at write time by the state store. Invalid rows that predate a validation change SHALL be dropped at load time with a warning; the remaining aliases still apply (partial self-heal, never a startup failure).

#### Scenario: invalid alias rejected at write

- **WHEN** a client submits an alias containing `/` via the state methods
- **THEN** the store rejects the mutation with a validation error and nothing is persisted

#### Scenario: externally broken alias dropped with warning

- **WHEN** the store contains an alias row referencing a non-existent
  provider (e.g. written before a validation rule tightened, or by a manual
  database edit)
- **THEN** the gateway starts with that alias dropped, logs a warning naming
  the alias, and other aliases remain effective
