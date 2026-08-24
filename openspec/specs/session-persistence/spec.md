# session-persistence Specification

## Purpose
Owns the persisted provider-state store: the on-disk layout of `~/.sebas/state.json` and the legacy `~/.sebas/providers.json` overlay, schema versioning and migrations from v1, corruption tolerance, atomic write mechanics, mode/selection repair rules, and exactly which runtime state is deliberately not persisted.

## Requirements

### Requirement: State file layout and overrides

The persisted state SHALL live in `~/.sebas/state.json` (overridable via environment variable) with the legacy overlay at `~/.sebas/providers.json` (independently overridable). Paths SHALL expand a leading `~/`. The state file SHALL contain: a schema version, provider entries created or modified through the UI (a delta over the seeded configuration), a tombstone list of deleted provider names, the provider mode, and the default selection for direct mode.

#### Scenario: Environment override relocates the state file

- **WHEN** the state-file environment variable points to a custom path
- **THEN** all reads and writes of provider state use that path

#### Scenario: Tilde paths expand to home

- **WHEN** a configured path begins with `~/`
- **THEN** the path is expanded to the user's home directory before use

### Requirement: Current schema version

The store SHALL write schema version 2 on every save, including the initial default state. Version 2 SHALL be accepted directly on load.

#### Scenario: Fresh save writes version 2

- **WHEN** the store saves state that has never been loaded from disk
- **THEN** the written file declares version 2

### Requirement: Migration from v1

A version-1 (or version-0) state file SHALL be migrated to version 2 on load: the mode and default selection are preserved; provider entries and tombstones are taken from the legacy overlay when it exists, and the overlay is deleted after a successful migration. Without an overlay, providers start empty.

#### Scenario: v1 state with overlay migrates and removes overlay

- **WHEN** a v1 state file coexists with a legacy overlay file
- **THEN** load merges both into a v2 state, writes it, and deletes the overlay file

#### Scenario: v1 state without overlay preserves mode

- **WHEN** a v1 state file exists with no overlay
- **THEN** load migrates it to v2 preserving the mode and default selection, with an empty provider set

### Requirement: One-time overlay reconciliation

A version-2 state file with a legacy overlay still present SHALL be reconciled on load: existing v2 provider entries win over overlay entries with the same name; tombstone lists are merged and deduplicated; the overlay is then deleted so it is never read again.

#### Scenario: Half-migrated state prefers v2 entries

- **WHEN** both a v2 state file and an overlay exist with conflicting entries for the same provider name
- **THEN** the v2 entry is kept, the overlay's other entries are merged in, and the overlay file is removed

### Requirement: Legacy field upgrade within v2

A v2 file carrying the legacy default-provider field SHALL be upgraded in memory to the current default-selection shape (provider set, model unset). If a file carries both the legacy field and the current field with conflicting values, the whole file SHALL be treated as corrupt and the default state used.

#### Scenario: Legacy field upgrades to default selection

- **WHEN** a v2 file contains the legacy `default_provider_for_direct` field
- **THEN** loading yields a default selection with that provider and no model

#### Scenario: Conflicting legacy and current fields fall back to default

- **WHEN** a v2 file contains both the legacy field and the current default selection naming different providers
- **THEN** the file is treated as corrupt and the default (empty) state is used

### Requirement: Corruption tolerance

An unreadable, corrupt, or unknown-version state file SHALL never crash the daemon: the system SHALL log a warning and fall back to the default state. A missing state file is not an error. A broken legacy overlay on the migration path SHALL be backed up and ignored.

#### Scenario: Corrupt JSON falls back to default

- **WHEN** the state file contains invalid JSON
- **THEN** a warning is logged and the default state is used

#### Scenario: Unknown version falls back to default

- **WHEN** the state file declares an unrecognized schema version
- **THEN** a warning is logged and the default state is used

### Requirement: Atomic write per mutation

Every mutation SHALL persist immediately via an atomic write: content is written to a temporary sibling file, flushed to disk, then renamed over the target; missing parent directories are created on save. There is no batching, debounce, or shutdown-only flush. Load operations that perform migration are also permitted to write.

#### Scenario: Mutation persists immediately

- **WHEN** a provider entry is created, modified, or deleted
- **THEN** the state file on disk reflects the change before the operation returns

#### Scenario: Interrupted write leaves prior state intact

- **WHEN** a save is interrupted before the rename
- **THEN** the previous state file remains valid

### Requirement: Mode repair on load

On every version-2 load, the store SHALL repair a stale direct-mode pointer: if the mode names a provider that is in the tombstone list, the mode SHALL be reset to off and a default selection naming that provider SHALL be cleared. A mode pointer to a provider merely missing from the entries (no tombstone) SHALL be preserved — that is a legal pre-configuration state.

#### Scenario: Stale direct pointer is cleared

- **WHEN** the loaded state has direct mode pointing at a deleted provider
- **THEN** the mode becomes off and the default selection naming that provider is cleared

#### Scenario: Pointer to missing provider is kept

- **WHEN** the loaded state has direct mode pointing at a provider with no entry and no tombstone
- **THEN** the mode is left unchanged

### Requirement: Default selection semantics

The default selection SHALL comprise a provider name and an optional model. The model field SHALL be omitted from the persisted JSON when unset. Deserialization SHALL accept both the object form and the legacy bare-string form, and SHALL be insensitive to field order. Deleting a provider SHALL atomically (in one write) remove its entry, append its tombstone, and clear a default selection that names it; mode cleanup is deferred to the next load's repair step.

#### Scenario: No model omits the field

- **WHEN** the default selection has a provider but no model
- **THEN** the persisted JSON contains no model field

#### Scenario: Deleting the default provider clears the selection atomically

- **WHEN** the user deletes the provider that the default selection names
- **THEN** a single write removes the entry, records the tombstone, and clears the selection

### Requirement: Runtime state is not persisted by this store

The store SHALL NOT persist the permission allowlist, outstanding permission cards, card states, or in-flight spawn placeholders. The agent session map is persisted by a separate mechanism at daemon shutdown and is not part of this store's file.

#### Scenario: Allowlist survives no restart

- **WHEN** the daemon restarts
- **THEN** previously granted session-scoped permissions are gone and the user is prompted again

#### Scenario: Spawn placeholders are never written

- **WHEN** the daemon shuts down while a spawn is in flight
- **THEN** the persisted state contains no trace of the in-flight spawn
