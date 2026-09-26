## MODIFIED Requirements

### Requirement: Session rows are named by the first prompt

A rail session row SHALL display a name for the session: if the operator has set a label for the session, the label SHALL be shown; otherwise, if an auto-generated title exists, the title SHALL be shown; otherwise the preview of the session's first user message SHALL be used. A session that has neither a label, nor an auto-generated title, nor any user message (a zero-turn placeholder) SHALL fall back to its short session identifier. Names longer than the implementation-defined display cap SHALL be truncated with an ellipsis, while the row's hover title SHALL carry the full text. When the first message is sent to a placeholder session, its name SHALL update from the identifier to the message preview without a page reload — unless an operator label is set, which SHALL stay stable. The operator SHALL be able to set and change the label from the rail (row actions), and the rail's session dialogs SHALL name the session by the same label as its row.

The workbench focused-session header SHALL name the session by the same chain (operator label, then auto-generated title, then first-message preview, then identifier) — it SHALL NOT display a raw session identifier or its truncation while any naming source exists.

Restoring an archived session SHALL preserve the naming inputs captured at archive time: the first-prompt preview and the operator label (if any) SHALL survive the restore, so the restored row is named exactly as before archival and never regresses to the raw session identifier while a naming source exists.

A label write that the server accepts SHALL reach connected clients as a session update event, and every connected client's rail SHALL re-render that row's name from the event without a page reload — the same liveness the first-message preview already enjoys, regardless of whether the write came from the rail dialog or the API. The rename dialog SHALL persist exactly the value it displays: submitting the dialog SHALL deliver the shown text to the label write path, and a confirmed save SHALL never leave the stored label unchanged while the dialog rendered a non-empty value.

#### Scenario: named by the first message

- **WHEN** a session has no operator label, no auto-generated title, and has received a first user message
- **THEN** its rail row shows a preview of that message instead of the session identifier

#### Scenario: placeholder falls back to the identifier

- **WHEN** a zero-turn placeholder session has no user message, no label, and no auto-generated title
- **THEN** its rail row shows the short session identifier as the name

#### Scenario: long first message is truncated

- **WHEN** a session's display name exceeds the display cap
- **THEN** the row shows a truncated name ending with an ellipsis, and hovering the row reveals the full text via its title

#### Scenario: placeholder name updates after the first message

- **WHEN** the operator sends the first message into a placeholder session without a label
- **THEN** the row's name becomes the message preview without a page reload

#### Scenario: operator label takes precedence

- **WHEN** the operator sets a label on a session that already has messages
- **THEN** the rail row and session dialogs show the label (not the first-message preview) and keep it across turns and reloads

#### Scenario: auto-titled row shows the title

- **WHEN** a session has no operator label and its auto-generated title has arrived
- **THEN** the rail row and the focused-session header show the title (not the preview or the identifier), updated live without a page reload

#### Scenario: renaming from the rail

- **WHEN** the operator picks rename in a rail session row's overflow menu and submits a new label
- **THEN** the row's name updates to the label without a page reload, and clearing the label falls back to the first-message preview

#### Scenario: rename dialog saves what it shows

- **WHEN** the operator types a non-empty name into the rail's rename dialog and confirms the save
- **THEN** the stored label equals the displayed text, and a subsequent page load shows the same name (a confirmed save is never a silent no-op)

#### Scenario: label writes through any path update the row live

- **WHEN** a label write is accepted through any path (rail dialog or the label API) while the session list is open in a browser
- **THEN** that session's rail row shows the new label driven by the session update event, without a manual reload

#### Scenario: restore preserves the row name

- **WHEN** an archived session that was named by its first-prompt preview (or carried an operator label) is restored to a project
- **THEN** the restored row shows the same name source as before archival instead of the raw session identifier

## ADDED Requirements

### Requirement: Sessions are auto-titled from the first prompt

On a session's first user message (a spawn carrying a prompt, or the first message into a placeholder), the system SHALL asynchronously generate a title from that message with a one-shot LLM call and store it as the session's auto-title: the call SHALL use the provider store's default selection (default provider and model), SHALL NOT block or delay the message being dispatched to the agent, and SHALL NOT overwrite an operator label. When the title is produced it SHALL reach connected clients through the normal session update liveness. If no default provider is configured, or the call fails, the system SHALL fall back silently — the first-message preview remains the name, and no error is surfaced to the operator. The title SHALL be sanitized to a single line with a display-length cap. A cleared operator label SHALL fall back per the naming rule without triggering re-generation.

#### Scenario: title arrives without delaying the first turn

- **WHEN** a session's first message is dispatched and the title call completes afterwards
- **THEN** the agent turn proceeds immediately, and the row's name updates to the title when it arrives

#### Scenario: no default provider falls back silently

- **WHEN** a session's first message arrives and no default provider/model is configured
- **THEN** the row's name is the first-message preview, no error is surfaced, and the turn proceeds normally

#### Scenario: failed title call falls back silently

- **WHEN** the title LLM call fails (network, auth, timeout)
- **THEN** the row's name is the first-message preview, no error is surfaced, and the session is unaffected

#### Scenario: operator label survives the late title

- **WHEN** the operator sets a label before the auto-title arrives
- **THEN** the auto-title does not overwrite the operator label

#### Scenario: title is single-line and capped

- **WHEN** the generated title contains newlines or exceeds the display cap
- **THEN** the stored title is collapsed to a single line and truncated to the cap
