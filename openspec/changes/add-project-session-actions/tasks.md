## 1. Backend: optional prompt in session creation

- [ ] 1.1 Change `CreateSessionRequest` in `api.rs` — make `prompt` an `Option<String>` instead of `String`; verify the existing `POST /api/sessions` tests still pass with the new type
- [ ] 1.2 Update `create_session` handler — when `prompt` is `None`, call `backend.spawn("".to_string(), ...)` with an empty string; verify a test sends a POST without `prompt` and gets a 201 with a session key
- [ ] 1.3 Verify the in-process backend's `spawn` with empty prompt creates a placeholder mapping but no live ACP child; verify the session appears in `snapshot` with status `Spawning` and zero turns

## 2. Backend: directory browser API

- [ ] 2.1 Add `GET /api/fs/browse?path=<path>` endpoint in `projects.rs` or a new `fs.rs` module returning `{ entries: [{ name: string, is_dir: boolean }] }`; canonicalise the path and reject traversal outside the home directory; verify unit tests for accessible path, traversal attempt, and non-existent path
- [ ] 2.2 Mount the route in `routes.rs`; verify an integration test calls `GET /api/fs/browse` and gets a 200 with entries

## 3. Backend: archive module

- [ ] 3.1 Create `archive.rs` in `sebas-webui/src/` with struct `ArchiveEntry { session_key, project_path, label, archived_at, retention_deadline }` and `ArchiveFile { entries }`; implement `load()`/`save()` with atomic tmp+rename, same pattern as `projects.rs`; verify unit tests for empty file, add entry, remove entry, and unparseable file
- [ ] 3.2 Implement `archive_session(key, project_path, label, retention_days)` — creates an entry, saves to file; verify the entry is in the loaded list
- [ ] 3.3 Implement `restore_session(key)` — removes entry from archive, returns the entry data; verify the entry is no longer in the loaded list
- [ ] 3.4 Implement `cleanup_expired()` — iterates entries, removes those past `retention_deadline`; verify a test adds an entry with a past deadline, calls `cleanup_expired`, and asserts the entry is gone
- [ ] 3.5 Add `archive_retention_days` to `WebUiConfig` with default 30; verify the config struct deserialises with and without the field

## 4. Backend: archive routes

- [ ] 4.1 Add `POST /api/sessions/{key}/archive` handler — validates session exists, archives it (calls `close` to kill the child if active, then moves to archive); verify integration test archives a session and it no longer appears in `GET /api/sessions`
- [ ] 4.2 Add `POST /api/sessions/{key}/restore` handler — restores an archived session to its original project; verify integration test restores and the session is back in `GET /api/sessions`
- [ ] 4.3 Add `GET /api/archive` handler — returns the list of archived entries with their metadata; verify integration test returns the archived entry
- [ ] 4.4 Wire archive expiry into WebUI startup and into `GET /api/sessions` and `GET /api/archive` — call `cleanup_expired()` before each response; verify a test with an expired entry yields no archived entries after the list call
- [ ] 4.5 Reject messages to archived sessions — `send_message` handler checks the archive and returns 400 if the session is archived; verify integration test

## 5. Frontend: project rail additions

- [ ] 5.1 Replace the inline add-project form with a `wa-dialog` that shows a directory browser (fetching `GET /api/fs/browse`) and a manual path input; clicking "Add project" calls `api.projects.add(path)`; verify the dialog renders and a project is registered
- [ ] 5.2 Add a `+` button to each project row (the `renderRow` method) that calls `api.createSession("", projectDir, backend)` to create a zero-prompt placeholder session, then selects the project and activates the session; verify clicking the button creates a session that appears in the rail
- [ ] 5.3 Add an archive button to each session row (the `renderSessionRow` method) that calls `POST /api/sessions/{key}/archive`; verify clicking the button archives the session and it moves to History
- [ ] 5.4 Split History/Inbox — rename the current History group (sessions with `project_dir === null`) to "Inbox"; add a new History group for archived sessions fetched from `GET /api/archive`; verify both groups render with correct content
- [ ] 5.5 Add restore interaction — clicking an archived session in History restores it via `POST /api/sessions/{key}/restore`; verify the session reappears under its original project
- [ ] 5.6 Add archive-only visual state — archived session rows show a muted style with a "restored" indicator; verify the visual diff

## 6. Verification

- [ ] 6.1 Run `cargo test -p sebas-webui` and verify all new and existing tests pass
- [ ] 6.2 Run `cargo clippy -p sebas-webui` and verify no new warnings
- [ ] 6.3 Full integration test: add a project via dialog, create a zero-prompt session, send a message, archive the session, verify it appears in History, restore it, verify it's back in the project, and confirm the expired session is cleaned up
- [ ] 6.4 Verify `POST /api/sessions` with `prompt: "..."` (existing behaviour) still works and creates a session with a first turn