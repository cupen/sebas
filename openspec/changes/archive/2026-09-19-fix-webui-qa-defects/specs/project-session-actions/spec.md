## MODIFIED Requirements

### Requirement: Session archive

The rail session row's overflow menu SHALL be the operator-facing archive entry: archiving moves the session to the History group and carries the close semantics (the child is killed if active; the confirm dialog warns about pending submissions that will be discarded). The focused session's header SHALL render no action buttons and no navigation link. An archived session SHALL be read-only — the operator cannot send messages into it, cannot close it, and cannot switch to it as the active session. An archived session SHALL be restorable to its original project. The archive entry SHALL carry the session's identity — agent binding (`agent_kind`), desired permission mode, and the current/available model catalog — and restoring SHALL rebuild the session with that identity intact, so the restored session continues with the same agent and model surface it had before archiving. Legacy archive entries without identity fields SHALL restore with the existing fallbacks and the UI SHALL present that fallback honestly. Restoring SHALL remove the History entry and rebuild the session row **as one atomic outcome**: after a successful restore the session MUST be present in the session list under its original project with its full transcript, and the archive MUST no longer hold the entry. An implementation that consumes the archive entry without rebuilding the session — leaving the data reachable nowhere — SHALL be treated as data loss and is non-conformant.

#### Scenario: archive a session

- **WHEN** the operator picks archive in a rail session row's overflow menu and confirms
- **THEN** the session is moved to the History group, marked as read-only, and cannot be interacted with

#### Scenario: archived session is read-only

- **WHEN** the operator attempts to send a message to an archived session
- **THEN** the system rejects the message with a 400 response stating the session is archived

#### Scenario: restore archived session

- **WHEN** the operator clicks an archived session in the History group and confirms the restore dialog
- **THEN** the session is restored to its original project, becomes writable, and is activated

#### Scenario: restore preserves the transcript

- **WHEN** an archived session holding N transcript entries is restored
- **THEN** the rebuilt session exposes the same N entries via the session detail API, the session is listed under its original project, and the History group no longer lists it
- **AND** no state exists in which the archive entry is consumed while the session is absent from the session list

#### Scenario: focused session header offers no actions

- **WHEN** a session is focused and the operator looks at the session header
- **THEN** the header shows display information only — no Archive button, no Close button, and no "All sessions" link

#### Scenario: 恢复保留 agent 身份与模型面

- **WHEN** a session bound to a non-default agent with a selected model is archived and then restored
- **THEN** the restored session reports the same `agent_kind`, desired mode, and current/available models, and the next turn runs on that agent

#### Scenario: 旧归档条目如实回退

- **WHEN** a legacy archive entry without the identity fields is restored
- **THEN** the session comes back on the default agent with honest fallback display instead of a fabricated identity
