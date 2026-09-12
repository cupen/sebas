## ADDED Requirements

### Requirement: Conversation incremental sync

The workbench SHALL fetch conversation entries incrementally using the
entries' monotonic positions as the sync cursor. The workbench SHALL keep an
in-memory cursor per session — the highest position it currently renders.
The first fetch of a session within a page lifetime (initial load, deep
link, F5 reload, or a session never focused before) SHALL fetch the full
sequence; every subsequent fetch of that session (WebSocket-event refetches,
post-action refetches) SHALL request only entries after the cursor and
append the result to the locally rendered sequence. Cursors SHALL be
per-session and survive session switching within the page lifetime; a page
reload resets all cursors (the next fetch is full again). The cursor SHALL
advance only after a successful merge; a failed fetch SHALL keep the local
sequence and cursor unchanged.

#### Scenario: first fetch is full

- **WHEN** the operator focuses a session for the first time in this page
  lifetime
- **THEN** the workbench fetches and renders the full entry sequence from
  position 0

#### Scenario: refetch is incremental and appends

- **WHEN** a WebSocket event triggers a refetch of the focused session
  whose local cursor is position N
- **THEN** the workbench requests entries after N and appends only the new
  entries to the rendered sequence, without re-rendering or re-fetching
  earlier entries

#### Scenario: switching sessions keeps per-session cursors

- **WHEN** the operator switches between two focused sessions and back
- **THEN** each session resumes from its own cursor (incremental), not a
  full refetch, as long as the page has not reloaded

#### Scenario: failed fetch does not advance the cursor

- **WHEN** an incremental fetch fails (network or server error)
- **THEN** the local sequence and cursor stay unchanged, and the next
  successful refetch resumes from the same cursor without gaps

#### Scenario: reload resets cursors

- **WHEN** the page is reloaded (F5) and the same session is reopened
- **THEN** the first fetch is full again (no stale localStorage cursor)
