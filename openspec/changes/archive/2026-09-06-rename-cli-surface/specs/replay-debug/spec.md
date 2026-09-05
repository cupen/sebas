## MODIFIED Requirements

### Requirement: Inbound recording

`sebas core --dump-inbound <dir>` SHALL record every raw inbound WS frame to
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

### Requirement: Replay routing fidelity

Replay SHALL drive the exact same parse-and-dispatch path as the live WS
loop (both delegate to one shared frame handler), into a fresh
`DispatchHandle` with an empty session map — no prior session state is
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
