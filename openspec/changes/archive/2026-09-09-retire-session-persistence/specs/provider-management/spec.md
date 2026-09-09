## ADDED Requirements

### Requirement: Default selection wire compatibility and atomic delete

The default selection SHALL comprise a provider name and an optional model; the model SHALL be stored as absent when unset. The channel wire format for the default selection SHALL accept both the object form and the legacy bare-string form. Deleting a provider SHALL atomically (in one transaction) remove its entry, record the deletion, and clear a default selection that names it; mode cleanup is applied by the load-time repair step. The selection SHALL persist across restarts through the state store (SQLite) — the legacy per-file JSON provider store is retired.

#### Scenario: No model omits the field

- **WHEN** the default selection has a provider but no model
- **THEN** the stored state has no model value, and the wire form omits the model field

#### Scenario: Deleting the default provider clears the selection atomically

- **WHEN** the user deletes the provider that the default selection names
- **THEN** a single transaction removes the entry, records the deletion, and clears the selection

#### Scenario: Default survives restart via state store

- **WHEN** the operator sets a default provider and model and the daemon restarts
- **THEN** the default selection is read back from the state store, not from a legacy JSON file
