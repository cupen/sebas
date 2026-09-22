## ADDED Requirements

### Requirement: Legacy JSON state files are retired

The state store SHALL be the only authority for provider state and for runtime state. The legacy state file and the legacy provider overlay file SHALL NOT be written, SHALL NOT be read, and SHALL NOT be imported: a machine that carries them SHALL start from the state store's contents, and the files SHALL be left on disk untouched for the operator to remove. The environment variables that pointed at those files SHALL be retired rather than silently honored, so that configuring them cannot create the false impression that they take effect. Reading clients that previously fell back to a file SHALL obtain the same data through the state methods, and SHALL present the existing unavailable state rather than a file-derived value when the store cannot be reached.

#### Scenario: no legacy file is written

- **WHEN** provider or runtime state is mutated through any surface
- **THEN** no legacy state file or provider overlay file is created or modified
- **AND** the state store reflects the mutation

#### Scenario: a machine carrying legacy files does not import them

- **WHEN** the process starts on a machine whose legacy provider overlay holds provider entries and the state store holds none
- **THEN** the state store stays empty
- **AND** the overlay file is left untouched and providers are re-created through the supported surfaces before they route

#### Scenario: reading clients no longer fall back to a file

- **WHEN** a client that previously read a legacy file needs the data and the store is unreachable
- **THEN** it reports the store-unavailable state naming the cause
- **AND** it does not read a file-derived value or present one as current

#### Scenario: retired environment variables have no effect

- **WHEN** the retired state-file or provider-overlay environment variables are set
- **THEN** they do not change where state is read from or written to
- **AND** the process behaves exactly as if they were unset
