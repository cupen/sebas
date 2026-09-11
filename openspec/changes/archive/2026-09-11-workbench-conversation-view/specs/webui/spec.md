## MODIFIED Requirements

### Requirement: Session dashboard and focus semantics

The cross-project session list SHALL render one row per known session (encoded key, chat and thread ids, session id, status, phase, relative last-active), active-first, and SHALL be reachable from the workbench rather than from primary navigation. The session list SHALL exclude archived sessions — those are served by `GET /api/archive`. Selecting a session in the rail, opening its `/sessions/{key}` deep link, or posting `/switch` SHALL focus that session in place — a display pointer only that never changes message routing — and `switch` returns the redirect target or 404 for an unknown key. There SHALL be no separate per-session detail surface: the workbench renders the focused session. Switching the displayed project SHALL NOT alter the focused session pointer. The rail's current-session marker SHALL be derived from the focused-session pointer, not from the browser location.

The focused-session pointer SHALL be the single source of truth for the workbench composer's follow-up vs creation mode: with a focused session the composer targets that session; without one the composer is in creation mode. Any path that focuses a session (switch endpoint, deep-link visit, placeholder creation) SHALL leave the composer able to submit a follow-up message to that session without further operator action.

#### Scenario: focus is cosmetic

- **WHEN** the user focuses session B in the WebUI while session A is active
- **THEN** subsequent Feishu messages still route per the router's own session mapping, unchanged

#### Scenario: switch unknown key

- **WHEN** `/api/sessions/{key}/switch` posts a key not in the map
- **THEN** the response is 404

#### Scenario: rail selection focuses in place

- **WHEN** the operator selects a session in the rail
- **THEN** that session becomes focused, the workbench renders its conversation in place, and the operator is not navigated to a separate detail page

#### Scenario: project switch leaves focus alone

- **WHEN** the operator switches the displayed project
- **THEN** the focused session pointer is unchanged

#### Scenario: archive hides session from list

- **WHEN** a session is archived
- **THEN** it is no longer returned by `GET /api/sessions` and appears only in the `GET /api/archive` response

#### Scenario: focusing enables immediate follow-up

- **WHEN** a session becomes focused through any supported path
- **THEN** the workbench composer's next submission is delivered to that session as a follow-up message

## ADDED Requirements

### Requirement: Session payload carries the conversation

The session payloads the workbench reads — `GET /api/sessions/{key}` and the
focused session in `GET /api/summary` — SHALL carry the session's conversation as
one ordered entry sequence. Each entry SHALL state its monotonic `position`, its
`kind` (a submission by the operator, or content produced by the agent), its
`element_type`, its content, and its timestamp. Submission entries SHALL be part
of that sequence. The former single `user_prompt` field and the agent-output-only
`body` field SHALL be retired: a client SHALL NOT have to reconstruct the
operator's turns from a separate field, nor infer turn boundaries from timestamps.
A session with no entries SHALL render an honest empty state rather than a failed
payload.

#### Scenario: both sides of the conversation are in the payload

- **WHEN** the browser requests a session in which the operator submitted messages across several turns
- **THEN** the payload carries one ordered entry sequence containing both the submissions and the agent's output, each entry stating kind and element_type

#### Scenario: retired fields are gone

- **WHEN** a session payload is returned
- **THEN** it carries no single-prompt field and no agent-output-only list, and the conversation is available only as the ordered entry sequence

#### Scenario: empty session is not an error

- **WHEN** a session has no transcript entries yet
- **THEN** the payload returns an empty entry sequence with a success status
