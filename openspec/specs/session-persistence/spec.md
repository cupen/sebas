# session-persistence Specification

## Purpose
Owns the persisted provider-state store: the on-disk layout of `~/.sebas/state.json` and the legacy `~/.sebas/providers.json` overlay, schema versioning and migrations from v1, corruption tolerance, atomic write mechanics, mode/selection repair rules, and exactly which runtime state is deliberately not persisted. The persistence responsibility is being migrated to the core state store (SQLite).

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

The store SHALL NOT persist the permission allowlist, outstanding permission cards, card states, or in-flight spawn placeholders. The agent session map SHALL be persisted in the state store and written per mutation, rather than only at daemon shutdown.

#### Scenario: Allowlist survives no restart

- **WHEN** the daemon restarts
- **THEN** previously granted session-scoped permissions are gone and the user is prompted again

#### Scenario: Spawn placeholders are never written

- **WHEN** the daemon shuts down while a spawn is in flight
- **THEN** the persisted state contains no trace of the in-flight spawn

#### Scenario: Session map survives unclean exit

- **WHEN** core is killed while sessions are active
- **THEN** the session map at next start reflects the last committed state, not the last graceful shutdown
