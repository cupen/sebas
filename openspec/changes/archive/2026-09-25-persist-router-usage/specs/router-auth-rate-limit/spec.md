## MODIFIED Requirements

### Requirement: Usage record pipeline

Usage records SHALL be written by an asynchronous sink: the request path
pushes records onto a bounded channel (capacity 256) consumed by a
background writer that commits them to the router's own usage database
under the state directory (its parent directory created on demand). When
the channel is full or the sink is closed, records are dropped with a
warning and the in-flight response is never blocked or failed by usage
accounting. The router SHALL be the only writer of that database and SHALL
NOT open the core state store's database. Stored records SHALL be subject
to a retention policy so the database cannot grow without bound: records
older than the configured retention, or beyond the configured row ceiling,
SHALL be pruned periodically in the background.

#### Scenario: sink overflow drops records

- **WHEN** more than 256 records are queued while the writer is stalled
- **THEN** excess records are dropped (warn-logged) and the corresponding
  responses were already served unaffected

#### Scenario: jsonl append

- **WHEN** two requests complete
- **THEN** the usage database contains two committed records under the
  router's state directory
- **AND** both are retrievable by query, in completion order

#### Scenario: router never opens the core state database

- **WHEN** the router writes usage while a core instance is running
- **THEN** the write goes to the router's own usage database
- **AND** the core state database is neither opened nor modified by the router

#### Scenario: retention prunes old records

- **WHEN** records older than the configured retention exist
- **THEN** they are pruned in the background
- **AND** records within the retention window are left intact

#### Scenario: row ceiling prunes the excess

- **WHEN** the number of stored records exceeds the configured ceiling
- **THEN** the oldest records are pruned until the ceiling is met
- **AND** the most recent records are retained

#### Scenario: pruning never blocks a response

- **WHEN** pruning runs while requests are in flight
- **THEN** the in-flight responses are unaffected
- **AND** usage accounting never blocks or fails a response

### Requirement: Usage record content

Each record SHALL carry: timestamp, protocol, model, provider,
upstream_model, status, latency_ms, ttft_ms, token counts (input, output,
cache_read, cache_creation), and an error field. The `key` field SHALL always
be empty — records are not attributable to a downstream key. The
error field is populated only for router-side failures (e.g. upstream
connect failure → status 502); upstream 4xx/5xx responses record their
status with no router error. Records SHALL be queryable by these fields
without external parsing of a log file.

#### Scenario: key never recorded

- **WHEN** a request authenticated as `k1` completes
- **THEN** the usage record's `key` field is the empty string

#### Scenario: upstream error recorded without router error

- **WHEN** the upstream returns 429
- **THEN** the record has status 429 and an empty error field

#### Scenario: records are queryable by field

- **WHEN** an operator queries the usage database for records of one model
- **THEN** the matching records are returned without parsing a log file
- **AND** the same fields are present as before this storage change
