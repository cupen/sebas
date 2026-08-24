## Purpose

Defines the offline debugging loop: recording every raw inbound Feishu
WebSocket frame to disk with `--dump-inbound`, and replaying captured frames
through the exact live routing path with `sebas replay` — with the
side-effect boundary that makes replay safe to run without a live bot.

## ADDED Requirements

### Requirement: Inbound recording

`sebas run --dump-inbound <dir>` SHALL record every raw inbound WS frame to
one JSON file per frame, named `{unix_nanos}-{pid}.json`, written before
parsing. The dump directory is created lazily on startup; if creation fails
the service logs a warning and continues with recording disabled. Recording
is enabled only by the CLI flag — there is no config key.

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

Replay SHALL drive the exact same parse-and-dispatch path as the live WS
loop (both delegate to one shared frame handler), into a fresh
`RouterHandle` with an empty session map — no prior session state is
restored, so every replay run starts from blank state and re-creates
whatever the frames imply.

#### Scenario: same path as live

- **WHEN** a captured owner text frame is replayed
- **THEN** the router emits the same `Out` instructions (ack reaction, ACP
  spawn) that the live path would emit for that frame

#### Scenario: blank slate per run

- **WHEN** the same directory is replayed twice
- **THEN** each run begins with an empty session map; the second run is not
  affected by the first

### Requirement: Replay filter divergence

Replay SHALL run chat-type-agnostic: the handler's allowed-chat-types list
and bot-name mention filter are empty during replay, so every captured
frame passes those gates regardless of the config that produced it. This is
the only behavioral divergence from the live loop, which applies the
configured filters.

#### Scenario: group frame replays without mention

- **WHEN** a captured group message without an @mention is replayed
- **THEN** it is dispatched (not filtered), whereas the live loop with a
  configured bot name would have dropped it

### Requirement: Event deduplication during replay

Replay SHALL deduplicate by `event_id` using the same seen-set mechanism as
the live loop (capacity 4096 with wholesale clear on overflow). The set is
per-invocation: duplicate frames within one run are skipped, and re-running
replay processes the same events again.

#### Scenario: duplicate frame skipped

- **WHEN** a dump directory contains the same event_id twice
- **THEN** the second occurrence is not dispatched and the printed count
  reflects one dispatch

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
