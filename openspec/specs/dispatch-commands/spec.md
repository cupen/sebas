# dispatch-commands Specification

## Purpose
Defines the slash-command surface: how commands are parsed from inbound text, how each command is routed (local handling, forwarding to the live session, or forwarding to the watchdog control plane), the behavior when no live session exists, and the feedback the user receives for each command class.

## Requirements

### Requirement: Command parsing rules

The system SHALL match commands case-sensitively against the exact command word after trimming surrounding whitespace. A leading `//` SHALL escape the line: the input is forwarded as a plain prompt (with one leading slash), never interpreted as a command. Unknown slash words and plain text SHALL pass through as prompts to the session, preserving the original input. Argument splitting SHALL occur on the first whitespace run only.

#### Scenario: Double slash escapes command interpretation

- **WHEN** the user sends `//compact`
- **THEN** the text is forwarded to the session as the prompt `/compact` and the compact command does not run

#### Scenario: Unknown command passes through

- **WHEN** the user sends `/foo bar`
- **THEN** the message is forwarded to the session as a prompt verbatim

#### Scenario: Case-sensitive matching

- **WHEN** the user sends `/NEW`
- **THEN** the text is not recognized as the new-session command and passes through as a prompt

### Requirement: Per-command argument validation

Each command SHALL enforce its own argument contract: `/switch` requires a numeric argument (otherwise the line passes through); `/rollback`, `/restart`, `/services` reject any argument; `/router` accepts only `on|off|restart|status`; `/webui` accepts only `status`; `/upgrade` accepts only `dev`/`--dev` and `--dry-run`/`dry-run` flags; `/system` ignores trailing arguments; empty `/btw` passes through as text.

#### Scenario: Router with invalid action passes through

- **WHEN** the user sends `/router enable`
- **THEN** the line is not recognized as a command and passes through as a prompt

#### Scenario: Upgrade flag normalization

- **WHEN** the user sends `/upgrade dev dry-run`
- **THEN** the bare `dev` token is normalized to the dev flag and the command runs as dev-mode dry-run

### Requirement: Session-forwarded commands

`/status`, `/cost`, `/compact`, and `/cancel` SHALL be forwarded to the mapped live session: the first three as prompt text to the session; `/cancel` as a cancel instruction. When no live session is mapped (including while a spawn is in flight), these commands SHALL produce no user-visible response (the help fallback is currently a silent no-op in the dispatch layer). `/compact` SHALL additionally send a progress card that updates live until the operation finishes.

#### Scenario: /cost with a live session

- **WHEN** the user sends `/cost` while a session is active for the chat
- **THEN** the prompt `/cost` is forwarded to the session and its answer streams back as usual

#### Scenario: /compact shows a progress card

- **WHEN** the user sends `/compact` while a session is active
- **THEN** a progress card is sent before the command is forwarded
- **AND** the card's elapsed-time display updates periodically until the operation completes

#### Scenario: /status with no session is silent

- **WHEN** the user sends `/status` with no mapped session
- **THEN** no card or message is sent to the chat

### Requirement: /new spawns a fresh session

`/new [prompt]` SHALL start a fresh session for the key, with trailing text used as the initial prompt (and as the basis for the card topic). It SHALL clear the chat-level permission allowlist and reply target so the new session inherits nothing. When a spawn is already in flight for the key, `/new` SHALL be ignored.

#### Scenario: /new with initial prompt

- **WHEN** the user sends `/new refactor the auth module`
- **THEN** a fresh session spawns and receives "refactor the auth module" as its first prompt

#### Scenario: /new resets session-scoped grants

- **WHEN** the user sends `/new` after having granted "allow session" permissions in the old session
- **THEN** the new session prompts for permissions again

### Requirement: /btw priority interjection

`/btw <text>` SHALL be routed through the same text path as a normal message but marked priority: when the session is mid-turn, it is enqueued ahead of ordinary queued turns; when the session is settled it behaves as a plain continue.

#### Scenario: /btw while session is busy

- **WHEN** the user sends `/btw what files did you touch?` while a turn is streaming
- **THEN** the question is queued ahead of other waiting turns and answered after the current turn completes

### Requirement: Control commands forwarded to the watchdog

`/upgrade [dev] [--dry-run]`, `/rollback`, `/restart`, `/services`, `/system`, `/router <action>`, and `/webui status` SHALL be translated into watchdog control requests and their results returned as plain text messages — no session is required. When the control credential is not configured, the system SHALL reply with a plain-text notice explaining bare-core mode instead of issuing the request. A watchdog communication failure or rejection SHALL surface as a plain-text failure message.

#### Scenario: /system with no session

- **WHEN** the user sends `/system` with no session mapped
- **THEN** a watchdog status request is issued and the result is sent as plain text

#### Scenario: Missing control credential

- **WHEN** a control command is issued while the control secret is unset
- **THEN** the user receives a plain-text notice that the daemon is running in bare-core mode and how to enable watchdog control

#### Scenario: Watchdog offline

- **WHEN** a control command is issued and the watchdog IPC call fails
- **THEN** the user receives a plain-text failure message naming the command and the error

### Requirement: /settings read and write

`/settings` with no arguments SHALL list all supported keys with current values and the settings file path. `/settings <key>` SHALL show one key's value. `/settings <key> <value>` SHALL validate the value (enumerations and numeric ranges per key; `theme_color` accepts any string) and, on success, persist to the settings file and apply to the live configuration atomically; invalid values SHALL be rejected with an explanatory message and not applied. Replies SHALL be plain text.

#### Scenario: Invalid value rejected

- **WHEN** the user sends `/settings thinking maybe`
- **THEN** the reply lists the accepted values (`show`/`hide`) and the setting is unchanged

#### Scenario: Valid value persists and applies

- **WHEN** the user sends `/settings thinking hide`
- **THEN** the value is written to the settings file, the live config is updated, and the reply confirms the new value and file path

### Requirement: Session-independent local commands

`/sessions`, `/help`, `/provider`, and `/settings` SHALL function without a mapped session. `/sessions` SHALL list all mappings in the chat across all threads, each labeled with its state (active / spawning / dormant) and last-active time; an empty list SHALL invite the user to send `/new`.

#### Scenario: /sessions lists across threads

- **WHEN** the user sends `/sessions` in a chat that has an active main-thread session and a dormant topic session
- **THEN** both mappings are listed with their respective states

#### Scenario: /help sends an interactive card

- **WHEN** the user sends `/help`
- **THEN** an interactive help card is sent with command groups; tapping a group updates the card in place and tapping a command issues it

### Requirement: Passthrough commands

`/model <text>` and `/goal <text>` SHALL be forwarded verbatim (full original line) as prompts to the session, not interpreted locally. `/cd <path>`, `/switch <n>`, and `/resume <sid>` are parsed but currently have no routing: they fall to the help fallback, which sends nothing — producing no user-visible effect.

#### Scenario: /model forwards as prompt

- **WHEN** the user sends `/model sonnet`
- **THEN** the session receives the literal prompt `/model sonnet`

#### Scenario: /cd currently has no effect

- **WHEN** the user sends `/cd /tmp`
- **THEN** the working directory is not changed and no response is sent to the chat

### Requirement: Inbound acknowledgment reaction

Every inbound text message (command or prompt) with a known reply target SHALL receive an immediate emoji reaction on the user's message before processing begins; the reaction is later swapped to reflect processing state.

#### Scenario: Text message gets an acknowledgment reaction

- **WHEN** any text message arrives from the chat
- **THEN** an acknowledgment emoji reaction is added to that message before routing proceeds
