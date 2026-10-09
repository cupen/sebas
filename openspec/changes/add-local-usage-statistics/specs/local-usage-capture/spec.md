## Purpose

不经 router 的 agent 回合用量落账：ACP 会话的回合 usage 持久化、native 内核解析上游 usage、双算规避、本地库归属与保留期。本地账本由 core 独占写入，与 router 的 usage.db 平行且同构，供双源统计查询。

## ADDED Requirements

### Requirement: One shared usage domain type drives both aggregations

The turn-usage record shape (timestamp, model, provider, status, input,
output, cache_read, cache_creation token counts, latency) and the
timeseries query/response shapes SHALL be defined once in the neutral
domain layer and consumed by both the router-side aggregation and the
core-side aggregation, such that identical record sets fed to either
implementation produce identical bucket structures. Each writer's table
row struct SHALL remain owned by its own crate.

#### Scenario: identical records aggregate identically on both sides

- **WHEN** the same set of usage records is aggregated by the
  router-side implementation and the core-side implementation with the
  same granularity, window, and tz_offset
- **THEN** both produce bucket structures with equal bucket keys, equal
  per-model token sums, and equal request counts

#### Scenario: local rows carry the shared record fields

- **WHEN** a local turn is recorded and read back
- **THEN** it carries every field of the shared record shape (including
  null-able token counts preserved as null, not zero)

### Requirement: ACP turn usage is persisted locally

For every ACP agent turn whose driver reports token usage, core SHALL
persist exactly one local usage row at turn completion, carrying the
reported model and the four token counts. Turns that report no token
counts SHALL still contribute a row's request count in aggregations (via
the recorded row) while contributing nothing to token sums. The
in-memory session chip semantics (feishu footer, webui usage badge) SHALL
remain unchanged.

#### Scenario: ACP turn lands one local row

- **WHEN** an ACP session completes a turn whose driver reported
  input 10 / output 50 for model `claude-sonnet`
- **THEN** exactly one local usage row exists for that turn with those
  token counts, and a subsequent local-source timeseries aggregation
  includes them

#### Scenario: core restart does not lose local history

- **WHEN** ACP turns have been recorded locally and the core process
  restarts
- **THEN** the previously persisted rows still appear in local-source
  aggregations

### Requirement: Native turn usage is extracted and persisted locally

The native kernel SHALL parse the usage fields of upstream responses
(Anthropic and OpenAI shapes) for each turn and core SHALL persist one
local usage row per completed native turn, same shape as ACP rows. Turns
whose upstream responses carry no usage SHALL contribute request count
only.

#### Scenario: native direct turn records usage

- **WHEN** a native session (direct provider connection) completes a turn
  and the upstream response reports input 5 / output 8
- **THEN** one local usage row exists with those counts and the response's
  model

#### Scenario: native turn without observable usage still counts requests

- **WHEN** a native turn fails before any usage is reported
- **THEN** the local row records the request with null token counts, and
  token sums are unchanged while the request count includes it

### Requirement: Turns routed through the router are not recorded locally

Core SHALL NOT record a local usage row for an agent turn whose upstream
path is the router (native sessions spawned with the router URL). ACP
agent sessions never dial through the router and SHALL always be recorded
locally. As a result, a request is counted by exactly one source.

#### Scenario: native via router counts once, in the router source

- **WHEN** the router is enabled and a native session turn completes
  through the router
- **THEN** the router-side aggregation includes the request and no local
  row exists for it, so an all-source aggregation counts it exactly once

#### Scenario: ACP session never double counts

- **WHEN** an ACP session turn completes while the router is enabled
- **THEN** only a local row exists for the turn (the router saw no such
  request)

### Requirement: Local usage store ownership and retention

Local usage rows SHALL live in a dedicated store owned and written
exclusively by core (never by router, never opened by router), located by
the sebas-home mapping with an explicit env override available. The store
SHALL use the shared persistence layer for its connection recipe, schema
sync, and single-writer execution model. Retention SHALL use two gates
(age and row cap) with defaults equal to the router side (30 days,
200 000 rows) and a background prune interval; prune actions SHALL be
logged with counts.

#### Scenario: store lands inside sebas home

- **WHEN** core runs with only `SEBAS_HOME` pinned
- **THEN** the local usage store is created inside that home (and nowhere
  outside it), and the env override redirects it when set

#### Scenario: retention prunes aged local rows

- **WHEN** a local usage row is older than the retention window and the
  prune interval elapses
- **THEN** the row is deleted and the prune is logged with a count
