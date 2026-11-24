# state-store delta — agent-task-center

## ADDED Requirements

### Requirement: Session map carries an optional task_id

The `session_map` table SHALL carry an optional `task_id TEXT` column holding the beads issue id (e.g. `bd-42`) of the task this session is advancing. The column SHALL be nullable; sessions not associated with any task SHALL store NULL. The column SHALL be added via the standard `sebas_db::schema` date-stamped migration. The core SHALL accept `task_id` from the WebUI at session-creation time and persist it verbatim; the core SHALL NOT validate that the named beads issue exists, because beads is outside the core's domain — task validity is the WebUI's responsibility, enforced at the HTTP API boundary before the create-session call reaches the core.

#### Scenario: Session created with a task link

- **WHEN** the WebUI creates a session and supplies `task_id = "bd-42"`
- **THEN** the persisted `session_map` row stores `task_id = "bd-42"` and subsequent reads return that value

#### Scenario: Session created without a task link

- **WHEN** the WebUI creates a session without supplying `task_id`
- **THEN** the persisted `session_map` row stores NULL in the `task_id` column

#### Scenario: Migration adds the column without dropping existing rows

- **WHEN** the schema migration runs against a database that already has `session_map` rows
- **THEN** `ALTER TABLE session_map ADD COLUMN task_id TEXT` is applied, existing rows keep their data, and `task_id` is NULL for all of them

#### Scenario: Core does not validate task existence

- **WHEN** the WebUI creates a session with `task_id = "bd-nonexistent"`
- **THEN** the core persists the value verbatim and the session is created; the WebUI is responsible for refusing such a request before it reaches the core
