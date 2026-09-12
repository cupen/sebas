## MODIFIED Requirements

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

`GET /api/sessions/{key}` SHALL accept an optional `entries_after` query
parameter: when present, the payload's `entries` SHALL contain only entries
whose `position` is greater than the given value, while the rest of the
payload (status, model face, project binding) stays complete; an absent
parameter SHALL keep the full-sequence behavior; a non-numeric value SHALL
be rejected with 400 rather than silently treated as a full fetch. Position
semantics SHALL guarantee incremental-fetch correctness: within a live
session,
positions SHALL be gapless (assigned contiguously from the log length) and
append-only (an appended entry is never rewritten or removed while the
session exists) — so a fetch from `entries_after=N` returns exactly the
entries a client holding position N has not seen.

#### Scenario: both sides of the conversation are in the payload

- **WHEN** the browser requests a session in which the operator submitted messages across several turns
- **THEN** the payload carries one ordered entry sequence containing both the submissions and the agent's output, each entry stating kind and element_type

#### Scenario: retired fields are gone

- **WHEN** a session payload is returned
- **THEN** it carries no single-prompt field and no agent-output-only list, and the conversation is available only as the ordered entry sequence

#### Scenario: empty session is not an error

- **WHEN** a session has no transcript entries yet
- **THEN** the payload returns an empty entry sequence with a success status

#### Scenario: incremental fetch returns only newer entries

- **WHEN** the browser requests the session with `entries_after=5` and the
  transcript holds positions 0..9
- **THEN** `entries` contains exactly positions 6..9 (ordered), and the
  payload's non-entry fields stay complete

#### Scenario: absent parameter keeps full sequence

- **WHEN** the browser requests the session without `entries_after`
- **THEN** `entries` contains the full sequence from position 0 (current
  behavior)

#### Scenario: invalid parameter is a typed rejection

- **WHEN** the browser requests the session with `entries_after=abc`
- **THEN** the response is 400 with an error message, not a silent full
  fetch

#### Scenario: positions are gapless and append-only

- **WHEN** entries are appended to a live session's transcript
- **THEN** each new entry's position equals the transcript length before
  the append (no gaps, no reuse), and previously appended entries keep
  their position and content unchanged
