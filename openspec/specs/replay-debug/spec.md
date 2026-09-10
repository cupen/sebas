# replay-debug Specification

## Purpose
Defines the offline debugging loop: recording every raw inbound Feishu
WebSocket frame to disk with `--dump-inbound`, and replaying captured frames
through the exact live routing path with `sebas replay` — with the
side-effect boundary that makes replay safe to run without a live bot.

## Requirements

### Requirement: Inbound recording

`sebas im --dump-inbound <dir>` (the flag moved off `sebas core` with
extract-im-service; the core binary has no Feishu ingress) SHALL record
every raw inbound Feishu WS frame to one JSON file per frame, named
`{unix_nanos}-{pid}.json`, written before parsing. The dump directory is
created at startup; if creation fails the service logs a warning and
continues with recording disabled. Recording is enabled only by the CLI
flag — there is no config key.

#### Scenario: frames recorded verbatim

- **WHEN** `--dump-inbound` is set and a message event arrives
- **THEN** a file appears in the directory containing the frame's raw bytes
  exactly as received

#### Scenario: dump failure degrades

- **WHEN** the dump directory cannot be created
- **THEN** the service starts normally with a warning and no frames are
  recorded

### Requirement: Recording scope

Recording SHALL capture every handled frame — including frames later
dropped by owner, chat-type, mention, or parse filtering — because the dump
write happens before any filtering or parsing.

#### Scenario: filtered frames recorded

- **WHEN** a non-owner message arrives while recording is on
- **THEN** it is still dumped to disk even though the router drops it

### Requirement: Replay invocation

`sebas replay --dir <path>` (the flag's only option) SHALL load all `*.json`
files (case-insensitive extension) from the directory in lexical filename
order — which preserves capture order given the timestamp-prefixed names —
and dispatch them sequentially into a fresh router. A missing directory is
a hard error. The run prints the count of successfully dispatched frames.

#### Scenario: ordered dispatch

- **WHEN** a dump directory holds frames `001-…`, `002-…`, `003-…`
- **THEN** they are dispatched in that order, sequentially

#### Scenario: missing directory

- **WHEN** `--dir` points at a nonexistent path
- **THEN** the command exits with an error

### Requirement: Replay routing fidelity

Replay SHALL dispatch the same neutral `ChannelEvent` shape the live
Feishu adapter emits, into a fresh `DispatchHandle` with an empty session
map — no prior session state is restored, so every replay run starts from
blank state and re-creates whatever the frames imply. Replay parses the
captured events directly (gates were applied at capture time); raw
pre-neutralization envelope dumps no longer parse and are skipped with a
warning.

#### Scenario: same event shape as live

- **WHEN** a captured owner text event is replayed
- **THEN** the router dispatches the same neutral `ChannelEvent` the live
  adapter produced, and exercises the same engine routing for it

#### Scenario: blank slate per run

- **WHEN** the same directory is replayed twice
- **THEN** each run begins with an empty session map; the second run is not
  affected by the first

### Requirement: Replay applies no channel gates

Replay SHALL apply no channel gates (chat-type filter, mention gating,
dedup): captured events already passed the adapter's gates at capture time,
and the neutral `ChannelEvent` carries no gate-relevant metadata. A captured
event is dispatched unconditionally.

#### Scenario: group frame replays without gates

- **WHEN** a captured group message is replayed
- **THEN** it is dispatched regardless of any chat-type or mention
  configuration (those gates ran at capture time)

### Requirement: Replay applies no deduplication

Replay SHALL NOT deduplicate: the neutral `ChannelEvent` carries no
`event_id`, and dedup is a live-adapter concern (4096-capacity seen-set)
that ran before capture. A dump directory containing the same event twice
dispatches both occurrences; producing a dump without duplicates is the
recorder operator's responsibility.

#### Scenario: duplicate frames both dispatched

- **WHEN** a dump directory contains the same event twice
- **THEN** both are dispatched and the printed count reflects two dispatches

### Requirement: Side-effect boundary

Replay SHALL be side-effect-free beyond in-memory router state: it
constructs no Feishu client (no tokens, no network calls), spawns no ACP
child, and starts no server. The router's outbound instructions are emitted
into a channel whose receiver is held but never consumed — the live-only
dispatch pump that performs Feishu/ACP side effects does not exist in
replay. All state mutations are discarded when the process exits.

#### Scenario: no feishu traffic

- **WHEN** a replay dispatch produces `Out::React` and `Out::SendCard`
- **THEN** no HTTP call is made to Feishu; the instructions accumulate in
  the unconsumed channel

#### Scenario: no child spawn

- **WHEN** a replay dispatch produces `Out::SpawnAcp`
- **THEN** no ACP subprocess is started

### Requirement: Per-frame fault tolerance

Replay SHALL be resilient to bad captures: an unreadable file or an
unparseable payload logs a warning, skips that frame, and continues; the
dispatch count includes only successfully dispatched frames.

#### Scenario: corrupt frame skipped

- **WHEN** a dump directory holds two valid frames and one corrupt file
- **THEN** the run completes, dispatches 2, and reports the corrupt frame
  via warning
