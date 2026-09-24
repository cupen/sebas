# session-persistence Specification

## Purpose
Owns the persisted provider-state store: schema versioning and migrations from v1, corruption tolerance, atomic write mechanics, mode/selection repair rules, and exactly which runtime state is deliberately not persisted. The state store is the only authority for provider state and for runtime state; the legacy `state.json` and `providers.json` files are retired — not written, not read, not imported — and the environment variables that pointed at them are retired too, so the store holds no on-disk layout contract for them. The agent session map is no longer held here: it is persisted in the core state store (SQLite), written per mutation rather than at shutdown, and no separate session-map file is written or read.

## Requirements

### Requirement: Default selection semantics

The default selection SHALL comprise a provider name and an optional model; the model SHALL be stored as absent when unset. The channel wire format for the default selection SHALL accept both the object form and the legacy bare-string form. Deleting a provider SHALL atomically (in one transaction) remove its entry, record the deletion, and clear a default selection that names it; mode cleanup is applied by the load-time repair step.

#### Scenario: No model omits the field

- **WHEN** the default selection has a provider but no model
- **THEN** the stored state has no model value, and the wire form omits the model field

#### Scenario: Deleting the default provider clears the selection atomically

- **WHEN** the user deletes the provider that the default selection names
- **THEN** a single transaction removes the entry, records the deletion, and clears the selection

### Requirement: Runtime state is not persisted by this store

The store SHALL NOT persist the permission allowlist, outstanding permission cards, card states, or in-flight spawn placeholders. The agent session map SHALL be dumped to the dispatch state file on the core's graceful-shutdown path (after the children are stopped, before exit) and reloaded at startup; per-mutation durability of the session map in the SQLite state store is a deferred migration (state-store keeps `session_map` as a reserved placeholder table until then). An unclean exit therefore loses only mutations since the last graceful shutdown, and the loaded map SHALL be tolerated as possibly stale.

#### Scenario: Allowlist survives no restart

- **WHEN** the daemon restarts
- **THEN** previously granted session-scoped permissions are gone and the user is prompted again

#### Scenario: Spawn placeholders are never written

- **WHEN** the daemon shuts down while a spawn is in flight
- **THEN** the persisted state contains no trace of the in-flight spawn

#### Scenario: Session map survives graceful shutdown only

- **WHEN** core is stopped gracefully (SIGTERM) while sessions exist
- **THEN** the state file is dumped before exit and the session map at next start reflects that last graceful shutdown; after an unclean kill the map is the last dump and may be stale

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
