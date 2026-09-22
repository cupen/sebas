## MODIFIED Requirements

### Requirement: Archive persistence

The archive registry SHALL persist to its own file, separate from the project registry and the router state file. Each entry SHALL record the session key, the original project path, the session label, the archive timestamp, and the retention deadline.

The archive file's location SHALL resolve in the following priority order: an explicit `SEBAS_ARCHIVE_PATH` override; otherwise `archive.json` in the state database's directory (the directory of `SEBAS_STATE_DB`), so that deployments which pin the state directory — sandboxes in particular — pin the archive with it. The legacy default (`archive.json` directly under the home `.sebas` directory) SHALL be honored as a migration source only: when the resolved location holds no archive file and the legacy location does, the WebUI SHALL move the legacy file to the resolved location at startup; when the move fails, the WebUI SHALL warn and continue reading from the legacy location rather than silently starting empty. A migration SHALL be announced through the WebUI's notification channel.

#### Scenario: archive survives restart

- **WHEN** the WebUI process is restarted after sessions were archived
- **THEN** the same archived sessions are listed

#### Scenario: archive expiry clean on startup

- **WHEN** the WebUI starts and an archived session has passed its retention deadline
- **THEN** that entry is removed from the archive file and the session is no longer listed

#### Scenario: pinned state directory pins the archive

- **WHEN** the WebUI runs with `SEBAS_STATE_DB` pointing inside a sandbox directory and no `SEBAS_ARCHIVE_PATH` is set
- **THEN** the archive file is read and written inside that same sandbox directory, and no path under the real home directory is read or written

#### Scenario: explicit override wins

- **WHEN** `SEBAS_ARCHIVE_PATH` is set
- **THEN** that exact file is used regardless of the state database location

#### Scenario: legacy archive migrates forward

- **WHEN** the WebUI starts with no archive file at the resolved location and a legacy archive file exists under the home `.sebas` directory
- **THEN** the legacy file is moved to the resolved location, its entries are listed as before, and a notification announces the migration

#### Scenario: failed migration degrades read-only rather than empty

- **WHEN** the legacy move fails at startup
- **THEN** the WebUI warns, continues to serve the legacy entries, and does not start an empty archive over them

## ADDED Requirements

### Requirement: Focused session termination is reflected consistently

When a session the operator is focused on is terminated and removed by the
backend — child process crash, dispatch reaping, or any other removal — the
focused view SHALL leave the Working state in the same update cycle: it SHALL
NOT continue rendering a live session with an active stop control once the
backend no longer knows the session. The termination SHALL be announced
through the notification channel with the session label and the observed
cause (for a child crash, at minimum that the agent process exited
unexpectedly), and the transcript already received SHALL remain viewable. The
rail, the focused view, and the backend session list SHALL agree on the
session's existence.

#### Scenario: child crash ends the Working state

- **WHEN** the focused session's agent child crashes after emitting partial
  output and the backend removes the session
- **THEN** the focused view stops presenting Working and its stop control, a
  notification names the crashed session, the received transcript stays
  readable, and the rail no longer lists the session

#### Scenario: rail and focused view agree with the backend

- **WHEN** a session is removed by the backend for any reason
- **THEN** within the same update cycle the rail drops the row and a focused
  view of that session either closes or presents the read-only remnant — it
  never shows a live state the backend does not confirm
